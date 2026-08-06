//! Bench-only Phase 3A acceptance harness. It calls the same profile-aware
//! SessionController path as the Tauri commands; it never permits human signals.
use std::{env, error::Error, fs, path::PathBuf};
use wvu_bioinstrumentation_studio_lib::{
    arduino_cli::ArduinoCli,
    profiles::{built_in_profiles, AcquisitionProfile},
    recording::{BmegReader, RecordingDuration, RecordingMetadata},
    session::SessionController,
};

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.len() != 3 || !matches!(arguments[0].as_str(), "simulator" | "hardware") {
        return Err(
            "usage: phase3a_profile_capture <simulator|hardware> <development|ecg|emg> <seconds>"
                .into(),
        );
    }
    let seconds: u64 = arguments[2].parse()?;
    if seconds < 10 {
        return Err("acceptance duration must be at least 10 seconds".into());
    }
    let profile = profile_for(&arguments[1])?;
    let output = env::temp_dir().join(format!(
        "wvu_phase3a_{}_{}",
        profile.category,
        chrono::Utc::now().timestamp_millis()
    ));
    fs::create_dir_all(&output)?;
    let session = SessionController::default();
    let duration = RecordingDuration::Timed { seconds };
    let summary = if arguments[0] == "simulator" {
        session.capture_simulator_with_profile(profile.snapshot(true), duration, &output)?
    } else {
        let cli = ArduinoCli::discover(None)?;
        let board = cli
            .boards()?
            .into_iter()
            .next()
            .ok_or("no detected UNO R4 WiFi")?;
        session.capture_serial_with_profile(
            profile.snapshot(true),
            &board.port,
            duration,
            &output,
        )?
    };
    validate(
        &summary.bmeg_path,
        &summary.metadata_path,
        &summary.csv_path,
        &profile,
    )?;
    println!(
        "{}",
        serde_json::json!({
            "profile_id": profile.profile_id, "category": profile.category, "samples": summary.samples,
            "packets": summary.packets, "measured_rate_hz": summary.measured_rate_hz,
            "integrity": summary.integrity, "bmeg": summary.bmeg_path, "metadata": summary.metadata_path,
            "csv": summary.csv_path, "completion": summary.completion_status
        })
    );
    Ok(())
}

fn profile_for(category: &str) -> Result<AcquisitionProfile, Box<dyn Error>> {
    built_in_profiles()?
        .into_iter()
        .find(|profile| profile.category == category)
        .ok_or_else(|| format!("unknown profile category {category}").into())
}

fn validate(
    bmeg: &str,
    metadata_path: &str,
    csv: &str,
    expected: &AcquisitionProfile,
) -> Result<(), Box<dyn Error>> {
    let metadata: RecordingMetadata = serde_json::from_slice(&fs::read(metadata_path)?)?;
    let snapshot = metadata
        .profile_snapshot
        .as_ref()
        .ok_or("missing profile snapshot")?;
    if snapshot.profile.profile_id != expected.profile_id || !snapshot.bench_notice_acknowledged {
        return Err("profile snapshot mismatch".into());
    }
    let mut reader = BmegReader::open(&PathBuf::from(bmeg))?;
    let mut previous: Option<(u32, u64)> = None;
    let mut count = 0u64;
    while let Some(sample) = reader.next_sample()? {
        if let Some((sequence, timestamp)) = previous {
            if sample.sequence != sequence.wrapping_add(1) || sample.timestamp_us <= timestamp {
                return Err("noncontiguous BMEG record".into());
            }
        }
        previous = Some((sample.sequence, sample.timestamp_us));
        count += 1;
    }
    let csv_rows = fs::read_to_string(csv)?.lines().skip(1).count() as u64;
    if count != metadata.total_samples || count != csv_rows {
        return Err("BMEG/CSV/metadata counts disagree".into());
    }
    if metadata.integrity.crc_failures != 0
        || metadata.integrity.missing_packet_sequences != 0
        || metadata.integrity.missing_sample_sequences != 0
        || metadata.integrity.firmware_overflows != 0
        || metadata.integrity.host_channel_overflows != 0
    {
        return Err("unexpected integrity counter".into());
    }
    Ok(())
}
