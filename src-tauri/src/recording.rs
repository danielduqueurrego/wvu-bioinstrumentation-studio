//! Phase 1 `.bmeg`: `BMEGREC1` + u16 JSON-header length + UTF-8 header + fixed records.
//! Each record is little-endian: u32 sample sequence, u64 timestamp_us, u16 ADC counts.
use crate::protocol::IntegrityCounters;
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
};
pub const BMEG_MAGIC: &[u8; 8] = b"BMEGREC1";
pub const BMEG_RECORD_BYTES: usize = 14;

/// A user-selected recording duration.  The tagged representation makes an
/// indefinite recording unambiguous at the Tauri boundary: it can never be
/// mistaken for a timed recording with zero seconds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum RecordingDuration {
    Timed { seconds: u64 },
    UntilStopped,
}

impl RecordingDuration {
    pub const MIN_TIMED_SECONDS: u64 = 10;

    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Timed { seconds } if *seconds < Self::MIN_TIMED_SECONDS => {
                Err("timed recordings must be at least 10 seconds")
            }
            Self::Timed { .. } | Self::UntilStopped => Ok(()),
        }
    }

    pub fn requested_seconds(&self) -> Option<u64> {
        match self {
            Self::Timed { seconds } => Some(*seconds),
            Self::UntilStopped => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Timed { .. } => "timed",
            Self::UntilStopped => "until_stopped",
        }
    }
}

/// The single authoritative reason a session's recording was finalized.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    #[default]
    User,
    TimedComplete,
    Disconnect,
    StorageGuard,
    ApplicationClose,
    Fault,
}

impl StopReason {
    pub fn recording_status(self) -> &'static str {
        match self {
            Self::User => "stopped_by_user",
            Self::TimedComplete => "complete",
            Self::Disconnect => "disconnected",
            Self::StorageGuard => "storage_guard",
            Self::ApplicationClose => "application_close",
            Self::Fault => "faulted",
        }
    }

    pub fn is_complete(self) -> bool {
        matches!(
            self,
            Self::User | Self::TimedComplete | Self::ApplicationClose
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordingMetadata {
    pub utc_start: DateTime<Utc>,
    pub local_start: DateTime<Local>,
    pub board: String,
    pub com_port: String,
    pub fqbn: String,
    pub arduino_cli_version: String,
    pub uno_r4_core_version: String,
    pub firmware_build: u32,
    pub protocol_version: String,
    pub analog_pin: String,
    pub adc_bits: u8,
    pub requested_sample_rate_hz: u32,
    pub measured_sample_rate_hz: f64,
    pub total_samples: u64,
    pub integrity: IntegrityCounters,
    pub app_version: String,
    pub simulator: bool,
    pub utc_stop: Option<DateTime<Utc>>,
    pub local_stop: Option<DateTime<Local>>,
    pub host_elapsed_seconds: Option<f64>,
    pub board_elapsed_seconds: Option<f64>,
    pub recording_status: String,
    pub bmeg_filename: String,
    pub csv_filename: Option<String>,
    pub notes: String,
    /// Optional fields preserve read compatibility with Phase 1.0 sidecars.
    #[serde(default)]
    pub duration_mode: Option<String>,
    #[serde(default)]
    pub requested_duration_seconds: Option<u64>,
    #[serde(default)]
    pub stop_reason: Option<StopReason>,
    #[serde(default)]
    pub initial_free_disk_bytes: Option<u64>,
    #[serde(default)]
    pub final_free_disk_bytes: Option<u64>,
    #[serde(default)]
    pub completion_status: String,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawSample {
    pub sequence: u32,
    pub timestamp_us: u64,
    pub counts: u16,
}
pub struct BmegWriter {
    writer: BufWriter<File>,
    pub records: u64,
}
#[derive(Debug, thiserror::Error)]
pub enum RecordingError {
    #[error("recording I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("recording header: {0}")]
    Json(#[from] serde_json::Error),
    #[error("recording header exceeds 65535 bytes")]
    HeaderTooLarge,
    #[error("not a Phase 1 BMEG recording")]
    BadMagic,
    #[error("recording is truncated")]
    Truncated,
}
impl BmegWriter {
    pub fn create(path: &Path, metadata: &RecordingMetadata) -> Result<Self, RecordingError> {
        let header = serde_json::to_vec(metadata)?;
        let header_len = u16::try_from(header.len()).map_err(|_| RecordingError::HeaderTooLarge)?;
        let mut writer = BufWriter::new(File::create(path)?);
        writer.write_all(BMEG_MAGIC)?;
        writer.write_all(&header_len.to_le_bytes())?;
        writer.write_all(&header)?;
        Ok(Self { writer, records: 0 })
    }
    pub fn write(&mut self, sample: RawSample) -> Result<(), RecordingError> {
        self.writer.write_all(&sample.sequence.to_le_bytes())?;
        self.writer.write_all(&sample.timestamp_us.to_le_bytes())?;
        self.writer.write_all(&sample.counts.to_le_bytes())?;
        self.records += 1;
        Ok(())
    }

    /// Periodic flushing bounds the amount of validated data held by the OS
    /// buffer during a long recording without retaining samples in memory.
    pub fn flush(&mut self) -> Result<(), RecordingError> {
        self.writer.flush()?;
        Ok(())
    }

    pub fn finish(mut self) -> Result<(), RecordingError> {
        self.writer.flush()?;
        Ok(())
    }
}

/// Streaming reader for finalized Phase 1 recordings. It retains just one record.
pub struct BmegReader {
    reader: BufReader<File>,
    pub metadata: RecordingMetadata,
}
impl BmegReader {
    pub fn open(path: &Path) -> Result<Self, RecordingError> {
        let mut reader = BufReader::new(File::open(path)?);
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic).map_err(map_eof)?;
        if &magic != BMEG_MAGIC {
            return Err(RecordingError::BadMagic);
        }
        let mut length = [0u8; 2];
        reader.read_exact(&mut length).map_err(map_eof)?;
        let mut header = vec![0u8; usize::from(u16::from_le_bytes(length))];
        reader.read_exact(&mut header).map_err(map_eof)?;
        Ok(Self {
            reader,
            metadata: serde_json::from_slice(&header)?,
        })
    }
    pub fn next_sample(&mut self) -> Result<Option<RawSample>, RecordingError> {
        let mut bytes = [0u8; BMEG_RECORD_BYTES];
        match self.reader.read(&mut bytes[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => {}
            Err(error) => return Err(RecordingError::Io(error)),
        }
        self.reader.read_exact(&mut bytes[1..]).map_err(map_eof)?;
        Ok(Some(RawSample {
            sequence: u32::from_le_bytes(
                bytes[0..4]
                    .try_into()
                    .map_err(|_| RecordingError::Truncated)?,
            ),
            timestamp_us: u64::from_le_bytes(
                bytes[4..12]
                    .try_into()
                    .map_err(|_| RecordingError::Truncated)?,
            ),
            counts: u16::from_le_bytes(
                bytes[12..14]
                    .try_into()
                    .map_err(|_| RecordingError::Truncated)?,
            ),
        }))
    }
}

fn map_eof(error: std::io::Error) -> RecordingError {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        RecordingError::Truncated
    } else {
        RecordingError::Io(error)
    }
}

/// Writes a CSV by streaming a BMEG file. No complete recording is kept in RAM.
pub fn export_bmeg_csv(bmeg: &Path, csv: &Path) -> Result<u64, RecordingError> {
    let mut input = BmegReader::open(bmeg)?;
    let first_timestamp = input.metadata.utc_start.timestamp_micros();
    let mut writer = BufWriter::new(File::create(csv)?);
    writer.write_all(
        b"sample_sequence,timestamp_us,elapsed_seconds,channel,adc_counts,volts,status_flags\n",
    )?;
    let mut count = 0u64;
    let mut origin = None;
    while let Some(sample) = input.next_sample()? {
        let start = *origin.get_or_insert(sample.timestamp_us);
        let elapsed = (sample.timestamp_us.saturating_sub(start)) as f64 / 1_000_000.0;
        writeln!(
            writer,
            "{},{},{elapsed:.6},A0,{},{:.6},1",
            sample.sequence,
            sample.timestamp_us,
            sample.counts,
            f64::from(sample.counts) * 5.0 / 4095.0
        )?;
        count += 1;
    }
    let _ = first_timestamp; // retained for future host/board latency diagnostics.
    writer.flush()?;
    Ok(count)
}
pub fn write_csv(path: &Path, samples: &[RawSample]) -> Result<(), RecordingError> {
    let mut writer = BufWriter::new(File::create(path)?);
    writer.write_all(b"sample_sequence,timestamp_us,counts,volts\n")?;
    for sample in samples {
        writeln!(
            writer,
            "{},{},{},{:.6}",
            sample.sequence,
            sample.timestamp_us,
            sample.counts,
            f64::from(sample.counts) * 5.0 / 4095.0
        )?;
    }
    writer.flush()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    #[test]
    fn bmeg_and_csv_round_trip() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let meta = RecordingMetadata {
            utc_start: Utc::now(),
            local_start: Local::now(),
            board: "simulator".into(),
            com_port: "SIM".into(),
            fqbn: "simulator".into(),
            arduino_cli_version: "n/a".into(),
            uno_r4_core_version: "n/a".into(),
            firmware_build: 1,
            protocol_version: "0.1".into(),
            analog_pin: "A0".into(),
            adc_bits: 12,
            requested_sample_rate_hz: 1000,
            measured_sample_rate_hz: 1000.0,
            total_samples: 1,
            integrity: IntegrityCounters::default(),
            app_version: "test".into(),
            simulator: true,
            utc_stop: None,
            local_stop: None,
            host_elapsed_seconds: None,
            board_elapsed_seconds: None,
            recording_status: "active".into(),
            bmeg_filename: "r.bmeg".into(),
            csv_filename: None,
            notes: "test".into(),
            duration_mode: Some("timed".into()),
            requested_duration_seconds: Some(10),
            stop_reason: None,
            initial_free_disk_bytes: Some(2 * 1024 * 1024 * 1024),
            final_free_disk_bytes: Some(2 * 1024 * 1024 * 1024),
            completion_status: "complete".into(),
        };
        let bmeg = dir.path().join("r.bmeg");
        let mut w = BmegWriter::create(&bmeg, &meta).unwrap_or_else(|e| panic!("{e}"));
        w.write(RawSample {
            sequence: 1,
            timestamp_us: 1000,
            counts: 2048,
        })
        .unwrap_or_else(|e| panic!("{e}"));
        w.finish().unwrap_or_else(|e| panic!("{e}"));
        assert!(std::fs::metadata(&bmeg)
            .map(|x| x.len() > 10)
            .unwrap_or(false));
        let mut reader = BmegReader::open(&bmeg).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            reader.next_sample().unwrap_or_else(|e| panic!("{e}")),
            Some(RawSample {
                sequence: 1,
                timestamp_us: 1000,
                counts: 2048
            })
        );
        assert_eq!(reader.next_sample().unwrap_or_else(|e| panic!("{e}")), None);
        let csv = dir.path().join("r.csv");
        write_csv(
            &csv,
            &[RawSample {
                sequence: 1,
                timestamp_us: 1000,
                counts: 2048,
            }],
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(std::fs::read_to_string(csv)
            .unwrap_or_default()
            .contains("timestamp_us"));
        let exported = dir.path().join("exported.csv");
        assert_eq!(
            export_bmeg_csv(&bmeg, &exported).unwrap_or_else(|e| panic!("{e}")),
            1
        );
    }

    #[test]
    fn truncated_records_are_rejected() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("short.bmeg");
        std::fs::write(&path, [BMEG_MAGIC.as_slice(), &[0, 0], &[1]].concat())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            BmegReader::open(&path),
            Err(RecordingError::Json(_))
        ));
    }

    #[test]
    fn duration_is_explicit_and_rejects_short_timed_requests() {
        assert!(RecordingDuration::Timed { seconds: 9 }.validate().is_err());
        assert!(RecordingDuration::Timed { seconds: 10 }.validate().is_ok());
        assert!(RecordingDuration::UntilStopped.validate().is_ok());
        let json = serde_json::to_string(&RecordingDuration::UntilStopped)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(json.contains("until_stopped"));
        assert!(!json.contains('0'));
    }

    #[test]
    fn phase_one_metadata_without_new_fields_still_deserializes() {
        let metadata = RecordingMetadata {
            utc_start: Utc::now(),
            local_start: Local::now(),
            board: "simulator".into(),
            com_port: "SIM".into(),
            fqbn: "simulator".into(),
            arduino_cli_version: "n/a".into(),
            uno_r4_core_version: "n/a".into(),
            firmware_build: 1,
            protocol_version: "0.1".into(),
            analog_pin: "A0".into(),
            adc_bits: 12,
            requested_sample_rate_hz: 1000,
            measured_sample_rate_hz: 1000.0,
            total_samples: 0,
            integrity: IntegrityCounters::default(),
            app_version: "test".into(),
            simulator: true,
            utc_stop: None,
            local_stop: None,
            host_elapsed_seconds: None,
            board_elapsed_seconds: None,
            recording_status: "complete".into(),
            bmeg_filename: "r.bmeg".into(),
            csv_filename: None,
            notes: "test".into(),
            duration_mode: Some("timed".into()),
            requested_duration_seconds: Some(10),
            stop_reason: Some(StopReason::TimedComplete),
            initial_free_disk_bytes: Some(1),
            final_free_disk_bytes: Some(1),
            completion_status: "complete".into(),
        };
        let mut old = serde_json::to_value(metadata).unwrap_or_else(|e| panic!("{e}"));
        let object = old
            .as_object_mut()
            .unwrap_or_else(|| panic!("metadata is not an object"));
        for name in [
            "duration_mode",
            "requested_duration_seconds",
            "stop_reason",
            "initial_free_disk_bytes",
            "final_free_disk_bytes",
            "completion_status",
        ] {
            object.remove(name);
        }
        let restored: RecordingMetadata =
            serde_json::from_value(old).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(restored.duration_mode, None);
        assert_eq!(restored.completion_status, "");
    }
}
