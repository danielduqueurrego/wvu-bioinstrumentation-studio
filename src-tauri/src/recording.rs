//! `.bmeg`: `BMEGREC1` + u16 JSON-header length + UTF-8 header + streamed records.
//! Legacy single-channel records are `u32 sequence, u64 timestamp_us, u16 ADC counts`.
//! Current profile-aware records preserve a synchronized frame as
//! `u32 sequence, u64 timestamp_us, u16 status_flags, u16[field_count] counts`.
use crate::{
    calibration::{
        apply_linear, counts_to_volts, mpxv_kpa, mpxv_mmhg, CalibrationPreset, CalibrationType,
        RecordingCalibration, DEFAULT_ADC_REFERENCE_V, DEFAULT_MPXV_SUPPLY_V,
    },
    profiles::ProfileSnapshot,
    protocol::IntegrityCounters,
};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufReader, BufWriter, IntoInnerError, Read, Write},
    path::Path,
};
pub const BMEG_MAGIC: &[u8; 8] = b"BMEGREC1";
pub const BMEG_RECORD_BYTES: usize = 14;
/// Synchronized records carry at most eight u16 fields (pulse ox: TX/RX x
/// RED, DARK 1, IR, DARK 2). The bound keeps one record encodable on the stack.
pub const MAX_SYNCHRONIZED_FIELDS: usize = 8;
const SYNCHRONIZED_RECORD_PREFIX_BYTES: usize = 14;

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
    /// Final sidecars produced by protocol v0.4 include firmware-timestamped
    /// interval statistics. Historical BMEG headers and sidecars deserialize
    /// with this field absent.
    #[serde(default)]
    pub timing_diagnostics: Option<AcquisitionTimingDiagnostics>,
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
    /// The operator's optional note, preserved byte-for-byte as UTF-8 text.
    /// `notes` contains either this exact value or a profile-derived default.
    #[serde(default)]
    pub user_recording_note: Option<String>,
    /// Explicit recording-safety facts. These values are never inferred from
    /// the presence of a sensor or from a non-bench profile category.
    #[serde(default)]
    pub recording_safety: RecordingSafetyContext,
    /// User-selected destination provenance. This is optional so recordings made
    /// before the single-window Project folder workflow remain readable.
    #[serde(default)]
    pub project_folder: Option<String>,
    #[serde(default)]
    pub output_folder: Option<String>,
    /// Optional fields preserve read compatibility with early sidecars.
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
    /// A frozen lab definition is written into both the BMEG JSON header and the
    /// metadata sidecar. Files made before this field existed remain legacy/general recordings.
    #[serde(default)]
    pub profile_snapshot: Option<ProfileSnapshot>,
    /// Historical metadata retained only so previously recorded BMEG
    /// files deserialize safely after the class application removed Validation.
    #[serde(default)]
    /// Historical optional metadata accepted only to read older recordings.
    pub validation_context: Option<LegacyValidationContext>,
    /// Manual, non-protocol experiment annotations. Markers never alter or remove raw samples.
    #[serde(default)]
    pub markers: Vec<RecordingMarker>,
    /// Derived-unit settings captured at recording start. Raw BMEG records
    /// remain ADC counts regardless of whether any of these conversions are selected.
    #[serde(default)]
    pub calibration: Option<RecordingCalibration>,
    /// The raw record layout this file was written with. Readers trust it over
    /// any profile-derived guess; files written before it existed omit it and
    /// fall back to `protocol_version`/profile detection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_layout: Option<RecordLayout>,
    /// Host time when the recording session was prepared (before the
    /// handshake). `utc_start`/`local_start` are stamped immediately before
    /// START so they describe acquisition, not connection setup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_opened_utc: Option<DateTime<Utc>>,
    /// Host time at which the firmware acknowledged START. Only the final
    /// sidecar carries it because the BMEG header is written before START.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acquisition_started_utc: Option<DateTime<Utc>>,
    /// Why the derived CSV is absent. The raw BMEG remains authoritative and
    /// a CSV failure never removes the BMEG or this sidecar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub csv_error: Option<String>,
    /// Problems while flushing, renaming, or writing recording files. The raw
    /// file named by `bmeg_filename` is kept even when this is present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalize_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AcquisitionTimingDiagnostics {
    pub requested_rate_hz: u32,
    pub measured_rate_hz: f64,
    pub interval_count: u64,
    pub median_interval_us: f64,
    pub minimum_interval_us: u64,
    pub maximum_interval_us: u64,
    pub p99_interval_us: f64,
    /// Intervals outside +/- this percentage of the nominal interval.
    pub interval_tolerance_percent: f64,
    pub intervals_outside_tolerance: u64,
    pub acquisition_queue_overflows: u64,
    pub timestamp_source: String,
    /// Firmware-observed intervals outside +/-5% of the configured dwell,
    /// indexed as RED, DARK 1, IR, DARK 2. Absent for simultaneous analog
    /// captures and recordings made by earlier firmware.
    #[serde(default)]
    pub pulse_state_intervals_outside_tolerance: Option<[u32; 4]>,
    /// Maximum absolute firmware-observed dwell error in microseconds for
    /// RED, DARK 1, IR, and DARK 2 respectively.
    #[serde(default)]
    pub pulse_state_maximum_error_us: Option<[u32; 4]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordingSafetyContext {
    pub bench_only_profile: bool,
    pub human_connection_authorized: bool,
    pub bench_notice_acknowledged: bool,
    pub medical_validation_status: String,
}

impl Default for RecordingSafetyContext {
    fn default() -> Self {
        Self {
            bench_only_profile: false,
            human_connection_authorized: false,
            bench_notice_acknowledged: false,
            medical_validation_status: "not_medical_device".into(),
        }
    }
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

/// Raw record encoding. It is written into every new BMEG header so a reader
/// never has to infer it from a profile's minimum protocol version.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordLayout {
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
    #[error("not a supported BMEG recording")]
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
        if field_count == 0 || field_count > MAX_SYNCHRONIZED_FIELDS {
            return Err(RecordingError::InvalidRecordShape);
        }
        Self::create_with_layout(path, metadata, RecordLayout::Synchronized { field_count })
    }

    fn create_with_layout(
        path: &Path,
        metadata: &RecordingMetadata,
        layout: RecordLayout,
    ) -> Result<Self, RecordingError> {
        let header = encode_header(metadata, layout)?;
        let header_len = u16::try_from(header.len()).map_err(|_| RecordingError::HeaderTooLarge)?;
        // `create_new` reserves the name atomically: if another recording
        // (for example a second copy of the app) created this file after the
        // name was chosen, this fails instead of truncating or sharing it.
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(BMEG_MAGIC)?;
        writer.write_all(&header_len.to_le_bytes())?;
        writer.write_all(&header)?;
        Ok(Self {
            writer,
            records: 0,
            layout,
        })
    }

    /// Each record is handed to the buffer in one `write_all`. If the medium
    /// fails, the buffer therefore never holds a partial record that a later
    /// successful flush would append to the file out of alignment.
    pub fn write(&mut self, sample: RawSample) -> Result<(), RecordingError> {
        if self.layout != RecordLayout::Legacy {
            return Err(RecordingError::InvalidRecordShape);
        }
        let mut bytes = [0u8; BMEG_RECORD_BYTES];
        bytes[0..4].copy_from_slice(&sample.sequence.to_le_bytes());
        bytes[4..12].copy_from_slice(&sample.timestamp_us.to_le_bytes());
        bytes[12..14].copy_from_slice(&sample.counts.to_le_bytes());
        self.writer.write_all(&bytes)?;
        self.records += 1;
        Ok(())
    }

    pub fn write_record(&mut self, record: &SynchronizedRecord) -> Result<(), RecordingError> {
        let RecordLayout::Synchronized { field_count } = self.layout else {
            return Err(RecordingError::InvalidRecordShape);
        };
        if record.counts.len() != field_count || field_count > MAX_SYNCHRONIZED_FIELDS {
            return Err(RecordingError::InvalidRecordShape);
        }
        let mut bytes = [0u8; SYNCHRONIZED_RECORD_PREFIX_BYTES + 2 * MAX_SYNCHRONIZED_FIELDS];
        bytes[0..4].copy_from_slice(&record.sequence.to_le_bytes());
        bytes[4..12].copy_from_slice(&record.timestamp_us.to_le_bytes());
        bytes[12..14].copy_from_slice(&record.status_flags.to_le_bytes());
        for (index, counts) in record.counts.iter().enumerate() {
            let offset = SYNCHRONIZED_RECORD_PREFIX_BYTES + index * 2;
            bytes[offset..offset + 2].copy_from_slice(&counts.to_le_bytes());
        }
        let length = synchronized_record_bytes(field_count);
        self.writer.write_all(&bytes[..length])?;
        self.records += 1;
        Ok(())
    }

    /// Periodic flushing bounds the amount of validated data held by the OS
    /// buffer during a long recording without retaining samples in memory.
    /// It deliberately does not fsync: the capture thread also services serial
    /// I/O, and an fsync on slow USB or cloud-backed media could stall it.
    pub fn flush(&mut self) -> Result<(), RecordingError> {
        self.writer.flush()?;
        Ok(())
    }

    /// Flushes buffered records and asks the operating system to persist the
    /// file before it is renamed, so a power loss shortly after the UI reports
    /// a saved recording cannot leave an empty or truncated BMEG.
    ///
    /// A failed flush (data not written) is an error. A failed fsync is not:
    /// some network, virtual, and cloud-mounted drives reject
    /// `FlushFileBuffers` even though every byte was written, and treating
    /// that as data loss would fault recordings that were saved correctly.
    pub fn finish(self) -> Result<(), RecordingError> {
        let file = self
            .writer
            .into_inner()
            .map_err(IntoInnerError::into_error)?;
        sync_best_effort(&file, "bmeg");
        Ok(())
    }
}

/// Size in bytes of one synchronized BMEG record with `field_count` fields.
pub const fn synchronized_record_bytes(field_count: usize) -> usize {
    SYNCHRONIZED_RECORD_PREFIX_BYTES + field_count * 2
}

/// Asks the operating system to persist `file`. Durability is requested, not
/// required: the data were already written, so a failure is logged as
/// `FSYNC_FAILED` and the caller continues.
pub(crate) fn sync_best_effort(file: &File, label: &str) {
    if let Err(error) = file.sync_all() {
        crate::app_log::record(
            "WARN",
            &format!(
                "FSYNC_FAILED file={label} kind={:?} raw_os_error={:?} detail={error}",
                error.kind(),
                error.raw_os_error()
            ),
        );
    }
}

/// Serializes the JSON header exactly as a writer with `layout` would.
fn encode_header(
    metadata: &RecordingMetadata,
    layout: RecordLayout,
) -> Result<Vec<u8>, RecordingError> {
    let mut header_metadata = metadata.clone();
    header_metadata.record_layout = Some(layout);
    let header = serde_json::to_vec(&header_metadata)?;
    if header.len() > usize::from(u16::MAX) {
        return Err(RecordingError::HeaderTooLarge);
    }
    Ok(header)
}

/// Proves that a synchronized recording header fits the 64 KiB BMEG limit.
/// Capture calls this before the board is configured so an oversized lab
/// definition or note is reported as such, not as a later connection fault.
pub fn check_synchronized_header(
    metadata: &RecordingMetadata,
    field_count: usize,
) -> Result<usize, RecordingError> {
    if field_count == 0 || field_count > MAX_SYNCHRONIZED_FIELDS {
        return Err(RecordingError::InvalidRecordShape);
    }
    encode_header(metadata, RecordLayout::Synchronized { field_count }).map(|header| header.len())
}

/// Streaming reader for finalized BMEG recordings. It retains just one record.
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
        let layout = match metadata.record_layout {
            Some(RecordLayout::Synchronized { field_count })
                if field_count == 0 || field_count > MAX_SYNCHRONIZED_FIELDS =>
            {
                return Err(RecordingError::InvalidRecordShape);
            }
            Some(layout) => layout,
            None => historical_record_layout(&metadata),
        };
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
                        .as_chunks::<2>()
                        .0
                        .iter()
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

/// Layout detection for files written before `record_layout` existed. The
/// synchronized writer has recorded protocol 0.2 or later since synchronized
/// frames were introduced, so `protocol_version` identifies such files even
/// when their lab declared an older minimum. The previous profile-minimum rule
/// is kept so every file that was readable before remains readable.
fn historical_record_layout(metadata: &RecordingMetadata) -> RecordLayout {
    metadata
        .profile_snapshot
        .as_ref()
        .filter(|snapshot| {
            snapshot.profile.required_firmware.protocol_minor_min >= 2
                || protocol_version_is_synchronized(&metadata.protocol_version)
        })
        .map(|snapshot| RecordLayout::Synchronized {
            field_count: snapshot.profile.acquisition.record_field_names().len(),
        })
        .unwrap_or(RecordLayout::Legacy)
}

fn protocol_version_is_synchronized(version: &str) -> bool {
    let mut parts = version
        .split('.')
        .map(|part| part.trim().parse::<u32>().ok());
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(major), Some(minor)) => major > 0 || minor >= 2,
        _ => false,
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
    finish_csv(writer)?;
    Ok(count)
}

/// Flushes an exported CSV and requests an fsync so it is durable before the
/// caller renames it into place. Only the flush is required to succeed (see
/// `BmegWriter::finish`).
fn finish_csv(writer: BufWriter<File>) -> Result<(), RecordingError> {
    let file = writer.into_inner().map_err(IntoInnerError::into_error)?;
    sync_best_effort(&file, "csv");
    Ok(())
}

/// Loop-invariant export decisions for one simultaneous channel. Computing
/// them once keeps per-record work to arithmetic and formatting.
struct ChannelCsvPlan<'a> {
    preset: Option<&'a CalibrationPreset>,
    emit_mmhg: bool,
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
    let adc_bits = input.metadata.adc_bits;
    let mut writer = BufWriter::new(File::create(csv)?);
    // Per-channel decisions are fixed for the whole file. The column schema
    // (including the display-unit-dependent mmHg column) is unchanged here;
    // see the review note D-5 before altering it.
    let mut channel_plans: Vec<ChannelCsvPlan<'_>> = Vec::new();
    if pulseox {
        writeln!(writer, "cycle_index,t_us,{}", fields.join(","))?;
    } else {
        let mut columns = Vec::new();
        for channel in snapshot.profile.acquisition.resolved_channels() {
            columns.push(CsvColumn::fixed(channel.csv_name.clone()));
            columns.push(CsvColumn::fixed(voltage_column_name(&channel.csv_name)));
            let preset = calibration.for_channel(&channel.id);
            let emit_mmhg = channel.id == "mpxv"
                || calibration
                    .channel_units
                    .get(&channel.id)
                    .is_some_and(|unit| unit == "mmhg");
            if let Some(preset) = preset {
                match preset.calibration_type {
                    CalibrationType::FixedFormula => {
                        columns.push(CsvColumn::fixed(derived_column_name(
                            &channel.csv_name,
                            "kPa",
                        )));
                        if emit_mmhg {
                            columns.push(CsvColumn::fixed(derived_column_name(
                                &channel.csv_name,
                                "mmHg",
                            )));
                        }
                    }
                    CalibrationType::Linear => {
                        columns.push(CsvColumn::calibrated(
                            &channel.csv_name,
                            &preset.output_units,
                        ));
                    }
                }
            }
            channel_plans.push(ChannelCsvPlan { preset, emit_mmhg });
        }
        writeln!(
            writer,
            "record_sequence,t_us,{}",
            csv_header_names(columns).join(",")
        )?;
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
            if record.counts.len() != channel_plans.len() {
                return Err(RecordingError::InvalidRecordShape);
            }
            for (counts, plan) in record.counts.iter().copied().zip(&channel_plans) {
                let volts = counts_to_volts(counts, adc_bits, adc_reference_v)
                    .map_err(|error| RecordingError::Csv(error.to_string()))?;
                write!(writer, ",{counts},{volts:.6}")?;
                if let Some(preset) = plan.preset {
                    match preset.calibration_type {
                        CalibrationType::FixedFormula => {
                            let kpa = mpxv_kpa(volts, sensor_supply_v)
                                .map_err(|error| RecordingError::Csv(error.to_string()))?;
                            write!(writer, ",{kpa:.6}")?;
                            if plan.emit_mmhg {
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
    finish_csv(writer)?;
    Ok(count)
}

/// One planned CSV column. A linear-calibration column is named from the
/// student's unit, so it may collide with another column; it then gets a
/// `_cal_` infix instead (only on collision, so existing names are kept).
struct CsvColumn {
    name: String,
    calibrated_alternative: Option<String>,
}

impl CsvColumn {
    fn fixed(name: String) -> Self {
        Self {
            name,
            calibrated_alternative: None,
        }
    }
    fn calibrated(raw_column: &str, units: &str) -> Self {
        let base = raw_column.strip_suffix("_counts").unwrap_or(raw_column);
        let derived = derived_column_name(raw_column, units);
        let suffix = derived
            .strip_prefix(base)
            .and_then(|rest| rest.strip_prefix('_'))
            .unwrap_or("calibrated")
            .to_string();
        Self {
            name: derived,
            calibrated_alternative: Some(format!("{base}_cal_{suffix}")),
        }
    }
}

/// Final, unique, CSV-safe header names. The first column with a name keeps
/// it; a later duplicate (for example a calibration in volts next to the
/// `_V` column) is renamed, never silently repeated.
fn csv_header_names(columns: Vec<CsvColumn>) -> Vec<String> {
    let mut used: std::collections::BTreeSet<String> = ["record_sequence", "t_us"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let mut names = Vec::with_capacity(columns.len());
    for column in columns {
        let mut name = column.name;
        if used.contains(&name) {
            if let Some(alternative) = column.calibrated_alternative {
                name = alternative;
            }
        }
        if used.contains(&name) {
            let stem = name.clone();
            let mut index = 2u32;
            while used.contains(&name) {
                name = format!("{stem}_{index}");
                index += 1;
            }
        }
        used.insert(name.clone());
        names.push(csv_header_field(&name));
    }
    names
}

/// A header cell that cannot split or shift columns and is not run as a
/// spreadsheet formula: names starting with `=`, `+`, `-`, `@`, tab, or CR
/// get a leading `'`, and names with a comma, quote, or line break are quoted
/// (RFC 4180). Safe names (every factory name) are written unchanged.
fn csv_header_field(name: &str) -> String {
    let guarded = if name.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{name}")
    } else {
        name.to_owned()
    };
    csv_field(&guarded)
}

pub(crate) fn voltage_column_name(raw_column: &str) -> String {
    raw_column
        .strip_suffix("_counts")
        .map(|base| format!("{base}_V"))
        .unwrap_or_else(|| format!("{raw_column}_V"))
}

pub(crate) fn derived_column_name(raw_column: &str, units: &str) -> String {
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
/// Moves a finished file to its final name, failing with `AlreadyExists`
/// instead of replacing a file that is already there (`fs::rename` replaces
/// on Windows). Both paths are in the same folder.
pub fn move_without_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
        let wide = |path: &Path| {
            path.as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>()
        };
        let (source, destination) = (wide(from), wide(to));
        // SAFETY: both arguments are NUL-terminated UTF-16 paths that outlive
        // the call. Without MOVEFILE_REPLACE_EXISTING an existing destination
        // makes the call fail (ERROR_ALREADY_EXISTS) and changes nothing.
        let moved = unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        };
        if moved == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(not(windows))]
    {
        std::fs::hard_link(from, to)?;
        std::fs::remove_file(from)
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
            timing_diagnostics: None,
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
            user_recording_note: None,
            recording_safety: RecordingSafetyContext::default(),
            project_folder: None,
            output_folder: None,
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
            record_layout: None,
            session_opened_utc: None,
            acquisition_started_utc: None,
            csv_error: None,
            finalize_error: None,
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
    fn legacy_metadata_without_new_fields_still_deserializes() {
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
            timing_diagnostics: None,
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
            user_recording_note: None,
            recording_safety: RecordingSafetyContext::default(),
            project_folder: None,
            output_folder: None,
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
            record_layout: None,
            session_opened_utc: None,
            acquisition_started_utc: None,
            csv_error: None,
            finalize_error: None,
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
            "timing_diagnostics",
            "user_recording_note",
            "recording_safety",
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
            timing_diagnostics: None,
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
            user_recording_note: None,
            recording_safety: RecordingSafetyContext::default(),
            project_folder: None,
            output_folder: None,
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
            record_layout: None,
            session_opened_utc: None,
            acquisition_started_utc: None,
            csv_error: None,
            finalize_error: None,
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
        let pulseox = crate::profiles::built_in_profiles()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .find(|profile| profile.category == "course_pulseox")
            .unwrap_or_else(|| panic!("missing pulse-ox profile"))
            .snapshot(false);
        metadata.profile_snapshot = Some(pulseox);
        metadata.calibration = None;
        metadata.adc_bits = 14;
        let pulse_bmeg = dir.path().join("pulse.bmeg");
        let pulse_csv = dir.path().join("pulse.csv");
        let mut pulse_writer = BmegWriter::create_synchronized(&pulse_bmeg, &metadata, 8)
            .unwrap_or_else(|e| panic!("{e}"));
        pulse_writer
            .write_record(&SynchronizedRecord {
                sequence: 7,
                timestamp_us: 28_000,
                status_flags: 1,
                counts: vec![101, 102, 103, 104, 201, 202, 203, 204],
            })
            .unwrap_or_else(|e| panic!("{e}"));
        pulse_writer
            .write_record(&SynchronizedRecord {
                sequence: 8,
                timestamp_us: 32_750,
                status_flags: 1,
                counts: vec![111, 112, 113, 114, 211, 212, 213, 214],
            })
            .unwrap_or_else(|e| panic!("{e}"));
        pulse_writer.finish().unwrap_or_else(|e| panic!("{e}"));
        export_bmeg_csv(&pulse_bmeg, &pulse_csv).unwrap_or_else(|e| panic!("{e}"));
        let pulse_text = std::fs::read_to_string(pulse_csv).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            pulse_text.lines().next(),
            Some("cycle_index,t_us,red_TX,dark1_TX,ir_TX,dark2_TX,red_RX,dark1_RX,ir_RX,dark2_RX")
        );
        assert_eq!(
            pulse_text.lines().nth(1),
            Some("7,28000,101,102,103,104,201,202,203,204")
        );
        assert_eq!(
            pulse_text.lines().nth(2),
            Some("8,32750,111,112,113,114,211,212,213,214")
        );
        let mut pulse_reader = BmegReader::open(&pulse_bmeg).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            pulse_reader
                .next_record()
                .unwrap_or_else(|e| panic!("{e}"))
                .map(|record| record.timestamp_us),
            Some(28_000)
        );
        assert_eq!(
            pulse_reader
                .next_record()
                .unwrap_or_else(|e| panic!("{e}"))
                .map(|record| record.timestamp_us),
            Some(32_750)
        );
        metadata.profile_snapshot = None;
        assert!(metadata.profile_snapshot.is_none());
    }

    fn layout_test_metadata(profile: Option<ProfileSnapshot>, protocol: &str) -> RecordingMetadata {
        RecordingMetadata {
            utc_start: Utc::now(),
            local_start: Local::now(),
            board: "Simulator".into(),
            board_serial: None,
            com_port: "SIM".into(),
            fqbn: "simulator".into(),
            arduino_cli_version: "n/a".into(),
            uno_r4_core_version: "n/a".into(),
            firmware_build: 0x0001_0006,
            protocol_version: protocol.into(),
            analog_pin: "A0".into(),
            active_analog_pins: vec!["A0".into()],
            digital_output_mapping: BTreeMap::new(),
            adc_bits: 12,
            requested_sample_rate_hz: 1_000,
            measured_sample_rate_hz: 0.0,
            timing_diagnostics: None,
            total_samples: 1,
            integrity: IntegrityCounters::default(),
            app_version: "test".into(),
            simulator: true,
            utc_stop: None,
            local_stop: None,
            host_elapsed_seconds: None,
            board_elapsed_seconds: None,
            recording_status: "active".into(),
            bmeg_filename: "layout.bmeg".into(),
            csv_filename: None,
            notes: "layout".into(),
            user_recording_note: None,
            recording_safety: RecordingSafetyContext::default(),
            project_folder: None,
            output_folder: None,
            duration_mode: Some("timed".into()),
            requested_duration_seconds: Some(10),
            stop_reason: None,
            initial_free_disk_bytes: None,
            final_free_disk_bytes: None,
            completion_status: "active".into(),
            profile_snapshot: profile,
            validation_context: None,
            markers: Vec::new(),
            calibration: None,
            record_layout: None,
            session_opened_utc: None,
            acquisition_started_utc: None,
            csv_error: None,
            finalize_error: None,
        }
    }

    #[test]
    fn a_recording_never_truncates_or_replaces_an_existing_file() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let metadata = layout_test_metadata(None, "0.4");
        let taken = dir.path().join("taken.bmeg.part");
        std::fs::write(&taken, b"another recording").unwrap_or_else(|e| panic!("{e}"));
        let refused = BmegWriter::create_synchronized(&taken, &metadata, 1);
        assert!(matches!(
            refused,
            Err(RecordingError::Io(ref error)) if error.kind() == std::io::ErrorKind::AlreadyExists
        ));
        assert_eq!(
            std::fs::read(&taken).unwrap_or_default(),
            b"another recording"
        );

        let finished = dir.path().join("mine.bmeg.part");
        BmegWriter::create_synchronized(&finished, &metadata, 1)
            .and_then(BmegWriter::finish)
            .unwrap_or_else(|e| panic!("{e}"));
        let destination = dir.path().join("final.bmeg");
        std::fs::write(&destination, b"kept").unwrap_or_else(|e| panic!("{e}"));
        let error = move_without_replacing(&finished, &destination)
            .err()
            .unwrap_or_else(|| panic!("an existing file was replaced"));
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&destination).unwrap_or_default(), b"kept");
        assert!(finished.is_file());

        std::fs::remove_file(&destination).unwrap_or_else(|e| panic!("{e}"));
        move_without_replacing(&finished, &destination).unwrap_or_else(|e| panic!("{e}"));
        assert!(destination.is_file());
        assert!(!finished.exists());
    }

    fn ecg_with_protocol_minimum(minor: u8) -> ProfileSnapshot {
        let mut profile = crate::profiles::built_in_profiles()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .find(|profile| profile.category == "course_ecg")
            .unwrap_or_else(|| panic!("missing ECG profile"));
        profile.required_firmware.protocol_minor_min = minor;
        profile.snapshot(true)
    }

    #[test]
    fn explicit_record_layout_reads_a_lab_with_an_old_protocol_minimum() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let metadata = layout_test_metadata(Some(ecg_with_protocol_minimum(1)), "0.4");
        let bmeg = dir.path().join("old-minimum.bmeg");
        let mut writer =
            BmegWriter::create_synchronized(&bmeg, &metadata, 1).unwrap_or_else(|e| panic!("{e}"));
        for sequence in 0..3u32 {
            writer
                .write_record(&SynchronizedRecord {
                    sequence,
                    timestamp_us: u64::from(sequence) * 1_000,
                    status_flags: 1,
                    counts: vec![2_048 + sequence as u16],
                })
                .unwrap_or_else(|e| panic!("{e}"));
        }
        writer.finish().unwrap_or_else(|e| panic!("{e}"));
        let mut reader = BmegReader::open(&bmeg).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            reader.metadata.record_layout,
            Some(RecordLayout::Synchronized { field_count: 1 })
        );
        assert_eq!(
            reader
                .next_record()
                .unwrap_or_else(|e| panic!("{e}"))
                .map(|record| record.counts),
            Some(vec![2_048])
        );
        let csv = dir.path().join("old-minimum.csv");
        assert_eq!(
            export_bmeg_csv(&bmeg, &csv).unwrap_or_else(|e| panic!("{e}")),
            3
        );
    }

    #[test]
    fn historical_headers_without_a_layout_use_the_recorded_protocol_version() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        // A synchronized file written before `record_layout` existed by a lab
        // whose declared minimum was protocol 0.1.
        let metadata = layout_test_metadata(Some(ecg_with_protocol_minimum(1)), "0.4");
        let header = serde_json::to_vec(&metadata).unwrap_or_else(|e| panic!("{e}"));
        assert!(!String::from_utf8_lossy(&header).contains("record_layout"));
        let header_len = u16::try_from(header.len()).unwrap_or_else(|e| panic!("{e}"));
        let mut record = Vec::new();
        record.extend_from_slice(&7u32.to_le_bytes());
        record.extend_from_slice(&1_000u64.to_le_bytes());
        record.extend_from_slice(&1u16.to_le_bytes());
        record.extend_from_slice(&1_234u16.to_le_bytes());
        let synchronized = dir.path().join("historical-synchronized.bmeg");
        let mut file_bytes = BMEG_MAGIC.to_vec();
        file_bytes.extend_from_slice(&header_len.to_le_bytes());
        file_bytes.extend_from_slice(&header);
        file_bytes.extend_from_slice(&record);
        std::fs::write(&synchronized, &file_bytes).unwrap_or_else(|e| panic!("{e}"));
        let mut reader = BmegReader::open(&synchronized).unwrap_or_else(|e| panic!("{e}"));
        let read = reader
            .next_record()
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("missing record"));
        assert_eq!(read.sequence, 7);
        assert_eq!(read.counts, vec![1_234]);
        assert_eq!(reader.next_record().unwrap_or_else(|e| panic!("{e}")), None);

        // A genuine legacy file (protocol 0.1, minimum 0.1) keeps the 14-byte layout.
        let legacy_metadata = layout_test_metadata(Some(ecg_with_protocol_minimum(1)), "0.1");
        let legacy_header = serde_json::to_vec(&legacy_metadata).unwrap_or_else(|e| panic!("{e}"));
        let legacy_len = u16::try_from(legacy_header.len()).unwrap_or_else(|e| panic!("{e}"));
        let mut legacy_record = Vec::new();
        legacy_record.extend_from_slice(&3u32.to_le_bytes());
        legacy_record.extend_from_slice(&2_000u64.to_le_bytes());
        legacy_record.extend_from_slice(&4_000u16.to_le_bytes());
        let legacy = dir.path().join("historical-legacy.bmeg");
        let mut legacy_bytes = BMEG_MAGIC.to_vec();
        legacy_bytes.extend_from_slice(&legacy_len.to_le_bytes());
        legacy_bytes.extend_from_slice(&legacy_header);
        legacy_bytes.extend_from_slice(&legacy_record);
        std::fs::write(&legacy, &legacy_bytes).unwrap_or_else(|e| panic!("{e}"));
        let mut legacy_reader = BmegReader::open(&legacy).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            legacy_reader
                .next_sample()
                .unwrap_or_else(|e| panic!("{e}")),
            Some(RawSample {
                sequence: 3,
                timestamp_us: 2_000,
                counts: 4_000,
            })
        );
    }

    #[test]
    fn oversized_headers_are_rejected_before_a_file_is_created() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let mut metadata = layout_test_metadata(Some(ecg_with_protocol_minimum(4)), "0.4");
        assert!(check_synchronized_header(&metadata, 1).is_ok());
        metadata.notes = "x".repeat(usize::from(u16::MAX));
        assert!(matches!(
            check_synchronized_header(&metadata, 1),
            Err(RecordingError::HeaderTooLarge)
        ));
        let bmeg = dir.path().join("too-large.bmeg");
        assert!(matches!(
            BmegWriter::create_synchronized(&bmeg, &metadata, 1),
            Err(RecordingError::HeaderTooLarge)
        ));
        assert!(!bmeg.exists());
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
            timing_diagnostics: None,
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
            user_recording_note: None,
            recording_safety: RecordingSafetyContext::default(),
            project_folder: None,
            output_folder: None,
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
            record_layout: None,
            session_opened_utc: None,
            acquisition_started_utc: None,
            csv_error: None,
            finalize_error: None,
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

    #[test]
    fn csv_header_names_stay_unique_and_cannot_break_the_header() {
        // Existing names are unchanged; a calibration in volts next to the
        // `_V` column and one in counts next to the raw column get `_cal_`.
        let names = csv_header_names(vec![
            CsvColumn::fixed("general_a0_counts".into()),
            CsvColumn::fixed(voltage_column_name("general_a0_counts")),
            CsvColumn::calibrated("general_a0_counts", "V"),
        ]);
        assert_eq!(
            names,
            ["general_a0_counts", "general_a0_V", "general_a0_cal_V"]
        );
        let names = csv_header_names(vec![
            CsvColumn::fixed("x_counts".into()),
            CsvColumn::fixed(voltage_column_name("x_counts")),
            CsvColumn::calibrated("x_counts", "counts"),
        ]);
        assert_eq!(names, ["x_counts", "x_V", "x_cal_counts"]);
        // The BP XGZP column keeps its documented name.
        let names = csv_header_names(vec![
            CsvColumn::fixed("xgzp_counts".into()),
            CsvColumn::fixed(voltage_column_name("xgzp_counts")),
            CsvColumn::calibrated("xgzp_counts", "mmHg"),
        ]);
        assert_eq!(names, ["xgzp_counts", "xgzp_V", "xgzp_mmHg"]);
        // Instructor names `x` and `x_V` (stored before the stricter rules).
        let names = csv_header_names(vec![
            CsvColumn::fixed("x".into()),
            CsvColumn::fixed(voltage_column_name("x")),
            CsvColumn::fixed("x_V".into()),
            CsvColumn::fixed(voltage_column_name("x_V")),
            CsvColumn::fixed("t_us".into()),
        ]);
        assert_eq!(names, ["x", "x_V", "x_V_2", "x_V_V", "t_us_2"]);
        // Unsafe stored names are quoted and cannot run as formulas.
        let names = csv_header_names(vec![
            CsvColumn::fixed("Vout, filtered".into()),
            CsvColumn::fixed("say \"hi\"".into()),
            CsvColumn::fixed("=HYPERLINK(\"x\")".into()),
            CsvColumn::fixed("-1".into()),
        ]);
        assert_eq!(
            names,
            [
                "\"Vout, filtered\"",
                "\"say \"\"hi\"\"\"",
                "\"'=HYPERLINK(\"\"x\"\")\"",
                "'-1"
            ]
        );
    }
}
