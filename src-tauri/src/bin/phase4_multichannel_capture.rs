//! Controlled Phase 4 acceptance harness. It exercises the production
//! SessionController, parser, writer, and exporter; it is not an alternate
//! acquisition implementation. Hardware use is limited to the UNO R4 WiFi
//! with floating analog inputs or a safe 0–5 V bench signal.
use std::{env, error::Error, fs, path::PathBuf};

use wvu_bioinstrumentation_studio_lib::{
    arduino_cli::ArduinoCli,
    profiles::{built_in_profiles, AcquisitionMode, AcquisitionProfile},
    recording::{BmegReader, RecordingDuration, RecordingMetadata},
    session::SessionController,
};

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.len() != 3 || !matches!(arguments[0].as_str(), "simulator" | "hardware") {
        return Err("usage: phase4_multichannel_capture <simulator|hardware> <ecg|emg|bp|pulseox|general> <seconds>".into());
    }
    let seconds: u64 = arguments[2].parse()?;
    if seconds < 10 {
        return Err("acceptance duration must be at least 10 seconds".into());
    }
    let profile = profile_for(&arguments[1])?;
    let output = env::temp_dir().join(format!(
        "wvu_phase4_{}_{}",
        profile.category,
        chrono::Utc::now().timestamp_millis()
    ));
    fs::create_dir_all(&output)?;
    let session = SessionController::default();
    let duration = RecordingDuration::Timed { seconds };
    let summary = if arguments[0] == "simulator" {
        session.capture_simulator_with_profile(profile.snapshot(false), duration, &output)?
    } else {
        let board = ArduinoCli::discover(None)?
            .boards()?
            .into_iter()
            .next()
            .ok_or("no detected UNO R4 WiFi")?;
        session.capture_serial_with_profile(
            profile.snapshot(false),
            &board.port,
            duration,
            &output,
        )?
    };
    let records = validate(
        &summary.bmeg_path,
        &summary.metadata_path,
        &summary.csv_path,
        &profile,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "profile_id": profile.profile_id,
            "category": profile.category,
            "records": records,
            "packets": summary.packets,
            "measured_rate_hz": summary.measured_rate_hz,
            "integrity": summary.integrity,
            "bmeg": summary.bmeg_path,
            "metadata": summary.metadata_path,
            "csv": summary.csv_path,
            "completion": summary.completion_status,
            "active_digital_output_mask": summary.active_digital_output_mask,
            "final_digital_output_mask": summary.final_digital_output_mask,
        }))?
    );
    Ok(())
}

fn profile_for(alias: &str) -> Result<AcquisitionProfile, Box<dyn Error>> {
    let category = match alias {
        "general" => "development",
        "ecg" => "course_ecg",
        "emg" => "course_emg_force",
        "bp" => "course_blood_pressure",
        "pulseox" => "course_pulseox",
        _ => return Err(format!("unknown Phase 4 profile alias {alias}").into()),
    };
    built_in_profiles()?
        .into_iter()
        .find(|profile| profile.category == category)
        .ok_or_else(|| format!("missing built-in profile category {category}").into())
}

fn validate(
    bmeg_path: &str,
    metadata_path: &str,
    csv_path: &str,
    profile: &AcquisitionProfile,
) -> Result<u64, Box<dyn Error>> {
    let metadata: RecordingMetadata = serde_json::from_slice(&fs::read(metadata_path)?)?;
    let snapshot = metadata
        .profile_snapshot
        .as_ref()
        .ok_or("missing profile snapshot")?;
    if snapshot.profile.profile_id != profile.profile_id {
        return Err("recording profile provenance mismatch".into());
    }
    let expected_fields = profile.acquisition.record_field_names();
    let expected_rate = f64::from(profile.acquisition.sample_rate_hz);
    let mut reader = BmegReader::open(&PathBuf::from(bmeg_path))?;
    let mut previous: Option<(u32, u64)> = None;
    let mut records = 0u64;
    while let Some(record) = reader.next_record()? {
        if record.counts.len() != expected_fields.len() {
            return Err(format!(
                "record has {} fields; expected {}",
                record.counts.len(),
                expected_fields.len()
            )
            .into());
        }
        if let Some((sequence, timestamp)) = previous {
            if record.sequence != sequence.wrapping_add(1) || record.timestamp_us <= timestamp {
                return Err("noncontiguous or nonmonotonic logical record".into());
            }
        }
        previous = Some((record.sequence, record.timestamp_us));
        records += 1;
    }
    let header = fs::read_to_string(csv_path)?
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    let expected_header = match profile.acquisition.acquisition_mode {
        AcquisitionMode::Simultaneous => {
            format!("record_sequence,t_us,{}", expected_fields.join(","))
        }
        AcquisitionMode::Pulseox4State => format!("cycle_index,t_us,{}", expected_fields.join(",")),
    };
    if header != expected_header {
        return Err(format!("CSV header mismatch: {header}").into());
    }
    let csv_rows = fs::read_to_string(csv_path)?.lines().skip(1).count() as u64;
    if records != metadata.total_samples || records != csv_rows {
        return Err("BMEG, metadata, and CSV record counts disagree".into());
    }
    if metadata.integrity.crc_failures != 0
        || metadata.integrity.invalid_frames != 0
        || metadata.integrity.missing_packet_sequences != 0
        || metadata.integrity.missing_sample_sequences != 0
        || metadata.integrity.duplicate_packets != 0
        || metadata.integrity.out_of_order_packets != 0
        || metadata.integrity.firmware_overflows != 0
        || metadata.integrity.host_channel_overflows != 0
    {
        return Err("unexpected integrity counter".into());
    }
    if (metadata.measured_sample_rate_hz - expected_rate).abs() > expected_rate * 0.02 {
        return Err("measured rate is outside the 2% acceptance band".into());
    }
    Ok(records)
}
