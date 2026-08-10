//! `.bmeg`: `BMEGREC1` + u16 JSON-header length + UTF-8 header + streamed records.
//! Legacy Phase 1–3 records are `u32 sequence, u64 timestamp_us, u16 ADC counts`.
//! Phase 4 profile-aware records preserve a synchronized frame as
//! `u32 sequence, u64 timestamp_us, u16 status_flags, u16[field_count] counts`.
use crate::{
    calibration::{
        apply_linear, counts_to_volts, mpxv_kpa, mpxv_mmhg, CalibrationType, RecordingCalibration,
        DEFAULT_ADC_REFERENCE_V, DEFAULT_MPXV_SUPPLY_V,
    },
    profiles::ProfileSnapshot,
    protocol::IntegrityCounters,
};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
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
    #[serde(default)]
    pub board_serial: Option<String>,
    pub com_port: String,
    pub fqbn: String,
    pub arduino_cli_version: String,
    pub uno_r4_core_version: String,
    pub firmware_build: u32,
    pub protocol_version: String,
    pub analog_pin: String,
    #[serde(default)]
    pub active_analog_pins: Vec<String>,
    #[serde(default)]
    pub digital_output_mapping: BTreeMap<String, String>,
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
    /// Phase 3A writes a frozen profile into both the BMEG JSON header and the
    /// metadata sidecar. Files made before this field existed remain legacy/general recordings.
    #[serde(default)]
    pub profile_snapshot: Option<ProfileSnapshot>,
    /// Historical Phase 3B metadata retained only so previously recorded BMEG
    /// files deserialize safely after the class application removed Validation.
    #[serde(default)]
    pub validation_context: Option<LegacyValidationContext>,
    /// Manual, non-protocol experiment annotations. Markers never alter or remove raw samples.
    #[serde(default)]
    pub markers: Vec<RecordingMarker>,
    /// Phase 5 derived-unit settings captured at recording start. Raw BMEG records
    /// remain ADC counts regardless of whether any of these conversions are selected.
    #[serde(default)]
    pub calibration: Option<RecordingCalibration>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyValidationContext {
    pub validation_id: String,
    pub test_type: String,
    pub run_number: u32,
    pub bench_only: bool,
    pub source_description: String,
    pub source_setpoint_v: Option<f64>,
    pub source_offset_v: Option<f64>,
    pub source_frequency_hz: Option<f64>,
    pub source_peak_to_peak_v: Option<f64>,
    #[serde(default)]
    pub equipment_metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub simulator_parameters: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordingMarker {
    pub timestamp_us: u64,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawSample {
    pub sequence: u32,
    pub timestamp_us: u64,
    pub counts: u16,
}

/// One synchronized logical acquisition record. The UNO reads configured analog pins in a
/// deterministic sequential order; this shared timestamp/sequence preserves the logical frame.
#[derive(Clone, Debug, PartialEq)]
pub struct SynchronizedRecord {
    pub sequence: u32,
    pub timestamp_us: u64,
    pub status_flags: u16,
    pub counts: Vec<u16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordLayout {
    Legacy,
    Synchronized { field_count: usize },
}
pub struct BmegWriter {
    writer: BufWriter<File>,
    pub records: u64,
    layout: RecordLayout,
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
    #[error("recording record shape does not match the profile field layout")]
    InvalidRecordShape,
    #[error("CSV conversion: {0}")]
    Csv(String),
}
impl BmegWriter {
    pub fn create(path: &Path, metadata: &RecordingMetadata) -> Result<Self, RecordingError> {
        Self::create_with_layout(path, metadata, RecordLayout::Legacy)
    }

    pub fn create_synchronized(
        path: &Path,
        metadata: &RecordingMetadata,
        field_count: usize,
    ) -> Result<Self, RecordingError> {
        if field_count == 0 || field_count > 8 {
            return Err(RecordingError::InvalidRecordShape);
        }
        Self::create_with_layout(path, metadata, RecordLayout::Synchronized { field_count })
    }

    fn create_with_layout(
        path: &Path,
        metadata: &RecordingMetadata,
        layout: RecordLayout,
    ) -> Result<Self, RecordingError> {
        let header = serde_json::to_vec(metadata)?;
        let header_len = u16::try_from(header.len()).map_err(|_| RecordingError::HeaderTooLarge)?;
        let mut writer = BufWriter::new(File::create(path)?);
        writer.write_all(BMEG_MAGIC)?;
        writer.write_all(&header_len.to_le_bytes())?;
        writer.write_all(&header)?;
        Ok(Self {
            writer,
            records: 0,
            layout,
        })
    }
    pub fn write(&mut self, sample: RawSample) -> Result<(), RecordingError> {
        if self.layout != RecordLayout::Legacy {
            return Err(RecordingError::InvalidRecordShape);
        }
        self.writer.write_all(&sample.sequence.to_le_bytes())?;
        self.writer.write_all(&sample.timestamp_us.to_le_bytes())?;
        self.writer.write_all(&sample.counts.to_le_bytes())?;
        self.records += 1;
        Ok(())
    }

    pub fn write_record(&mut self, record: &SynchronizedRecord) -> Result<(), RecordingError> {
        let RecordLayout::Synchronized { field_count } = self.layout else {
            return Err(RecordingError::InvalidRecordShape);
        };
        if record.counts.len() != field_count {
            return Err(RecordingError::InvalidRecordShape);
        }
        self.writer.write_all(&record.sequence.to_le_bytes())?;
        self.writer.write_all(&record.timestamp_us.to_le_bytes())?;
        self.writer.write_all(&record.status_flags.to_le_bytes())?;
        for counts in &record.counts {
            self.writer.write_all(&counts.to_le_bytes())?;
        }
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
    layout: RecordLayout,
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
        let metadata: RecordingMetadata = serde_json::from_slice(&header)?;
        let layout = metadata
            .profile_snapshot
            .as_ref()
            .filter(|snapshot| snapshot.profile.required_firmware.protocol_minor_min >= 2)
            .map(|snapshot| RecordLayout::Synchronized {
                field_count: snapshot.profile.acquisition.record_field_names().len(),
            })
            .unwrap_or(RecordLayout::Legacy);
        Ok(Self {
            reader,
            metadata,
            layout,
        })
    }
    pub fn next_sample(&mut self) -> Result<Option<RawSample>, RecordingError> {
        Ok(self.next_record()?.map(|record| RawSample {
            sequence: record.sequence,
            timestamp_us: record.timestamp_us,
            counts: record.counts[0],
        }))
    }

    pub fn next_record(&mut self) -> Result<Option<SynchronizedRecord>, RecordingError> {
        match self.layout {
            RecordLayout::Legacy => self.next_legacy_record(),
            RecordLayout::Synchronized { field_count } => {
                let mut prefix = [0u8; 14];
                match self.reader.read(&mut prefix[..1]) {
                    Ok(0) => return Ok(None),
                    Ok(_) => {}
                    Err(error) => return Err(RecordingError::Io(error)),
                }
                self.reader.read_exact(&mut prefix[1..]).map_err(map_eof)?;
                let mut values = vec![0u8; field_count * 2];
                self.reader.read_exact(&mut values).map_err(map_eof)?;
                Ok(Some(SynchronizedRecord {
                    sequence: u32::from_le_bytes(
                        prefix[0..4]
                            .try_into()
                            .map_err(|_| RecordingError::Truncated)?,
                    ),
                    timestamp_us: u64::from_le_bytes(
                        prefix[4..12]
                            .try_into()
                            .map_err(|_| RecordingError::Truncated)?,
                    ),
                    status_flags: u16::from_le_bytes(
                        prefix[12..14]
                            .try_into()
                            .map_err(|_| RecordingError::Truncated)?,
                    ),
                    counts: values
                        .chunks_exact(2)
                        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
                        .collect(),
                }))
            }
        }
    }

    fn next_legacy_record(&mut self) -> Result<Option<SynchronizedRecord>, RecordingError> {
        let mut bytes = [0u8; BMEG_RECORD_BYTES];
        match self.reader.read(&mut bytes[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => {}
            Err(error) => return Err(RecordingError::Io(error)),
        }
        self.reader.read_exact(&mut bytes[1..]).map_err(map_eof)?;
        Ok(Some(SynchronizedRecord {
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
            counts: vec![u16::from_le_bytes(
                bytes[12..14]
                    .try_into()
                    .map_err(|_| RecordingError::Truncated)?,
            )],
            status_flags: 1,
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
    if input.layout != RecordLayout::Legacy {
        return export_multichannel_csv(&mut input, csv);
    }
    let first_timestamp = input.metadata.utc_start.timestamp_micros();
    let mut writer = BufWriter::new(File::create(csv)?);
    let profile = input.metadata.profile_snapshot.clone();
    let validation = input.metadata.validation_context.clone();
    if profile.is_some() && validation.is_some() {
        writer.write_all(b"sample_sequence,timestamp_us,elapsed_seconds,channel,adc_counts,volts,status_flags,profile_id,profile_version,signal_label,validation_id,validation_test_type,validation_run_number\n")?;
    } else if profile.is_some() {
        writer.write_all(b"sample_sequence,timestamp_us,elapsed_seconds,channel,adc_counts,volts,status_flags,profile_id,profile_version,signal_label\n")?;
    } else {
        writer.write_all(
            b"sample_sequence,timestamp_us,elapsed_seconds,channel,adc_counts,volts,status_flags\n",
        )?;
    }
    let mut count = 0u64;
    let mut origin = None;
    while let Some(sample) = input.next_sample()? {
        let start = *origin.get_or_insert(sample.timestamp_us);
        let elapsed = (sample.timestamp_us.saturating_sub(start)) as f64 / 1_000_000.0;
        if let (Some(snapshot), Some(context)) = (profile.as_ref(), validation.as_ref()) {
            writeln!(
                writer,
                "{},{},{elapsed:.6},{},{},{:.6},1,{},{},{},{},{},{}",
                sample.sequence,
                sample.timestamp_us,
                csv_field(&snapshot.profile.acquisition.analog_pin),
                sample.counts,
                f64::from(sample.counts) * 5.0 / 4095.0,
                csv_field(&snapshot.profile.profile_id),
                csv_field(&snapshot.profile.profile_version),
                csv_field(&snapshot.profile.export.signal_name),
                csv_field(&context.validation_id),
                csv_field(&context.test_type),
                context.run_number,
            )?;
        } else if let Some(snapshot) = profile.as_ref() {
            writeln!(
                writer,
                "{},{},{elapsed:.6},{},{},{:.6},1,{},{},{}",
                sample.sequence,
                sample.timestamp_us,
                csv_field(&snapshot.profile.acquisition.analog_pin),
                sample.counts,
                f64::from(sample.counts) * 5.0 / 4095.0,
                csv_field(&snapshot.profile.profile_id),
                csv_field(&snapshot.profile.profile_version),
                csv_field(&snapshot.profile.export.signal_name)
            )?;
        } else {
            writeln!(
                writer,
                "{},{},{elapsed:.6},A0,{},{:.6},1",
                sample.sequence,
                sample.timestamp_us,
                sample.counts,
                f64::from(sample.counts) * 5.0 / 4095.0
            )?;
        }
        count += 1;
    }
    let _ = first_timestamp; // retained for future host/board latency diagnostics.
    writer.flush()?;
    Ok(count)
}

fn export_multichannel_csv(input: &mut BmegReader, csv: &Path) -> Result<u64, RecordingError> {
    let snapshot = input
        .metadata
        .profile_snapshot
        .clone()
        .ok_or(RecordingError::InvalidRecordShape)?;
    let fields = snapshot.profile.acquisition.record_field_names();
    let pulseox = matches!(
        snapshot.profile.acquisition.acquisition_mode,
        crate::profiles::AcquisitionMode::Pulseox4State
    );
    let calibration = input.metadata.calibration.clone().unwrap_or_default();
    let adc_reference_v =
        if calibration.adc_reference_v.is_finite() && calibration.adc_reference_v > 0.0 {
            calibration.adc_reference_v
        } else {
            // Legacy files predate explicit Vref metadata and retain the documented 5 V assumption.
            DEFAULT_ADC_REFERENCE_V
        };
    let sensor_supply_v =
        if calibration.mpxv_sensor_supply_v.is_finite() && calibration.mpxv_sensor_supply_v > 0.0 {
            calibration.mpxv_sensor_supply_v
        } else {
            DEFAULT_MPXV_SUPPLY_V
        };
    let mut writer = BufWriter::new(File::create(csv)?);
    if pulseox {
        writeln!(writer, "cycle_index,t_us,{}", fields.join(","))?;
    } else {
        let channels = snapshot.profile.acquisition.resolved_channels();
        let mut columns = Vec::new();
        for channel in &channels {
            columns.push(channel.csv_name.clone());
            columns.push(voltage_column_name(&channel.csv_name));
            if let Some(preset) = calibration.for_channel(&channel.id) {
                match preset.calibration_type {
                    CalibrationType::FixedFormula => {
                        columns.push(derived_column_name(&channel.csv_name, "kPa"));
                        if channel.id == "mpxv"
                            || calibration
                                .channel_units
                                .get(&channel.id)
                                .is_some_and(|unit| unit == "mmhg")
                        {
                            columns.push(derived_column_name(&channel.csv_name, "mmHg"));
                        }
                    }
                    CalibrationType::Linear => {
                        columns.push(derived_column_name(&channel.csv_name, &preset.output_units));
                    }
                }
            }
        }
        writeln!(writer, "record_sequence,t_us,{}", columns.join(","))?;
    }
    let mut count = 0u64;
    while let Some(record) = input.next_record()? {
        if record.counts.len() != fields.len() {
            return Err(RecordingError::InvalidRecordShape);
        }
        write!(writer, "{},{}", record.sequence, record.timestamp_us)?;
        if pulseox {
            for counts in record.counts {
                write!(writer, ",{counts}")?;
            }
        } else {
            let channels = snapshot.profile.acquisition.resolved_channels();
            for (index, counts) in record.counts.iter().copied().enumerate() {
                let channel = channels
                    .get(index)
                    .ok_or(RecordingError::InvalidRecordShape)?;
                let volts = counts_to_volts(counts, input.metadata.adc_bits, adc_reference_v)
                    .map_err(|error| RecordingError::Csv(error.to_string()))?;
                write!(writer, ",{counts},{volts:.6}")?;
                if let Some(preset) = calibration.for_channel(&channel.id) {
                    match preset.calibration_type {
                        CalibrationType::FixedFormula => {
                            let kpa = mpxv_kpa(volts, sensor_supply_v)
                                .map_err(|error| RecordingError::Csv(error.to_string()))?;
                            write!(writer, ",{kpa:.6}")?;
                            if channel.id == "mpxv"
                                || calibration
                                    .channel_units
                                    .get(&channel.id)
                                    .is_some_and(|unit| unit == "mmhg")
                            {
                                let mmhg = mpxv_mmhg(volts, sensor_supply_v)
                                    .map_err(|error| RecordingError::Csv(error.to_string()))?;
                                write!(writer, ",{mmhg:.6}")?;
                            }
                        }
                        CalibrationType::Linear => {
                            let value = apply_linear(volts, preset)
                                .map_err(|error| RecordingError::Csv(error.to_string()))?;
                            write!(writer, ",{value:.6}")?;
                        }
                    }
                }
            }
        }
        writer.write_all(b"\n")?;
        count += 1;
    }
    writer.flush()?;
    Ok(count)
}

fn voltage_column_name(raw_column: &str) -> String {
    raw_column
        .strip_suffix("_counts")
        .map(|base| format!("{base}_V"))
        .unwrap_or_else(|| format!("{raw_column}_V"))
}

fn derived_column_name(raw_column: &str, units: &str) -> String {
    let base = raw_column.strip_suffix("_counts").unwrap_or(raw_column);
    let units = units
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>();
    format!(
        "{base}_{}",
        if units.is_empty() {
            "calibrated"
        } else {
            &units
        }
    )
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
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
            board_serial: None,
            com_port: "SIM".into(),
            fqbn: "simulator".into(),
            arduino_cli_version: "n/a".into(),
            uno_r4_core_version: "n/a".into(),
            firmware_build: 1,
            protocol_version: "0.1".into(),
            analog_pin: "A0".into(),
            active_analog_pins: vec!["A0".into()],
            digital_output_mapping: BTreeMap::new(),
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
            profile_snapshot: None,
            validation_context: None,
            markers: Vec::new(),
            calibration: None,
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
            board_serial: None,
            com_port: "SIM".into(),
            fqbn: "simulator".into(),
            arduino_cli_version: "n/a".into(),
            uno_r4_core_version: "n/a".into(),
            firmware_build: 1,
            protocol_version: "0.1".into(),
            analog_pin: "A0".into(),
            active_analog_pins: vec!["A0".into()],
            digital_output_mapping: BTreeMap::new(),
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
            profile_snapshot: None,
            validation_context: None,
            markers: Vec::new(),
            calibration: None,
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

    #[test]
    fn profile_provenance_is_embedded_and_csv_is_profile_aware() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let profile = crate::profiles::built_in_profiles()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .find(|profile| profile.category == "course_ecg")
            .unwrap_or_else(|| panic!("missing ECG profile"))
            .snapshot(true);
        let mut metadata = RecordingMetadata {
            utc_start: Utc::now(),
            local_start: Local::now(),
            board: "Simulator".into(),
            board_serial: None,
            com_port: "SIM".into(),
            fqbn: "simulator".into(),
            arduino_cli_version: "n/a".into(),
            uno_r4_core_version: "n/a".into(),
            firmware_build: 0x0001_0001,
            protocol_version: "0.1".into(),
            analog_pin: "A0".into(),
            active_analog_pins: vec!["A0".into()],
            digital_output_mapping: BTreeMap::new(),
            adc_bits: 12,
            requested_sample_rate_hz: 1000,
            measured_sample_rate_hz: 0.0,
            total_samples: 1,
            integrity: IntegrityCounters::default(),
            app_version: "test".into(),
            simulator: true,
            utc_stop: None,
            local_stop: None,
            host_elapsed_seconds: None,
            board_elapsed_seconds: None,
            recording_status: "complete".into(),
            bmeg_filename: "profile.bmeg".into(),
            csv_filename: None,
            notes: "synthetic".into(),
            duration_mode: Some("timed".into()),
            requested_duration_seconds: Some(10),
            stop_reason: Some(StopReason::TimedComplete),
            initial_free_disk_bytes: None,
            final_free_disk_bytes: None,
            completion_status: "complete".into(),
            profile_snapshot: Some(profile.clone()),
            validation_context: None,
            markers: vec![RecordingMarker {
                timestamp_us: 0,
                label: "baseline".into(),
            }],
            calibration: None,
        };
        let bmeg = dir.path().join("profile.bmeg");
        let mut writer =
            BmegWriter::create_synchronized(&bmeg, &metadata, 1).unwrap_or_else(|e| panic!("{e}"));
        writer
            .write_record(&SynchronizedRecord {
                sequence: 0,
                timestamp_us: 0,
                status_flags: 1,
                counts: vec![2048],
            })
            .unwrap_or_else(|e| panic!("{e}"));
        writer.finish().unwrap_or_else(|e| panic!("{e}"));
        let read_back = BmegReader::open(&bmeg).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(read_back.metadata.profile_snapshot, Some(profile));
        assert_eq!(read_back.metadata.markers[0].label, "baseline");
        let csv = dir.path().join("profile.csv");
        assert_eq!(
            export_bmeg_csv(&bmeg, &csv).unwrap_or_else(|e| panic!("{e}")),
            1
        );
        assert!(std::fs::read_to_string(csv)
            .unwrap_or_default()
            .starts_with("record_sequence,t_us,ecg_counts"));
        let bp = crate::profiles::built_in_profiles()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .find(|profile| profile.category == "course_blood_pressure")
            .unwrap_or_else(|| panic!("missing BP profile"))
            .snapshot(false);
        metadata.profile_snapshot = Some(bp.clone());
        metadata.calibration = Some(RecordingCalibration {
            adc_reference_v: 5.0,
            mpxv_sensor_supply_v: 5.0,
            channel_units: BTreeMap::new(),
            active_calibrations: vec![
                crate::calibration::fixed_mpxv_calibration(
                    bp.profile.profile_id.clone(),
                    "mpxv".into(),
                    5.0,
                    5.0,
                )
                .unwrap_or_else(|e| panic!("{e}")),
                crate::calibration::CalibrationPreset {
                    schema_version: 1,
                    calibration_id: "team.xgzp".into(),
                    profile_id: bp.profile.profile_id.clone(),
                    channel_id: "xgzp".into(),
                    calibration_type: crate::calibration::CalibrationType::Linear,
                    input_quantity: "volts".into(),
                    output_quantity: "pressure".into(),
                    output_units: "mmHg".into(),
                    parameters: BTreeMap::from([("slope".into(), 100.0), ("offset".into(), -5.0)]),
                    created_at: Utc::now(),
                    label: "Team XGZP".into(),
                },
            ],
        });
        let bp_bmeg = dir.path().join("bp.bmeg");
        let bp_csv = dir.path().join("bp.csv");
        let mut bp_writer = BmegWriter::create_synchronized(&bp_bmeg, &metadata, 3)
            .unwrap_or_else(|e| panic!("{e}"));
        bp_writer
            .write_record(&SynchronizedRecord {
                sequence: 1,
                timestamp_us: 1_000,
                status_flags: 1,
                counts: vec![100, 2048, 2048],
            })
            .unwrap_or_else(|e| panic!("{e}"));
        bp_writer.finish().unwrap_or_else(|e| panic!("{e}"));
        export_bmeg_csv(&bp_bmeg, &bp_csv).unwrap_or_else(|e| panic!("{e}"));
        let bp_text = std::fs::read_to_string(bp_csv).unwrap_or_else(|e| panic!("{e}"));
        assert!(bp_text.starts_with("record_sequence,t_us,ppg_counts,ppg_V,mpxv_counts,mpxv_V,mpxv_kPa,mpxv_mmHg,xgzp_counts,xgzp_V,xgzp_mmHg"));
        assert!(BmegReader::open(&bp_bmeg)
            .unwrap_or_else(|e| panic!("{e}"))
            .metadata
            .calibration
            .is_some());
        metadata.profile_snapshot = None;
        assert!(metadata.profile_snapshot.is_none());
    }

    #[test]
    fn legacy_validation_context_deserializes_without_changing_course_csv() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let profile = crate::profiles::built_in_profiles()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .find(|profile| profile.category == "course_ecg")
            .unwrap_or_else(|| panic!("missing ECG profile"))
            .snapshot(true);
        let metadata = RecordingMetadata {
            utc_start: Utc::now(),
            local_start: Local::now(),
            board: "Simulator".into(),
            board_serial: None,
            com_port: "SIM".into(),
            fqbn: "simulator".into(),
            arduino_cli_version: "n/a".into(),
            uno_r4_core_version: "n/a".into(),
            firmware_build: 0x0001_0001,
            protocol_version: "0.1".into(),
            analog_pin: "A0".into(),
            active_analog_pins: vec!["A0".into()],
            digital_output_mapping: BTreeMap::new(),
            adc_bits: 12,
            requested_sample_rate_hz: 1_000,
            measured_sample_rate_hz: 1_000.0,
            total_samples: 2,
            integrity: IntegrityCounters::default(),
            app_version: "test".into(),
            simulator: true,
            utc_stop: None,
            local_stop: None,
            host_elapsed_seconds: None,
            board_elapsed_seconds: None,
            recording_status: "complete".into(),
            bmeg_filename: "validation.bmeg".into(),
            csv_filename: None,
            notes: "bench only".into(),
            duration_mode: Some("timed".into()),
            requested_duration_seconds: Some(10),
            stop_reason: Some(StopReason::TimedComplete),
            initial_free_disk_bytes: None,
            final_free_disk_bytes: None,
            completion_status: "complete".into(),
            profile_snapshot: Some(profile),
            validation_context: Some(LegacyValidationContext {
                validation_id: "wvu.validation.001".into(),
                test_type: "dc_operating_range_sweep".into(),
                run_number: 1,
                bench_only: true,
                source_description: "2.5 V safe source".into(),
                source_setpoint_v: Some(2.5),
                source_offset_v: None,
                source_frequency_hz: None,
                source_peak_to_peak_v: None,
                equipment_metadata: BTreeMap::new(),
                simulator_parameters: BTreeMap::from([("seed".into(), "test".into())]),
            }),
            markers: Vec::new(),
            calibration: None,
        };
        let bmeg = dir.path().join("validation.bmeg");
        let csv = dir.path().join("validation.csv");
        let mut writer =
            BmegWriter::create_synchronized(&bmeg, &metadata, 1).unwrap_or_else(|e| panic!("{e}"));
        writer
            .write_record(&SynchronizedRecord {
                sequence: 0,
                timestamp_us: 0,
                status_flags: 1,
                counts: vec![2048],
            })
            .unwrap_or_else(|e| panic!("{e}"));
        writer
            .write_record(&SynchronizedRecord {
                sequence: 1,
                timestamp_us: 1_000,
                status_flags: 1,
                counts: vec![2048],
            })
            .unwrap_or_else(|e| panic!("{e}"));
        writer.finish().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            export_bmeg_csv(&bmeg, &csv).unwrap_or_else(|e| panic!("{e}")),
            2
        );
        let read_back = BmegReader::open(&bmeg).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            read_back.metadata.validation_context,
            metadata.validation_context
        );
        let header = std::fs::read_to_string(csv).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            header.lines().next(),
            Some("record_sequence,t_us,ecg_counts,ecg_V")
        );
    }
}
