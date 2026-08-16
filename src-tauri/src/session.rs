//! Production acquisition-session controller.
//!
//! The controller is shared by Tauri commands, but a worker owns every blocking
//! transport read and disk write. Status and the bounded display history are copied
//! under a short mutex so polling the UI never waits for serial I/O.
use crate::{
    acquisition::{AcquisitionController, AcquisitionSnapshot, FirmwareCapabilities},
    calibration::RecordingCalibration,
    profiles::{built_in_profiles, ProfileSnapshot, ProfileStatus},
    protocol::{
        encode_frame, Frame, FrameParser, IntegrityCounters, MessageType, SampleBatch,
        CONTROLLED_SERIAL_BAUD, REFERENCE_DEVICE_ID, REFERENCE_FIRMWARE_BUILD,
    },
    recording::{
        export_bmeg_csv, BmegWriter, RecordingDuration, RecordingMarker, RecordingMetadata,
        StopReason, SynchronizedRecord,
    },
};
use chrono::{DateTime, Duration as ChronoDuration, Local, Utc};
use serde::Serialize;
use std::{
    collections::VecDeque,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{sync_channel, Receiver},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Display data are deliberately independent of the raw BMEG writer.  Retain
/// enough full-rate frames to satisfy the longest supported 30-second live
/// window at the maximum 1 kHz course frame rate, then decimate only the UI
/// response when necessary.
pub const DISPLAY_HISTORY_SECONDS: f64 = 30.0;
pub const DISPLAY_CAPACITY: usize = 30_000;
pub const DEFAULT_DISPLAY_WINDOW_SECONDS: f64 = 5.0;
pub const MAX_DISPLAY_RENDER_POINTS: usize = 2_000;
const MIN_DISPLAY_WINDOW_SECONDS: f64 = 0.5;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const HANDSHAKE_STARTUP_GRACE: Duration = Duration::from_millis(1_250);
const HANDSHAKE_OVERALL_TIMEOUT: Duration = Duration::from_secs(8);
const HANDSHAKE_PING_ATTEMPTS: u32 = 3;
const HANDSHAKE_PING_INTERVAL: Duration = Duration::from_secs(2);
const RESET_ENUMERATION_TIMEOUT: Duration = Duration::from_secs(10);
const RESET_ENUMERATION_POLL: Duration = Duration::from_millis(300);
const RESET_ENUMERATION_SETTLE: Duration = Duration::from_secs(2);
const RESET_APPLICATION_GRACE: Duration = Duration::from_secs(2);
const RESET_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(12);
const DISK_WARNING_BYTES: u64 = 1024 * 1024 * 1024;
const DISK_CRITICAL_BYTES: u64 = 250 * 1024 * 1024;
const DISK_CHECK_INTERVAL: Duration = Duration::from_secs(15);
const RECORDING_FLUSH_INTERVAL: Duration = Duration::from_secs(5);
/// `serialport` maps one configured timeout to both Windows read and write
/// timeouts.  Twenty-five milliseconds is useful for short startup probes,
/// but is too aggressive for a live 921600-baud CDC session: Windows may abort
/// a harmless PING write with ERROR_OPERATION_ABORTED (995) even while sample
/// frames are arriving.  This remains far below the firmware's five-second
/// host-command watchdog and keeps Stop responsiveness bounded.
const ACTIVE_SERIAL_IO_TIMEOUT: Duration = Duration::from_millis(250);
/// The reference firmware's host-command watchdog expires after five seconds.
/// Keep PINGs frequent enough to leave room for a small number of transient
/// Windows CDC write timeouts without pretending that a persistent transport
/// failure is healthy.
const ACTIVE_PING_INTERVAL: Duration = Duration::from_secs(1);
const PING_RECENT_SAMPLE_GRACE: Duration = Duration::from_secs(2);
const PING_KEEPALIVE_RETRY_WINDOW: Duration = Duration::from_secs(4);
const MAX_DEFERRED_PING_FAILURES: u8 = 3;
/// A healthy course stream produces a sample batch far more frequently than
/// this.  The bounded allowance covers ordinary Windows scheduling pauses, but
/// prevents a dead stream from remaining visibly "Recording" forever.
const SAMPLE_STREAM_STALL_TIMEOUT: Duration = Duration::from_secs(7);

fn normalize_display_window_seconds(window_seconds: f64) -> f64 {
    if window_seconds.is_finite() {
        window_seconds.clamp(MIN_DISPLAY_WINDOW_SECONDS, DISPLAY_HISTORY_SECONDS)
    } else {
        DEFAULT_DISPLAY_WINDOW_SECONDS
    }
}

fn normalize_display_max_points(max_points: usize) -> usize {
    max_points.clamp(1, MAX_DISPLAY_RENDER_POINTS)
}

/// Filters a chronological display cache by timestamp and deterministically
/// decimates it for rendering. The final element is always the exact newest
/// record, even when the interval contains more records than the UI budget.
fn select_display_records(
    records: &VecDeque<SynchronizedRecord>,
    window_seconds: f64,
    max_points: usize,
) -> Vec<SynchronizedRecord> {
    let Some(latest) = records.back() else {
        return Vec::new();
    };
    let window_us = (window_seconds * 1_000_000.0).round() as u64;
    let earliest_timestamp = latest.timestamp_us.saturating_sub(window_us);
    let filtered: Vec<SynchronizedRecord> = records
        .iter()
        .filter(|record| record.timestamp_us >= earliest_timestamp)
        .cloned()
        .collect();

    if filtered.len() <= max_points {
        return filtered;
    }
    if max_points == 1 {
        return filtered.last().cloned().into_iter().collect();
    }

    // Include both interval endpoints with evenly distributed indices. The
    // newest exact raw display record is therefore never replaced by a stride
    // approximation, while recorded BMEG/CSV frames are never decimated.
    let last_index = filtered.len() - 1;
    (0..max_points)
        .map(|index| {
            let selected = index * last_index / (max_points - 1);
            filtered[selected].clone()
        })
        .collect()
}

/// Kept behind a small interface so storage guard behavior is deterministic in
/// tests and the production implementation remains Windows-compatible.
trait DiskSpaceProvider: Send + Sync {
    fn available_space(&self, path: &Path) -> std::io::Result<u64>;
}

struct SystemDiskSpace;

impl DiskSpaceProvider for SystemDiskSpace {
    fn available_space(&self, path: &Path) -> std::io::Result<u64> {
        fs2::available_space(path)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionFailureCategory {
    PortBusy,
    PortOpenNoBytes,
    NonProtocolBytes,
    ProtocolCrcFailure,
    WrongProtocolVersion,
    MissingFirmwareIdentity,
    IncompatibleFirmwareIdentity,
    HandshakeIncomplete,
    /// A serial operation failed, but Windows still enumerated the selected
    /// port or the device identity could not be proven absent.
    SerialTransportError,
    /// The controlled firmware reported an explicit protocol-level capture
    /// error, such as its host-command watchdog expiring.
    FirmwareProtocolError,
    /// The port remained open, but no synchronized sample was received within
    /// the bounded active-stream deadline.
    NoDataTimeout,
    DeviceDisconnected,
    ResetPortDidNotReturn,
    ResetReturnedDifferentPort,
    AmbiguousReturningDevice,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConnectionDiagnostics {
    pub selected_port: String,
    pub board: String,
    pub fqbn: String,
    pub port_opened: bool,
    pub bytes_received: u64,
    pub valid_frames: u64,
    pub crc_failures: u64,
    pub skipped_noise_bytes: u64,
    pub hello_received: bool,
    pub capabilities_received: bool,
    pub pong_received: bool,
    pub protocol_version: Option<String>,
    pub firmware_capabilities: Option<FirmwareCapabilities>,
    pub firmware_build: Option<u32>,
    pub firmware_board_id: Option<u32>,
    pub raw_byte_classification: String,
    pub ping_attempts: u32,
    pub handshake_elapsed_ms: u128,
    pub reset_attempted: bool,
    pub original_port: Option<String>,
    pub final_port: Option<String>,
    pub disappearance_observed: bool,
    pub reappearance_observed: bool,
    pub bootloader_observed: bool,
    pub failure_category: Option<ConnectionFailureCategory>,
    pub recommended_action: String,
    /// Capture-time serial diagnostics are intentionally retained only in
    /// Advanced details/application.log. They distinguish a host transport
    /// failure from a proven USB device removal.
    pub terminal_error_classification: Option<String>,
    pub terminal_error_stage: Option<String>,
    pub terminal_error_kind: Option<String>,
    pub terminal_error_raw_os_error: Option<i32>,
    pub terminal_error_detail: Option<String>,
    pub terminal_error_elapsed_ms: Option<u128>,
    pub last_valid_packet_utc: Option<String>,
    pub last_valid_sample_utc: Option<String>,
    pub last_successful_ping_utc: Option<String>,
    pub last_pong_or_status_utc: Option<String>,
    pub selected_port_present_after_error: Option<bool>,
    pub same_vid_pid_present_after_error: Option<bool>,
    pub same_serial_present_after_error: Option<bool>,
    pub uno_r4_present_after_error: Option<bool>,
    pub port_enumeration_error: Option<String>,
}

impl ConnectionDiagnostics {
    fn new(port: impl Into<String>) -> Self {
        let selected_port = port.into();
        Self {
            selected_port: selected_port.clone(),
            board: "Arduino UNO R4 WiFi".into(),
            fqbn: "arduino:renesas_uno:unor4wifi".into(),
            port_opened: false,
            bytes_received: 0,
            valid_frames: 0,
            crc_failures: 0,
            skipped_noise_bytes: 0,
            hello_received: false,
            capabilities_received: false,
            pong_received: false,
            protocol_version: None,
            firmware_capabilities: None,
            firmware_build: None,
            firmware_board_id: None,
            raw_byte_classification: "no bytes received".into(),
            ping_attempts: 0,
            handshake_elapsed_ms: 0,
            reset_attempted: false,
            original_port: Some(selected_port),
            final_port: None,
            disappearance_observed: false,
            reappearance_observed: false,
            bootloader_observed: false,
            failure_category: None,
            recommended_action: "Retry handshake or refresh devices.".into(),
            terminal_error_classification: None,
            terminal_error_stage: None,
            terminal_error_kind: None,
            terminal_error_raw_os_error: None,
            terminal_error_detail: None,
            terminal_error_elapsed_ms: None,
            last_valid_packet_utc: None,
            last_valid_sample_utc: None,
            last_successful_ping_utc: None,
            last_pong_or_status_utc: None,
            selected_port_present_after_error: None,
            same_vid_pid_present_after_error: None,
            same_serial_present_after_error: None,
            uno_r4_present_after_error: None,
            port_enumeration_error: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ResetRetryResult {
    pub original_port: String,
    pub final_port: Option<String>,
    pub handshake_succeeded: bool,
    pub diagnostics: ConnectionDiagnostics,
}

#[derive(Clone, Debug, Serialize)]
pub struct HandshakeRetryResult {
    pub handshake_succeeded: bool,
    pub diagnostics: ConnectionDiagnostics,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResetTarget {
    pub port: String,
    pub serial_number: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum SessionState {
    Disconnected,
    Connecting,
    Connected,
    Configured,
    Acquiring,
    Stopping,
    Faulted,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionSummary {
    pub state: SessionState,
    pub samples: u64,
    pub packets: u64,
    pub measured_rate_hz: f64,
    pub board_elapsed_seconds: f64,
    pub host_elapsed_seconds: f64,
    pub bmeg_path: String,
    pub csv_path: String,
    pub metadata_path: String,
    pub recording_status: String,
    pub duration: RecordingDuration,
    pub stop_reason: StopReason,
    pub completion_status: String,
    pub initial_free_disk_bytes: Option<u64>,
    pub final_free_disk_bytes: Option<u64>,
    pub integrity: IntegrityCounters,
    pub error: Option<String>,
    pub profile: ProfileSnapshot,
    pub calibration: RecordingCalibration,
    pub active_digital_output_mask: Option<u8>,
    pub final_digital_output_mask: Option<u8>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionStatus {
    pub state: SessionState,
    pub board: String,
    pub port: String,
    pub protocol_version: String,
    pub simulator: bool,
    pub samples: u64,
    pub packets: u64,
    pub measured_rate_hz: f64,
    pub integrity: IntegrityCounters,
    pub duration: Option<RecordingDuration>,
    pub elapsed_seconds: f64,
    pub remaining_seconds: Option<f64>,
    pub available_disk_bytes: Option<u64>,
    pub storage_warning: Option<String>,
    pub stop_reason: Option<StopReason>,
    pub connection_diagnostics: Option<ConnectionDiagnostics>,
    pub last_error: Option<String>,
    pub last_summary: Option<SessionSummary>,
    pub profile: Option<ProfileSnapshot>,
    pub calibration: RecordingCalibration,
    pub digital_output_mask: Option<u8>,
    /// Timestamp of the first accepted recording frame. This is used only as
    /// the live plot's elapsed-time origin; raw timestamps remain unchanged.
    pub display_origin_timestamp_us: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("illegal transition: {0}")]
    State(&'static str),
    #[error("serial: {0}")]
    Serial(#[from] serialport::Error),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("recording: {0}")]
    Recording(#[from] crate::recording::RecordingError),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("storage: {0}")]
    Storage(String),
}

#[derive(Clone)]
pub struct SessionController {
    runtime: Arc<Mutex<SessionRuntime>>,
    cancel: Arc<AtomicBool>,
    worker: Arc<Mutex<Option<JoinHandle<()>>>>,
    stop_reason: Arc<Mutex<Option<StopReason>>>,
    disk_space: Arc<dyn DiskSpaceProvider>,
}

struct SessionRuntime {
    state: SessionState,
    board: String,
    port: String,
    simulator: bool,
    protocol_version: String,
    recent: VecDeque<SynchronizedRecord>,
    samples: u64,
    packets: u64,
    measured_rate_hz: f64,
    integrity: IntegrityCounters,
    last_error: Option<String>,
    last_summary: Option<SessionSummary>,
    duration: Option<RecordingDuration>,
    started_at: Option<Instant>,
    available_disk_bytes: Option<u64>,
    storage_warning: Option<String>,
    stop_reason: Option<StopReason>,
    connection_diagnostics: Option<ConnectionDiagnostics>,
    profile: Option<ProfileSnapshot>,
    calibration: RecordingCalibration,
    markers: Vec<RecordingMarker>,
    digital_output_mask: Option<u8>,
    display_origin_timestamp_us: Option<u64>,
}

/// Immutable inputs for one production capture worker.  Keeping these together
/// avoids a parallel set of positional parameters drifting as recording
/// provenance evolves.
#[derive(Clone)]
struct CaptureRequest {
    simulator: bool,
    source: String,
    profile: ProfileSnapshot,
    duration: RecordingDuration,
    output_dir: PathBuf,
    calibration: RecordingCalibration,
    recording_path_context: Option<RecordingPathContext>,
    /// Identity observed before the worker owns the serial handle. It is later
    /// compared with a read-only Windows enumeration if a serial operation
    /// fails; no reset, reopen, or Arduino CLI scan is performed at that time.
    expected_port_identity: Option<UsbPortIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct UsbPortIdentity {
    port: String,
    vid: u16,
    pid: u16,
    serial_number: Option<String>,
}

#[derive(Clone, Debug)]
struct PortEnumerationDiagnostic {
    selected_port_present: Option<bool>,
    same_vid_pid_present: Option<bool>,
    same_serial_present: Option<bool>,
    uno_r4_present: Option<bool>,
    observed_vid: Option<u16>,
    observed_pid: Option<u16>,
    observed_serial_number: Option<String>,
    enumeration_error: Option<String>,
}

#[derive(Clone, Copy, Debug)]
enum SerialFailureStage {
    SerialRead,
    PingWrite,
    PingFlush,
    FirmwareError,
    NoDataTimeout,
    StopWrite,
    StopFlush,
    StopStatusRead,
}

impl SerialFailureStage {
    fn label(self) -> &'static str {
        match self {
            Self::SerialRead => "SERIAL_READ",
            Self::PingWrite => "PING_WRITE",
            Self::PingFlush => "PING_FLUSH",
            Self::FirmwareError => "FIRMWARE_ERROR",
            Self::NoDataTimeout => "NO_DATA_TIMEOUT",
            Self::StopWrite => "STOP_WRITE",
            Self::StopFlush => "STOP_FLUSH",
            Self::StopStatusRead => "STOP_STATUS_READ",
        }
    }

    fn classification(self) -> &'static str {
        match self {
            Self::SerialRead => "SerialReadError",
            Self::PingWrite | Self::PingFlush => "PingWriteError",
            Self::FirmwareError => "ProtocolFailure",
            Self::NoDataTimeout => "NoDataTimeout",
            Self::StopWrite | Self::StopFlush => "SerialWriteError",
            Self::StopStatusRead => "SerialReadError",
        }
    }
}

#[derive(Debug)]
struct SerialTerminalFailure {
    stage: SerialFailureStage,
    error: std::io::Error,
}

impl SerialTerminalFailure {
    fn new(stage: SerialFailureStage, error: std::io::Error) -> Self {
        Self { stage, error }
    }

    fn detail(&self) -> String {
        format!(
            "{} [{}] kind={:?} raw_os_error={:?}: {}",
            self.stage.label(),
            self.stage.classification(),
            self.error.kind(),
            self.error.raw_os_error(),
            self.error
        )
    }

    fn into_session_error(self) -> SessionError {
        SessionError::Io(self.error)
    }
}

fn firmware_error_detail(code: Option<u8>, payload: Option<&[u8]>) -> String {
    match code {
        Some(7) => "firmware ERROR 7: host-command watchdog expired; acquisition stopped".into(),
        Some(8) => {
            let Some(payload) = payload.filter(|bytes| bytes.len() >= 16) else {
                return "firmware ERROR 8: ADC conversion deadline expired; acquisition entered its safe fault state".into();
            };
            let stage = payload[1];
            let channel = payload[2];
            let fsp_error = payload[3];
            let starts = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
            let completions =
                u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
            let timeouts =
                u32::from_le_bytes([payload[12], payload[13], payload[14], payload[15]]);
            format!(
                "firmware ERROR 8: ADC conversion deadline expired; stage={stage} channel=A{channel} fsp_error={fsp_error} adc_starts={starts} adc_completions={completions} adc_timeouts={timeouts}; acquisition entered its safe fault state"
            )
        }
        Some(9) => "firmware ERROR 9: ADC driver operation failed; acquisition entered its safe fault state".into(),
        Some(code) => format!("firmware ERROR {code}: acquisition stopped"),
        None => "firmware sent ERROR_MESSAGE without an error code".into(),
    }
}

fn no_data_timeout_detail(elapsed: Duration) -> String {
    format!(
        "no synchronized sample batch for {} ms while the recording session remained active",
        elapsed.as_millis()
    )
}

fn sample_stream_stall_failure(
    progress: &CaptureProgress,
    started: Instant,
    now: Instant,
) -> Option<SerialTerminalFailure> {
    let sample_silence = now.duration_since(progress.last_valid_sample.unwrap_or(started));
    (sample_silence >= SAMPLE_STREAM_STALL_TIMEOUT).then(|| {
        SerialTerminalFailure::new(
            SerialFailureStage::NoDataTimeout,
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                no_data_timeout_detail(sample_silence),
            ),
        )
    })
}

/// Windows can report `TimedOut` (including ERROR_SEM_TIMEOUT / 121) or
/// `WouldBlock` from a short CDC write while the same COM handle is still
/// receiving current sample batches.  A single event does not prove that the
/// USB device disappeared, and immediately faulting loses an otherwise sound
/// recording.  We defer only these PING-path errors, only while samples remain
/// current, and only while there is room to send another command before the
/// firmware's five-second watchdog.
fn may_defer_ping_failure(
    failure: &SerialTerminalFailure,
    now: Instant,
    last_confirmed_keepalive: Instant,
    last_valid_sample: Option<Instant>,
    consecutive_failures: u8,
) -> bool {
    let is_ping_transport_timeout = matches!(
        failure.stage,
        SerialFailureStage::PingWrite | SerialFailureStage::PingFlush
    ) && matches!(
        failure.error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    );
    let samples_are_current = last_valid_sample
        .is_some_and(|last_sample| now.duration_since(last_sample) <= PING_RECENT_SAMPLE_GRACE);
    let keepalive_has_margin =
        now.duration_since(last_confirmed_keepalive) < PING_KEEPALIVE_RETRY_WINDOW;

    is_ping_transport_timeout
        && samples_are_current
        && keepalive_has_margin
        && consecutive_failures <= MAX_DEFERRED_PING_FAILURES
}

#[derive(Debug)]
enum CaptureCommandError {
    Protocol(String),
    Serial(SerialTerminalFailure),
}

#[derive(Clone, Debug, Default)]
struct CaptureProgress {
    packets: u64,
    samples: u64,
    pongs: u64,
    statuses: u64,
    last_valid_packet: Option<Instant>,
    last_valid_sample: Option<Instant>,
    last_ping_attempt: Option<Instant>,
    last_successful_ping: Option<Instant>,
    last_ping_write_duration: Option<Duration>,
    last_pong_or_status: Option<Instant>,
    last_recording_flush: Option<Instant>,
    last_recording_flush_duration: Option<Duration>,
    last_disk_check: Option<Instant>,
    last_disk_check_duration: Option<Duration>,
}

impl CaptureProgress {
    fn observe(&mut self, snapshot: &AcquisitionSnapshot, now: Instant) {
        if snapshot.integrity.received_packets > self.packets {
            self.last_valid_packet = Some(now);
            self.packets = snapshot.integrity.received_packets;
        }
        if snapshot.sample_count > self.samples {
            self.last_valid_sample = Some(now);
            self.samples = snapshot.sample_count;
        }
        if snapshot.pong_count > self.pongs || snapshot.status_count > self.statuses {
            self.last_pong_or_status = Some(now);
            self.pongs = snapshot.pong_count;
            self.statuses = snapshot.status_count;
        }
    }
}

struct ActiveSerialFailureContext<'a> {
    port: &'a str,
    expected_identity: Option<&'a UsbPortIdentity>,
    started: Instant,
    started_utc: DateTime<Utc>,
    progress: &'a CaptureProgress,
    snapshot: &'a AcquisitionSnapshot,
}

/// The Project folder and the relative trial folder selected at Start. The
/// BMEG remains portable because its raw samples do not depend on this context.
#[derive(Clone)]
pub struct RecordingPathContext {
    pub project_folder: String,
    pub output_folder: String,
}

/// Inputs used exclusively to create the immutable recording metadata header.
/// Keeping them together prevents recording provenance from growing another
/// positional-argument list as student-facing settings evolve.
struct InitialMetadataRequest<'a> {
    simulator: bool,
    source: &'a str,
    bmeg: &'a Path,
    duration: &'a RecordingDuration,
    profile: &'a ProfileSnapshot,
    calibration: RecordingCalibration,
    recording_path_context: Option<&'a RecordingPathContext>,
}

impl Default for SessionController {
    fn default() -> Self {
        Self::with_disk_space(Arc::new(SystemDiskSpace))
    }
}

impl SessionController {
    fn with_disk_space(disk_space: Arc<dyn DiskSpaceProvider>) -> Self {
        Self {
            runtime: Arc::new(Mutex::new(SessionRuntime {
                state: SessionState::Disconnected,
                board: String::new(),
                port: String::new(),
                simulator: false,
                protocol_version: "0.3".into(),
                recent: VecDeque::with_capacity(DISPLAY_CAPACITY),
                samples: 0,
                packets: 0,
                measured_rate_hz: 0.0,
                integrity: IntegrityCounters::default(),
                last_error: None,
                last_summary: None,
                duration: None,
                started_at: None,
                available_disk_bytes: None,
                storage_warning: None,
                stop_reason: None,
                connection_diagnostics: None,
                profile: None,
                calibration: RecordingCalibration::default(),
                markers: Vec::new(),
                digital_output_mask: None,
                display_origin_timestamp_us: None,
            })),
            cancel: Arc::new(AtomicBool::new(false)),
            worker: Arc::new(Mutex::new(None)),
            stop_reason: Arc::new(Mutex::new(None)),
            disk_space,
        }
    }
    pub fn status(&self) -> Result<SessionStatus, SessionError> {
        let runtime = self.lock_runtime()?;
        Ok(Self::status_from_runtime(&runtime))
    }

    /// Returns the complete bounded display cache for diagnostics and tests.
    /// Callers rendering the live UI should use `recent_display_samples` so a
    /// long window never transfers tens of thousands of frames every poll.
    pub fn recent_samples(&self) -> Result<Vec<SynchronizedRecord>, SessionError> {
        Ok(self.lock_runtime()?.recent.iter().cloned().collect())
    }

    /// Read-only, timestamp-windowed display snapshot. This never changes the
    /// acquisition state, raw writer, integrity counters, or retained records.
    pub fn recent_display_samples(
        &self,
        window_seconds: f64,
        max_points: usize,
    ) -> Result<Vec<SynchronizedRecord>, SessionError> {
        let runtime = self.lock_runtime()?;
        Ok(select_display_records(
            &runtime.recent,
            normalize_display_window_seconds(window_seconds),
            normalize_display_max_points(max_points),
        ))
    }

    pub fn is_recording(&self) -> Result<bool, SessionError> {
        Ok(matches!(
            self.lock_runtime()?.state,
            SessionState::Connecting
                | SessionState::Connected
                | SessionState::Configured
                | SessionState::Acquiring
                | SessionState::Stopping
        ))
    }

    /// Adds a bounded user annotation to the active logical-record stream. It is recorded in the
    /// finalized metadata sidecar/header and never changes the raw BMEG samples.
    pub fn add_marker(&self, label: String) -> Result<RecordingMarker, SessionError> {
        let label = label.trim();
        if label.len() > 80 || label.contains('\0') {
            return Err(SessionError::State(
                "marker label must be at most 80 non-NUL characters",
            ));
        }
        let mut runtime = self.lock_runtime()?;
        if runtime.state != SessionState::Acquiring {
            return Err(SessionError::State(
                "markers can be added only while recording",
            ));
        }
        let timestamp_us = runtime
            .recent
            .back()
            .map(|record| record.timestamp_us)
            .unwrap_or(0);
        let marker = RecordingMarker {
            timestamp_us,
            label: label.into(),
        };
        runtime.markers.push(marker.clone());
        Ok(marker)
    }

    /// Starts the common production path on a worker. This returns quickly; callers poll status.
    pub fn start_simulator(
        &self,
        duration: RecordingDuration,
        output_dir: PathBuf,
    ) -> Result<SessionStatus, SessionError> {
        self.start_simulator_with_profile(default_general_profile()?, duration, output_dir)
    }

    pub fn start_simulator_with_profile(
        &self,
        profile: ProfileSnapshot,
        duration: RecordingDuration,
        output_dir: PathBuf,
    ) -> Result<SessionStatus, SessionError> {
        self.start_simulator_with_profile_and_calibration(
            profile,
            duration,
            output_dir,
            RecordingCalibration::default(),
        )
    }

    /// Starts a profile recording with a frozen, display/export-only calibration snapshot.
    pub fn start_simulator_with_profile_and_calibration(
        &self,
        profile: ProfileSnapshot,
        duration: RecordingDuration,
        output_dir: PathBuf,
        calibration: RecordingCalibration,
    ) -> Result<SessionStatus, SessionError> {
        self.start_simulator_with_profile_calibration_and_path_context(
            profile,
            duration,
            output_dir,
            calibration,
            None,
        )
    }

    pub fn start_simulator_with_profile_calibration_and_path_context(
        &self,
        profile: ProfileSnapshot,
        duration: RecordingDuration,
        output_dir: PathBuf,
        calibration: RecordingCalibration,
        recording_path_context: Option<RecordingPathContext>,
    ) -> Result<SessionStatus, SessionError> {
        self.validate_duration(&duration)?;
        calibration
            .validate()
            .map_err(|error| SessionError::Protocol(error.to_string()))?;
        self.begin_session(
            true,
            "Simulator",
            "SIM",
            duration.clone(),
            profile.clone(),
            calibration.clone(),
        )?;
        let controller = self.clone();
        self.spawn_worker(move || {
            controller.capture_simulator_worker(
                profile,
                duration,
                output_dir,
                calibration,
                recording_path_context,
            )
        })?;
        self.status()
    }

    /// Starts the common production path with a serial transport on a worker.
    pub fn start_serial(
        &self,
        port_name: String,
        duration: RecordingDuration,
        output_dir: PathBuf,
    ) -> Result<SessionStatus, SessionError> {
        self.start_serial_with_profile(default_general_profile()?, port_name, duration, output_dir)
    }

    pub fn start_serial_with_profile(
        &self,
        profile: ProfileSnapshot,
        port_name: String,
        duration: RecordingDuration,
        output_dir: PathBuf,
    ) -> Result<SessionStatus, SessionError> {
        self.start_serial_with_profile_and_calibration(
            profile,
            port_name,
            duration,
            output_dir,
            RecordingCalibration::default(),
        )
    }

    pub fn start_serial_with_profile_and_calibration(
        &self,
        profile: ProfileSnapshot,
        port_name: String,
        duration: RecordingDuration,
        output_dir: PathBuf,
        calibration: RecordingCalibration,
    ) -> Result<SessionStatus, SessionError> {
        self.start_serial_with_profile_calibration_and_path_context(
            profile,
            port_name,
            duration,
            output_dir,
            calibration,
            None,
        )
    }

    pub fn start_serial_with_profile_calibration_and_path_context(
        &self,
        profile: ProfileSnapshot,
        port_name: String,
        duration: RecordingDuration,
        output_dir: PathBuf,
        calibration: RecordingCalibration,
        recording_path_context: Option<RecordingPathContext>,
    ) -> Result<SessionStatus, SessionError> {
        self.validate_duration(&duration)?;
        calibration
            .validate()
            .map_err(|error| SessionError::Protocol(error.to_string()))?;
        self.begin_session(
            false,
            "Arduino UNO R4 WiFi",
            &port_name,
            duration.clone(),
            profile.clone(),
            calibration.clone(),
        )?;
        let controller = self.clone();
        self.spawn_worker(move || {
            controller.capture_serial_worker(
                profile,
                port_name,
                duration,
                output_dir,
                calibration,
                recording_path_context,
            )
        })?;
        self.status()
    }

    /// Synchronous entry point used by the controlled acceptance harness and tests.
    pub fn capture_simulator(
        &self,
        duration: RecordingDuration,
        output_dir: &Path,
    ) -> Result<SessionSummary, SessionError> {
        self.capture_simulator_with_profile(default_general_profile()?, duration, output_dir)
    }

    pub fn capture_simulator_with_profile(
        &self,
        profile: ProfileSnapshot,
        duration: RecordingDuration,
        output_dir: &Path,
    ) -> Result<SessionSummary, SessionError> {
        self.validate_duration(&duration)?;
        self.begin_session(
            true,
            "Simulator",
            "SIM",
            duration.clone(),
            profile.clone(),
            RecordingCalibration::default(),
        )?;
        self.capture_simulator_worker(
            profile,
            duration,
            output_dir.to_path_buf(),
            RecordingCalibration::default(),
            None,
        )?;
        self.status()?.last_summary.ok_or(SessionError::State(
            "simulator session did not produce a summary",
        ))
    }

    #[cfg(test)]
    fn capture_simulator_for_test(
        &self,
        seconds: u64,
        output_dir: &Path,
    ) -> Result<SessionSummary, SessionError> {
        let duration = RecordingDuration::Timed { seconds };
        self.begin_session(
            true,
            "Simulator",
            "SIM",
            duration.clone(),
            default_general_profile()?,
            RecordingCalibration::default(),
        )?;
        self.capture_simulator_worker(
            default_general_profile()?,
            duration,
            output_dir.to_path_buf(),
            RecordingCalibration::default(),
            None,
        )?;
        self.status()?.last_summary.ok_or(SessionError::State(
            "simulator session did not produce a summary",
        ))
    }

    /// Synchronous serial entry point used by the controlled acceptance harness.
    pub fn capture_serial(
        &self,
        port_name: &str,
        duration: RecordingDuration,
        output_dir: &Path,
    ) -> Result<SessionSummary, SessionError> {
        self.capture_serial_with_profile(
            default_general_profile()?,
            port_name,
            duration,
            output_dir,
        )
    }

    pub fn capture_serial_with_profile(
        &self,
        profile: ProfileSnapshot,
        port_name: &str,
        duration: RecordingDuration,
        output_dir: &Path,
    ) -> Result<SessionSummary, SessionError> {
        self.validate_duration(&duration)?;
        self.begin_session(
            false,
            "Arduino UNO R4 WiFi",
            port_name,
            duration.clone(),
            profile.clone(),
            RecordingCalibration::default(),
        )?;
        self.capture_serial_worker(
            profile,
            port_name.to_owned(),
            duration,
            output_dir.to_path_buf(),
            RecordingCalibration::default(),
            None,
        )?;
        self.status()?.last_summary.ok_or(SessionError::State(
            "serial session did not produce a summary",
        ))
    }

    /// Idempotent: it asks a running worker to finish the current recording.
    pub fn request_stop(&self) -> Result<SessionStatus, SessionError> {
        self.request_stop_with_reason(StopReason::User)
    }

    /// Idempotently asks the worker to stop and records the first terminal
    /// reason.  A racing manual stop cannot overwrite timed completion.
    pub fn request_stop_with_reason(
        &self,
        reason: StopReason,
    ) -> Result<SessionStatus, SessionError> {
        {
            let mut runtime = self.lock_runtime()?;
            if matches!(
                runtime.state,
                SessionState::Acquiring | SessionState::Configured | SessionState::Connecting
            ) {
                runtime.state = SessionState::Stopping;
            }
        }
        self.set_stop_reason_once(reason)?;
        self.cancel.store(true, Ordering::Release);
        self.status()
    }

    /// Idempotent disconnect. The worker is joined without holding the runtime mutex.
    pub fn disconnect(&self) -> Result<SessionStatus, SessionError> {
        self.request_stop_with_reason(StopReason::User)?;
        let handle = self
            .worker
            .lock()
            .map_err(|_| SessionError::State("worker lock poisoned"))?
            .take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
        let mut runtime = self.lock_runtime()?;
        runtime.state = SessionState::Disconnected;
        Ok(Self::status_from_runtime(&runtime))
    }

    /// Clears an idle terminal fault before a new, explicitly requested recording.
    ///
    /// A failed handshake intentionally retains `Faulted` diagnostics for recovery,
    /// but it owns no serial transport. Once a later verification has succeeded (or
    /// a fresh start has passed all of its checks), that historical fault must not
    /// prevent a new session. Active sessions remain protected and are never reset.
    pub fn prepare_for_new_recording(&self) -> Result<SessionStatus, SessionError> {
        if self.is_recording()? {
            return Err(SessionError::State(
                "cannot begin a new session while a recording or connection is active",
            ));
        }
        self.wait_for_worker()?;
        let mut runtime = self.lock_runtime()?;
        runtime.state = SessionState::Disconnected;
        runtime.last_error = None;
        runtime.stop_reason = None;
        runtime.connection_diagnostics = None;
        self.cancel.store(false, Ordering::Release);
        *self
            .stop_reason
            .lock()
            .map_err(|_| SessionError::State("stop reason lock poisoned"))? = None;
        Ok(Self::status_from_runtime(&runtime))
    }

    pub fn wait_for_worker(&self) -> Result<(), SessionError> {
        let handle = self
            .worker
            .lock()
            .map_err(|_| SessionError::State("worker lock poisoned"))?
            .take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
        Ok(())
    }

    /// User-initiated handshake retry that does not reset or upload the board.
    /// It is useful when the serial port was briefly busy but the device is healthy.
    pub fn retry_handshake(
        &self,
        target: ResetTarget,
    ) -> Result<HandshakeRetryResult, SessionError> {
        if self.is_recording()? {
            return Err(SessionError::State(
                "cannot retry a handshake while a recording or connection is active",
            ));
        }
        self.wait_for_worker()?;
        self.set_state(SessionState::Connecting)?;
        self.cancel.store(false, Ordering::Release);
        self.set_reset_diagnostics(ConnectionDiagnostics::new(&target.port))?;

        let result = (|| {
            let mut port = serialport::new(&target.port, CONTROLLED_SERIAL_BAUD)
                .timeout(Duration::from_millis(25))
                .open()?;
            self.mark_port_opened(&target.port)?;
            port.clear(serialport::ClearBuffer::Input)?;
            port.write_data_terminal_ready(true)?;
            port.write_request_to_send(true)?;
            self.set_state(SessionState::Connected)?;
            let (sender, _receiver) = sync_channel(16);
            let mut acquisition = AcquisitionController::new(sender);
            self.wait_for_handshake(&mut port, &mut acquisition)
        })();
        let diagnostics = self
            .status()?
            .connection_diagnostics
            .unwrap_or_else(|| ConnectionDiagnostics::new(&target.port));
        match result {
            Ok(()) => {
                self.set_state(SessionState::Disconnected)?;
                Ok(HandshakeRetryResult {
                    handshake_succeeded: true,
                    diagnostics,
                })
            }
            Err(error) => {
                self.set_fault(error.to_string())?;
                Ok(HandshakeRetryResult {
                    handshake_succeeded: false,
                    diagnostics,
                })
            }
        }
    }

    /// User-initiated recovery only. This never uploads firmware and is rejected
    /// while any acquisition state owns a recording or serial transport.
    pub fn reset_and_retry(&self, target: ResetTarget) -> Result<ResetRetryResult, SessionError> {
        if self.is_recording()? {
            return Err(SessionError::State(
                "cannot reset a board while a recording or connection is active",
            ));
        }
        self.wait_for_worker()?;
        self.set_state(SessionState::Connecting)?;
        self.cancel.store(false, Ordering::Release);
        let mut diagnostics = ConnectionDiagnostics::new(&target.port);
        diagnostics.reset_attempted = true;
        diagnostics.original_port = Some(target.port.clone());
        diagnostics.recommended_action = "Touching the selected UNO R4 WiFi at 1200 bps.".into();
        self.set_reset_diagnostics(diagnostics.clone())?;

        // Opening then immediately dropping a 1200-bps CDC handle is the explicit,
        // user-approved reset mechanism observed on the UNO R4 WiFi. It is never
        // attempted as part of a normal connection and is restricted to a discovered target.
        // The observed manual recovery uses .NET SerialPort's defaults, which leave
        // DTR and RTS deasserted for the 1200-bps open/close. Be explicit here: the
        // serialport crate otherwise preserves Windows control-line state on open.
        match serialport::new(&target.port, 1_200)
            .timeout(Duration::from_millis(250))
            .dtr_on_open(false)
            .open()
        {
            Ok(mut touch_port) => {
                let _ = touch_port.write_data_terminal_ready(false);
                let _ = touch_port.write_request_to_send(false);
                thread::sleep(Duration::from_millis(50));
                drop(touch_port);
            }
            Err(error) => {
                diagnostics.failure_category = Some(ConnectionFailureCategory::PortBusy);
                diagnostics.recommended_action =
                    recommended_action(&ConnectionFailureCategory::PortBusy).into();
                self.set_reset_diagnostics(diagnostics)?;
                self.set_fault(error.to_string())?;
                return Err(SessionError::Serial(error));
            }
        }

        let reset_started = Instant::now();
        let mut final_port = None;
        let mut ambiguity = false;
        while reset_started.elapsed() < RESET_ENUMERATION_TIMEOUT {
            if self.cancel.load(Ordering::Acquire) {
                return Err(SessionError::State("board reset was cancelled"));
            }
            // Arduino CLI discovery is comparatively slow on Windows. Poll the
            // operating-system USB serial list here so the short bootloader window
            // is observable; initial target selection still comes from Arduino CLI.
            let ports = enumerate_uno_usb_ports();
            let original_present = ports.iter().any(|port| target_matches_port(&target, port));
            diagnostics.disappearance_observed |= !original_present;
            diagnostics.bootloader_observed |= ports.iter().any(|port| {
                port.role == UnoUsbRole::Bootloader
                    && target
                        .serial_number
                        .as_ref()
                        .is_none_or(|serial| port.serial_number.as_ref() == Some(serial))
            });
            match select_returning_port(&target, &ports) {
                ReturningPort::Found(port)
                    if reset_started.elapsed() >= RESET_ENUMERATION_SETTLE =>
                {
                    final_port = Some(port.port);
                    break;
                }
                ReturningPort::Ambiguous => ambiguity = true,
                ReturningPort::Absent | ReturningPort::Found(_) => {}
            }
            thread::sleep(RESET_ENUMERATION_POLL);
        }

        let Some(final_port) = final_port else {
            let category = if ambiguity {
                ConnectionFailureCategory::AmbiguousReturningDevice
            } else {
                ConnectionFailureCategory::ResetPortDidNotReturn
            };
            diagnostics.failure_category = Some(category.clone());
            diagnostics.handshake_elapsed_ms = reset_started.elapsed().as_millis();
            diagnostics.recommended_action = recommended_action(&category).into();
            self.set_reset_diagnostics(diagnostics.clone())?;
            self.set_fault(format!("{category:?}"))?;
            return Ok(ResetRetryResult {
                original_port: target.port,
                final_port: None,
                handshake_succeeded: false,
                diagnostics,
            });
        };

        diagnostics.final_port = Some(final_port.clone());
        diagnostics.reappearance_observed = true;
        diagnostics.selected_port = final_port.clone();
        diagnostics.recommended_action = if final_port.eq_ignore_ascii_case(&target.port) {
            "Board re-enumerated. Retrying normal protocol handshake.".into()
        } else {
            "Board returned on a different COM port. Retrying normal protocol handshake.".into()
        };
        self.set_reset_diagnostics(diagnostics.clone())?;

        let result = (|| {
            // UNO R4 bootloader activity can leave the USB bridge's COM number
            // unchanged. Give its application firmware a bounded settling period
            // even when no disappearance was observable.
            thread::sleep(RESET_APPLICATION_GRACE);
            let mut port = serialport::new(&final_port, CONTROLLED_SERIAL_BAUD)
                .timeout(Duration::from_millis(25))
                .open()?;
            self.mark_port_opened(&final_port)?;
            port.clear(serialport::ClearBuffer::Input)?;
            port.write_data_terminal_ready(true)?;
            port.write_request_to_send(true)?;
            self.set_state(SessionState::Connected)?;
            let (sender, _receiver) = sync_channel(16);
            let mut acquisition = AcquisitionController::new(sender);
            self.wait_for_handshake_with_policy(
                &mut port,
                &mut acquisition,
                HANDSHAKE_STARTUP_GRACE,
                RESET_HANDSHAKE_TIMEOUT,
                HANDSHAKE_PING_ATTEMPTS,
                HANDSHAKE_PING_INTERVAL,
            )
        })();
        let status = self.status()?;
        let diagnostics = status.connection_diagnostics.unwrap_or(diagnostics);
        match result {
            Ok(()) => {
                self.set_state(SessionState::Disconnected)?;
                Ok(ResetRetryResult {
                    original_port: target.port,
                    final_port: Some(final_port),
                    handshake_succeeded: true,
                    diagnostics,
                })
            }
            Err(error) => {
                self.set_fault(error.to_string())?;
                Ok(ResetRetryResult {
                    original_port: target.port,
                    final_port: Some(final_port),
                    handshake_succeeded: false,
                    diagnostics,
                })
            }
        }
    }

    fn validate_duration(&self, duration: &RecordingDuration) -> Result<(), SessionError> {
        duration.validate().map_err(SessionError::State)
    }

    fn begin_session(
        &self,
        simulator: bool,
        board: &str,
        port: &str,
        duration: RecordingDuration,
        profile: ProfileSnapshot,
        calibration: RecordingCalibration,
    ) -> Result<(), SessionError> {
        validate_profile_snapshot(&profile)?;
        calibration
            .validate()
            .map_err(|error| SessionError::Protocol(error.to_string()))?;
        let mut runtime = self.lock_runtime()?;
        if runtime.state != SessionState::Disconnected {
            return Err(SessionError::State(
                "only one session may be active; disconnect first",
            ));
        }
        runtime.state = SessionState::Connecting;
        runtime.board = board.to_owned();
        runtime.port = port.to_owned();
        runtime.simulator = simulator;
        runtime.protocol_version = "0.3".into();
        runtime.recent.clear();
        runtime.samples = 0;
        runtime.packets = 0;
        runtime.measured_rate_hz = 0.0;
        runtime.integrity = IntegrityCounters::default();
        runtime.last_error = None;
        runtime.last_summary = None;
        runtime.duration = Some(duration);
        runtime.started_at = Some(Instant::now());
        runtime.available_disk_bytes = None;
        runtime.storage_warning = None;
        runtime.stop_reason = None;
        runtime.connection_diagnostics = if simulator {
            None
        } else {
            Some(ConnectionDiagnostics::new(port))
        };
        runtime.profile = Some(profile);
        runtime.calibration = calibration;
        runtime.markers.clear();
        runtime.display_origin_timestamp_us = None;
        self.cancel.store(false, Ordering::Release);
        let mut stop_reason = self
            .stop_reason
            .lock()
            .map_err(|_| SessionError::State("stop reason lock poisoned"))?;
        *stop_reason = None;
        Ok(())
    }

    fn spawn_worker<F>(&self, capture: F) -> Result<(), SessionError>
    where
        F: FnOnce() -> Result<(), SessionError> + Send + 'static,
    {
        let mut worker = self
            .worker
            .lock()
            .map_err(|_| SessionError::State("worker lock poisoned"))?;
        if worker.as_ref().is_some_and(|handle| !handle.is_finished()) {
            return Err(SessionError::State("worker is already running"));
        }
        if let Some(previous) = worker.take() {
            let _ = previous.join();
        }
        *worker = Some(thread::spawn(move || {
            let _ = capture();
        }));
        Ok(())
    }

    fn capture_simulator_worker(
        &self,
        profile: ProfileSnapshot,
        duration: RecordingDuration,
        output_dir: PathBuf,
        calibration: RecordingCalibration,
        recording_path_context: Option<RecordingPathContext>,
    ) -> Result<(), SessionError> {
        let result = (|| {
            let mut simulator = SimulatorIo::new(&profile, duration.clone())?;
            self.set_state(SessionState::Connected)?;
            self.capture_transport(
                &mut simulator,
                CaptureRequest {
                    simulator: true,
                    source: "SIM".into(),
                    profile,
                    duration,
                    output_dir,
                    calibration,
                    recording_path_context,
                    expected_port_identity: None,
                },
            )
        })();
        self.finish_worker_error(&result)?;
        result
    }

    fn capture_serial_worker(
        &self,
        profile: ProfileSnapshot,
        port_name: String,
        duration: RecordingDuration,
        output_dir: PathBuf,
        calibration: RecordingCalibration,
        recording_path_context: Option<RecordingPathContext>,
    ) -> Result<(), SessionError> {
        let result = (|| {
            // This is read-only Windows serial-port enumeration. The captured
            // identity is used only if a later active-capture error occurs.
            let expected_port_identity = usb_port_identity(&port_name);
            crate::app_log::record("INFO", &format!("SERIAL_OPEN_BEGIN port={port_name}"));
            let mut port = serialport::new(&port_name, CONTROLLED_SERIAL_BAUD)
                .timeout(ACTIVE_SERIAL_IO_TIMEOUT)
                .open()
                .map_err(|error| {
                    crate::app_log::record(
                        "WARN",
                        &format!("START_FAIL stage=SERIAL_OPEN detail={error}"),
                    );
                    SessionError::Serial(error)
                })?;
            self.mark_port_opened(&port_name)?;
            crate::app_log::record(
                "INFO",
                &format!(
                    "SERIAL_OPEN_OK port={port_name} io_timeout_ms={}",
                    ACTIVE_SERIAL_IO_TIMEOUT.as_millis()
                ),
            );
            // Clear stale bytes before asserting host control lines. The firmware then emits HELLO.
            port.clear(serialport::ClearBuffer::Input)?;
            port.write_data_terminal_ready(true)?;
            port.write_request_to_send(true)?;
            // wait_for_handshake passively listens through the bounded startup grace
            // before its first PING, so a healthy board can announce itself first.
            self.set_state(SessionState::Connected)?;
            self.capture_transport(
                &mut port,
                CaptureRequest {
                    simulator: false,
                    source: port_name,
                    profile,
                    duration,
                    output_dir,
                    calibration,
                    recording_path_context,
                    expected_port_identity,
                },
            )
        })();
        self.finish_worker_error(&result)?;
        result
    }

    fn capture_transport<T: Read + Write>(
        &self,
        io: &mut T,
        request: CaptureRequest,
    ) -> Result<(), SessionError> {
        let CaptureRequest {
            simulator,
            source,
            profile,
            duration,
            output_dir,
            calibration,
            recording_path_context,
            expected_port_identity,
        } = request;
        // Collect tooling provenance before START so no external command can delay raw intake.
        crate::app_log::record("INFO", "OPEN_RECORDING_FILE_BEGIN");
        let (temporary_bmeg, bmeg, csv, metadata) = self
            .allocate_paths(&output_dir, &profile)
            .map_err(|error| {
                crate::app_log::record(
                    "WARN",
                    &format!("START_FAIL stage=OPEN_RECORDING_FILE detail={error}"),
                );
                error
            })?;
        crate::app_log::record("INFO", "OPEN_RECORDING_FILE_OK");
        let initial_free_disk_bytes = self.free_disk_space(&output_dir)?;
        self.update_disk_space(initial_free_disk_bytes)?;
        if initial_free_disk_bytes < DISK_CRITICAL_BYTES {
            return Err(SessionError::Storage(format!(
                "only {} MiB free in {}; recording requires at least {} MiB",
                initial_free_disk_bytes / (1024 * 1024),
                output_dir.display(),
                DISK_CRITICAL_BYTES / (1024 * 1024)
            )));
        }
        let mut initial_meta = self
            .initial_metadata(InitialMetadataRequest {
                simulator,
                source: &source,
                bmeg: &bmeg,
                duration: &duration,
                profile: &profile,
                calibration,
                recording_path_context: recording_path_context.as_ref(),
            })
            .map_err(|error| {
                crate::app_log::record(
                    "WARN",
                    &format!("START_FAIL stage=PREPARE_METADATA detail={error}"),
                );
                error
            })?;
        initial_meta.initial_free_disk_bytes = Some(initial_free_disk_bytes);
        let (tx, rx) = sync_channel(4_096);
        let mut acquisition = AcquisitionController::new(tx);
        crate::app_log::record("INFO", "HANDSHAKE_BEGIN");
        self.wait_for_handshake(io, &mut acquisition)
            .map_err(|error| {
                crate::app_log::record(
                    "WARN",
                    &format!("START_FAIL stage=HANDSHAKE detail={error}"),
                );
                error
            })?;
        crate::app_log::record("INFO", "HANDSHAKE_OK");
        let negotiated_capabilities = acquisition.snapshot().firmware_capabilities;
        crate::app_log::record("INFO", "CONFIGURE_SEND");
        self.send_command(
            io,
            MessageType::Configure,
            1,
            configure_payload(
                &profile.profile.acquisition,
                negotiated_capabilities.as_ref(),
            )?,
        )
        .map_err(|error| {
            crate::app_log::record(
                "WARN",
                &format!("START_FAIL stage=CONFIGURE_SEND detail={error}"),
            );
            error
        })?;
        self.wait_until(io, &mut acquisition, |s| s.config_ack_seen, "CONFIG_ACK")
            .map_err(|error| {
                crate::app_log::record(
                    "WARN",
                    &format!("START_FAIL stage=CONFIG_ACK detail={error}"),
                );
                error
            })?;
        crate::app_log::record("INFO", "CONFIG_ACK_RECEIVED");
        acquisition.configure().map_err(SessionError::State)?;
        self.set_state(SessionState::Configured)?;
        // The recording is open before START, so every validated post-start sample has a sink.
        let mut raw = BmegWriter::create_synchronized(
            &temporary_bmeg,
            &initial_meta,
            profile.profile.acquisition.record_field_names().len(),
        )
        .map_err(|error| {
            crate::app_log::record(
                "WARN",
                &format!("START_FAIL stage=OPEN_RECORDING_FILE detail={error}"),
            );
            error
        })?;
        acquisition.start().map_err(SessionError::State)?;
        crate::app_log::record("INFO", "START_SEND");
        self.send_command(io, MessageType::Start, 2, vec![])
            .map_err(|error| {
                crate::app_log::record(
                    "WARN",
                    &format!("START_FAIL stage=START_SEND detail={error}"),
                );
                error
            })?;
        self.wait_until(io, &mut acquisition, |s| s.status_seen, "START status")
            .map_err(|error| {
                crate::app_log::record(
                    "WARN",
                    &format!("START_FAIL stage=START_ACK_OR_FIRST_SAMPLE detail={error}"),
                );
                error
            })?;
        self.set_state(SessionState::Acquiring)?;
        crate::app_log::record("INFO", "RECORDING_ACTIVE");
        let active_digital_output_mask = acquisition.snapshot().digital_output_mask;

        let started = Instant::now();
        let started_utc = Utc::now();
        crate::app_log::record(
            "INFO",
            &format!(
                "RECORDING_START duration_mode={} requested_seconds={:?} source={} lab={} port={} output_dir={} initial_free_disk_bytes={}",
                duration.label(),
                duration.requested_seconds(),
                if simulator { "simulator" } else { "hardware" },
                profile.profile.display_name,
                source,
                output_dir.display(),
                initial_free_disk_bytes,
            ),
        );
        // `START` was accepted before this loop, so it is a valid initial
        // firmware-watchdog keepalive even though it is not a PING response.
        let mut last_ping = Instant::now();
        let mut last_confirmed_keepalive = started;
        let mut consecutive_deferred_ping_failures = 0u8;
        let mut last_disk_check = Instant::now();
        let mut last_flush = Instant::now();
        let mut buffer = [0u8; 512];
        let mut terminal_error: Option<SerialTerminalFailure> = None;
        let mut terminal_failure_category: Option<ConnectionFailureCategory> = None;
        let mut stop_reason = None;
        let mut progress = CaptureProgress::default();
        progress.observe(&acquisition.snapshot(), started);

        loop {
            if self.cancel.load(Ordering::Acquire) {
                stop_reason = Some(self.stop_reason()?.unwrap_or(StopReason::User));
                break;
            }
            if let RecordingDuration::Timed { seconds } = &duration {
                if started.elapsed() >= Duration::from_secs(*seconds) {
                    stop_reason = Some(self.set_stop_reason_once(StopReason::TimedComplete)?);
                    break;
                }
            }
            match io.read(&mut buffer) {
                Ok(n) if n > 0 => {
                    acquisition.ingest_bytes(&buffer[..n]);
                    progress.observe(&acquisition.snapshot(), Instant::now());
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => {
                    terminal_error = Some(SerialTerminalFailure::new(
                        SerialFailureStage::SerialRead,
                        error,
                    ));
                    break;
                }
            }
            self.drain_samples(&rx, &mut raw)?;
            self.update_from_acquisition(&acquisition)?;
            let active_snapshot = acquisition.snapshot();
            let observed_now = Instant::now();
            progress.observe(&active_snapshot, observed_now);
            if active_snapshot.firmware_error_count > 0 {
                terminal_error = Some(SerialTerminalFailure::new(
                    SerialFailureStage::FirmwareError,
                    std::io::Error::other(firmware_error_detail(
                        active_snapshot.firmware_error_code,
                        active_snapshot.firmware_error_payload.as_deref(),
                    )),
                ));
                break;
            }
            if let Some(stall) = sample_stream_stall_failure(&progress, started, observed_now) {
                terminal_error = Some(stall);
                break;
            }
            if last_ping.elapsed() >= ACTIVE_PING_INTERVAL {
                let ping_started = Instant::now();
                progress.last_ping_attempt = Some(ping_started);
                match self.send_capture_command(io, MessageType::Ping, 3, vec![]) {
                    Ok(()) => {
                        let now = Instant::now();
                        last_ping = now;
                        last_confirmed_keepalive = now;
                        consecutive_deferred_ping_failures = 0;
                        progress.last_successful_ping = Some(now);
                        progress.last_ping_write_duration = Some(now.duration_since(ping_started));
                    }
                    Err(CaptureCommandError::Serial(error)) => {
                        let now = Instant::now();
                        last_ping = now;
                        consecutive_deferred_ping_failures =
                            consecutive_deferred_ping_failures.saturating_add(1);
                        progress.last_ping_write_duration = Some(now.duration_since(ping_started));

                        if may_defer_ping_failure(
                            &error,
                            now,
                            last_confirmed_keepalive,
                            progress.last_valid_sample,
                            consecutive_deferred_ping_failures,
                        ) {
                            let sample_age_ms = progress
                                .last_valid_sample
                                .map(|last_sample| now.duration_since(last_sample).as_millis())
                                .unwrap_or(u128::MAX);
                            crate::app_log::record(
                                "WARN",
                                &format!(
                                    "PING_TRANSIENT_WRITE_FAILURE stage={} classification={} error_kind={:?} raw_os_error={:?} detail={} deferred_attempt={} sample_age_ms={} keepalive_age_ms={} next_ping_ms={}",
                                    error.stage.label(),
                                    error.stage.classification(),
                                    error.error.kind(),
                                    error.error.raw_os_error(),
                                    error.error,
                                    consecutive_deferred_ping_failures,
                                    sample_age_ms,
                                    now.duration_since(last_confirmed_keepalive).as_millis(),
                                    ACTIVE_PING_INTERVAL.as_millis(),
                                ),
                            );
                        } else {
                            crate::app_log::record(
                                "ERROR",
                                &format!(
                                    "PING_WRITE_FAILURE_TERMINAL stage={} error_kind={:?} raw_os_error={:?} deferred_attempts={} sample_age_ms={} keepalive_age_ms={}",
                                    error.stage.label(),
                                    error.error.kind(),
                                    error.error.raw_os_error(),
                                    consecutive_deferred_ping_failures,
                                    progress
                                        .last_valid_sample
                                        .map(|last_sample| now.duration_since(last_sample).as_millis())
                                        .unwrap_or(u128::MAX),
                                    now.duration_since(last_confirmed_keepalive).as_millis(),
                                ),
                            );
                            terminal_error = Some(error);
                            break;
                        }
                    }
                    Err(CaptureCommandError::Protocol(detail)) => {
                        terminal_error = Some(SerialTerminalFailure::new(
                            SerialFailureStage::PingWrite,
                            std::io::Error::other(format!(
                                "protocol while encoding PING: {detail}"
                            )),
                        ));
                        break;
                    }
                }
            }
            if last_flush.elapsed() >= RECORDING_FLUSH_INTERVAL {
                let flush_started = Instant::now();
                raw.flush()?;
                let now = Instant::now();
                last_flush = now;
                progress.last_recording_flush = Some(now);
                progress.last_recording_flush_duration = Some(now.duration_since(flush_started));
            }
            if last_disk_check.elapsed() >= DISK_CHECK_INTERVAL {
                let disk_check_started = Instant::now();
                let free = self.free_disk_space(&output_dir)?;
                let now = Instant::now();
                self.update_disk_space(free)?;
                if free < DISK_CRITICAL_BYTES {
                    stop_reason = Some(self.set_stop_reason_once(StopReason::StorageGuard)?);
                    break;
                }
                last_disk_check = now;
                progress.last_disk_check = Some(now);
                progress.last_disk_check_duration = Some(now.duration_since(disk_check_started));
            }
        }

        if let Some(error) = terminal_error.as_ref() {
            // This reads only the operating system's port list. It does not
            // reopen, reset, upload to, or otherwise touch the selected board.
            terminal_failure_category = Some(self.record_active_serial_failure(
                error,
                ActiveSerialFailureContext {
                    port: &source,
                    expected_identity: expected_port_identity.as_ref(),
                    started,
                    started_utc,
                    progress: &progress,
                    snapshot: &acquisition.snapshot(),
                },
            )?);
        }

        match self.send_capture_command(io, MessageType::Stop, 4, vec![]) {
            Ok(()) => {}
            Err(CaptureCommandError::Serial(error)) => {
                self.record_serial_cleanup_failure(&error, &source, started, started_utc, &progress)
            }
            Err(CaptureCommandError::Protocol(detail)) => crate::app_log::record(
                "WARN",
                &format!("SERIAL_CLEANUP_ERROR stage=STOP_PROTOCOL detail={detail}"),
            ),
        }
        // Give the controlled firmware a bounded opportunity to confirm that all
        // course LED outputs are low before the serial handle is released.
        let stop_status_deadline = Instant::now() + Duration::from_millis(300);
        while terminal_error.is_none() && Instant::now() < stop_status_deadline {
            match io.read(&mut buffer) {
                Ok(n) if n > 0 => {
                    acquisition.ingest_bytes(&buffer[..n]);
                    progress.observe(&acquisition.snapshot(), Instant::now());
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => {
                    // A failure after an intentional STOP cannot change the
                    // completed capture into a device-removal result. Keep it
                    // in diagnostics, but preserve the original stop reason.
                    self.record_serial_cleanup_failure(
                        &SerialTerminalFailure::new(SerialFailureStage::StopStatusRead, error),
                        &source,
                        started,
                        started_utc,
                        &progress,
                    );
                    break;
                }
            }
            if acquisition.snapshot().digital_output_mask == Some(0) {
                break;
            }
        }
        self.drain_samples(&rx, &mut raw)?;
        raw.finish()?;
        fs::rename(&temporary_bmeg, &bmeg)?;
        let csv_rows = export_bmeg_csv(&bmeg, &csv)?;
        let mut snapshot = acquisition.snapshot();
        let final_digital_output_mask = snapshot.digital_output_mask;
        let terminal_error_detail = terminal_error.as_ref().map(SerialTerminalFailure::detail);
        if terminal_error_detail.is_some()
            && matches!(
                terminal_failure_category,
                Some(ConnectionFailureCategory::DeviceDisconnected)
            )
        {
            snapshot.integrity.disconnect_events += 1;
        }
        self.update_snapshot(&snapshot)?;
        let reason = if terminal_error_detail.is_some() {
            if matches!(
                terminal_failure_category,
                Some(ConnectionFailureCategory::DeviceDisconnected)
            ) {
                StopReason::Disconnect
            } else {
                StopReason::Fault
            }
        } else {
            stop_reason.unwrap_or(StopReason::Fault)
        };
        let host_elapsed = started.elapsed().as_secs_f64();
        let board_elapsed = board_elapsed_seconds(&snapshot);
        let recording_status = reason.recording_status();
        let final_free_disk_bytes = self.free_disk_space(&output_dir).ok();
        if let Some(free) = final_free_disk_bytes {
            self.update_disk_space(free)?;
        }
        let terminal_trigger = if terminal_error_detail.is_some() {
            "serial_error"
        } else {
            match reason {
                StopReason::User => "user_stop",
                StopReason::TimedComplete => "timed_complete",
                StopReason::Disconnect => "disconnect_request",
                StopReason::StorageGuard => "storage_guard",
                StopReason::ApplicationClose => "application_close",
                StopReason::Fault => "fault",
            }
        };
        crate::app_log::record(
            if terminal_error_detail.is_some() { "ERROR" } else { "INFO" },
            &format!(
                "RECORDING_TERMINAL trigger={} stop_reason={:?} cancel={} terminal_error={} elapsed_ms={} port={} samples={} packets={} free_disk_bytes={:?}",
                terminal_trigger,
                reason,
                self.cancel.load(Ordering::Acquire),
                terminal_error_detail.as_deref().unwrap_or("none"),
                (host_elapsed * 1_000.0).round() as u128,
                source,
                snapshot.sample_count,
                snapshot.integrity.received_packets,
                final_free_disk_bytes,
            ),
        );
        let mut final_meta = initial_meta;
        final_meta.utc_stop = Some(Utc::now());
        final_meta.local_stop = Some(Local::now());
        final_meta.host_elapsed_seconds = Some(host_elapsed);
        final_meta.board_elapsed_seconds = Some(board_elapsed);
        final_meta.measured_sample_rate_hz = snapshot.measured_rate_hz;
        final_meta.total_samples = snapshot.sample_count;
        final_meta.integrity = snapshot.integrity.clone();
        final_meta.recording_status = recording_status.into();
        final_meta.csv_filename = Some(file_name_string(&csv)?);
        final_meta.duration_mode = Some(duration.label().into());
        final_meta.requested_duration_seconds = duration.requested_seconds();
        final_meta.stop_reason = Some(reason);
        final_meta.initial_free_disk_bytes = Some(initial_free_disk_bytes);
        final_meta.final_free_disk_bytes = final_free_disk_bytes;
        final_meta.completion_status = if reason.is_complete() {
            "complete"
        } else {
            "incomplete"
        }
        .into();
        final_meta.markers = self.lock_runtime()?.markers.clone();
        fs::write(
            &metadata,
            serde_json::to_vec_pretty(&final_meta)
                .map_err(crate::recording::RecordingError::Json)?,
        )?;
        let summary = SessionSummary {
            state: if terminal_error_detail.is_some() {
                SessionState::Faulted
            } else {
                SessionState::Disconnected
            },
            samples: snapshot.sample_count,
            packets: snapshot.integrity.received_packets,
            measured_rate_hz: snapshot.measured_rate_hz,
            board_elapsed_seconds: board_elapsed,
            host_elapsed_seconds: host_elapsed,
            bmeg_path: bmeg.display().to_string(),
            csv_path: csv.display().to_string(),
            metadata_path: metadata.display().to_string(),
            recording_status: recording_status.into(),
            duration: duration.clone(),
            stop_reason: reason,
            completion_status: final_meta.completion_status.clone(),
            initial_free_disk_bytes: Some(initial_free_disk_bytes),
            final_free_disk_bytes,
            integrity: snapshot.integrity,
            error: terminal_error_detail.clone(),
            profile,
            calibration: final_meta.calibration.clone().unwrap_or_default(),
            active_digital_output_mask,
            final_digital_output_mask,
        };
        self.set_summary(summary.clone())?;
        self.set_runtime_stop_reason(reason)?;
        if let Some(error) = terminal_error {
            // Preserve the stage/kind/Windows error in Advanced details rather
            // than overwriting it with the generic I/O display string.
            self.set_fault(terminal_error_detail.unwrap_or_else(|| error.detail()))?;
            return Err(error.into_session_error());
        }
        if csv_rows != summary.samples {
            let error = SessionError::Recording(crate::recording::RecordingError::Truncated);
            self.set_fault(error.to_string())?;
            return Err(error);
        }
        self.set_state(SessionState::Disconnected)?;
        Ok(())
    }

    fn wait_for_handshake<T: Read + Write>(
        &self,
        io: &mut T,
        acquisition: &mut AcquisitionController,
    ) -> Result<(), SessionError> {
        self.wait_for_handshake_with_policy(
            io,
            acquisition,
            HANDSHAKE_STARTUP_GRACE,
            HANDSHAKE_OVERALL_TIMEOUT,
            HANDSHAKE_PING_ATTEMPTS,
            HANDSHAKE_PING_INTERVAL,
        )
    }

    fn wait_for_handshake_with_policy<T: Read + Write>(
        &self,
        io: &mut T,
        acquisition: &mut AcquisitionController,
        startup_grace: Duration,
        overall_timeout: Duration,
        ping_attempt_limit: u32,
        ping_interval: Duration,
    ) -> Result<(), SessionError> {
        let started = Instant::now();
        let deadline = started + overall_timeout;
        let mut next_ping = started + startup_grace;
        let mut ping_attempts = 0u32;
        let mut bytes_received = 0u64;
        let mut valid_packets_seen = 0u64;
        let mut buffer = [0u8; 256];

        while Instant::now() < deadline {
            if self.cancel.load(Ordering::Acquire) {
                return Err(SessionError::State("session was cancelled"));
            }
            if ping_attempts < ping_attempt_limit && Instant::now() >= next_ping {
                self.send_command(io, MessageType::Ping, ping_attempts, vec![])?;
                ping_attempts += 1;
                next_ping += ping_interval;
            }
            match io.read(&mut buffer) {
                Ok(n) if n > 0 => {
                    bytes_received += n as u64;
                    acquisition.ingest_bytes(&buffer[..n]);
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => {
                    let snapshot = acquisition.snapshot();
                    self.update_handshake_diagnostics(
                        &snapshot,
                        bytes_received,
                        ping_attempts,
                        started.elapsed(),
                        Some(ConnectionFailureCategory::DeviceDisconnected),
                    )?;
                    return Err(SessionError::Io(error));
                }
            }
            let snapshot = acquisition.snapshot();
            if snapshot.integrity.received_packets > valid_packets_seen {
                // A fragmented valid handshake is progress.  Do not inject an
                // unnecessary PING between its HELLO/CAPABILITIES/PONG frames.
                // Passive HELLO/CAPABILITIES traffic must not postpone the
                // first scheduled PING, because the required PONG has not yet
                // been solicited.
                valid_packets_seen = snapshot.integrity.received_packets;
                if ping_attempts > 0 {
                    next_ping = Instant::now() + ping_interval;
                }
            }
            self.update_snapshot(&snapshot)?;
            if snapshot.hello_seen && snapshot.capabilities_seen && snapshot.pong_seen {
                if let Some(category) = firmware_identity_failure_category(&snapshot) {
                    self.update_handshake_diagnostics(
                        &snapshot,
                        bytes_received,
                        ping_attempts,
                        started.elapsed(),
                        Some(category.clone()),
                    )?;
                    return Err(SessionError::Protocol(format!(
                        "handshake reached protocol v{}.{} on {} but firmware identity was not accepted: build={:?}, device={:?}. {}",
                        crate::protocol::PROTOCOL_MAJOR,
                        crate::protocol::PROTOCOL_MINOR,
                        self.status()?.port,
                        snapshot.firmware_build,
                        snapshot.firmware_board_id,
                        recommended_action(&category),
                    )));
                }
                self.update_handshake_diagnostics(
                    &snapshot,
                    bytes_received,
                    ping_attempts,
                    started.elapsed(),
                    None,
                )?;
                return Ok(());
            }
        }

        let snapshot = acquisition.snapshot();
        let category = handshake_failure_category(&snapshot, bytes_received);
        self.update_handshake_diagnostics(
            &snapshot,
            bytes_received,
            ping_attempts,
            started.elapsed(),
            Some(category.clone()),
        )?;
        Err(SessionError::Protocol(format!(
            "handshake failed on {} after {} ms: {:?}; bytes={}, valid_frames={}, CRC failures={}, HELLO={}, CAPABILITIES={}, PONG={}. {}",
            self.status()?.port,
            started.elapsed().as_millis(),
            category,
            bytes_received,
            snapshot.integrity.received_packets,
            snapshot.integrity.crc_failures,
            snapshot.hello_seen,
            snapshot.capabilities_seen,
            snapshot.pong_seen,
            recommended_action(&category),
        )))
    }

    fn wait_until<T: Read + Write, F: Fn(&AcquisitionSnapshot) -> bool>(
        &self,
        io: &mut T,
        acquisition: &mut AcquisitionController,
        predicate: F,
        name: &str,
    ) -> Result<(), SessionError> {
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        let mut buffer = [0u8; 256];
        while Instant::now() < deadline {
            if self.cancel.load(Ordering::Acquire) {
                return Err(SessionError::State("session was cancelled"));
            }
            match io.read(&mut buffer) {
                Ok(n) if n > 0 => acquisition.ingest_bytes(&buffer[..n]),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => return Err(SessionError::Io(error)),
            }
            let snapshot = acquisition.snapshot();
            self.update_snapshot(&snapshot)?;
            if predicate(&snapshot) {
                return Ok(());
            }
        }
        let snapshot = acquisition.snapshot();
        Err(SessionError::Protocol(format!(
            "timed out waiting for {name} (HELLO={}, CAPABILITIES={}, PONG={}, CONFIG_ACK={}, STATUS={})",
            snapshot.hello_seen,
            snapshot.capabilities_seen,
            snapshot.pong_seen,
            snapshot.config_ack_seen,
            snapshot.status_seen,
        )))
    }

    fn send_command<T: Write>(
        &self,
        io: &mut T,
        message_type: MessageType,
        sequence: u32,
        payload: Vec<u8>,
    ) -> Result<(), SessionError> {
        let bytes = encode_command(message_type, sequence, payload)?;
        io.write_all(&bytes)?;
        io.flush()?;
        Ok(())
    }

    /// Writes a command from the active capture loop while retaining whether a
    /// Windows error originated in `write_all` or `flush`. Startup commands
    /// deliberately remain on `send_command`.
    fn send_capture_command<T: Write>(
        &self,
        io: &mut T,
        message_type: MessageType,
        sequence: u32,
        payload: Vec<u8>,
    ) -> Result<(), CaptureCommandError> {
        let bytes = encode_command(message_type, sequence, payload)
            .map_err(|error| CaptureCommandError::Protocol(error.to_string()))?;
        let write_stage = if message_type == MessageType::Ping {
            SerialFailureStage::PingWrite
        } else {
            SerialFailureStage::StopWrite
        };
        io.write_all(&bytes).map_err(|error| {
            CaptureCommandError::Serial(SerialTerminalFailure::new(write_stage, error))
        })?;
        let flush_stage = if message_type == MessageType::Ping {
            SerialFailureStage::PingFlush
        } else {
            SerialFailureStage::StopFlush
        };
        io.flush().map_err(|error| {
            CaptureCommandError::Serial(SerialTerminalFailure::new(flush_stage, error))
        })?;
        Ok(())
    }

    fn record_serial_cleanup_failure(
        &self,
        error: &SerialTerminalFailure,
        port: &str,
        started: Instant,
        started_utc: DateTime<Utc>,
        progress: &CaptureProgress,
    ) {
        crate::app_log::record(
            "WARN",
            &format!(
                "SERIAL_CLEANUP_ERROR stage={} classification={} error_kind={:?} raw_os_error={:?} detail={} elapsed_ms={} port={} samples={} packets={} last_valid_packet_utc={} last_valid_sample_utc={} last_successful_ping_utc={} last_pong_or_status_utc={}",
                error.stage.label(),
                error.stage.classification(),
                error.error.kind(),
                error.error.raw_os_error(),
                error.error,
                started.elapsed().as_millis(),
                port,
                progress.samples,
                progress.packets,
                capture_timestamp(started_utc, started, progress.last_valid_packet).unwrap_or_else(|| "none".into()),
                capture_timestamp(started_utc, started, progress.last_valid_sample).unwrap_or_else(|| "none".into()),
                capture_timestamp(started_utc, started, progress.last_successful_ping).unwrap_or_else(|| "none".into()),
                capture_timestamp(started_utc, started, progress.last_pong_or_status).unwrap_or_else(|| "none".into()),
            ),
        );
    }

    fn record_active_serial_failure(
        &self,
        error: &SerialTerminalFailure,
        context: ActiveSerialFailureContext<'_>,
    ) -> Result<ConnectionFailureCategory, SessionError> {
        // `available_ports` is a read-only operating-system enumeration. It
        // never opens the COM handle and does not perform the 1200-bps reset.
        let port_state = diagnose_port_enumeration(context.port, context.expected_identity);
        let device_removed = port_state.selected_port_present == Some(false)
            && port_state.same_serial_present == Some(false);
        let category = if device_removed {
            ConnectionFailureCategory::DeviceDisconnected
        } else {
            match error.stage {
                SerialFailureStage::FirmwareError => {
                    ConnectionFailureCategory::FirmwareProtocolError
                }
                SerialFailureStage::NoDataTimeout => ConnectionFailureCategory::NoDataTimeout,
                _ => ConnectionFailureCategory::SerialTransportError,
            }
        };
        let elapsed_ms = context.started.elapsed().as_millis();
        let last_valid_packet_utc = capture_timestamp(
            context.started_utc,
            context.started,
            context.progress.last_valid_packet,
        );
        let last_valid_sample_utc = capture_timestamp(
            context.started_utc,
            context.started,
            context.progress.last_valid_sample,
        );
        let last_ping_attempt_utc = capture_timestamp(
            context.started_utc,
            context.started,
            context.progress.last_ping_attempt,
        );
        let last_successful_ping_utc = capture_timestamp(
            context.started_utc,
            context.started,
            context.progress.last_successful_ping,
        );
        let last_pong_or_status_utc = capture_timestamp(
            context.started_utc,
            context.started,
            context.progress.last_pong_or_status,
        );
        let last_recording_flush_utc = capture_timestamp(
            context.started_utc,
            context.started,
            context.progress.last_recording_flush,
        );
        let last_disk_check_utc = capture_timestamp(
            context.started_utc,
            context.started,
            context.progress.last_disk_check,
        );
        let available_disk_bytes = self.lock_runtime()?.available_disk_bytes;

        crate::app_log::record(
            "ERROR",
            &format!(
                "SERIAL_TERMINAL_ERROR stage={} classification={} error_kind={:?} raw_os_error={:?} detail={} elapsed_ms={} port={} samples={} packets={} last_valid_packet_utc={} last_valid_sample_utc={} last_ping_attempt_utc={} last_successful_ping_utc={} last_ping_write_duration_ms={:?} last_pong_or_status_utc={} last_recording_flush_utc={} last_recording_flush_duration_ms={:?} last_disk_check_utc={} last_disk_check_duration_ms={:?} available_disk_bytes={:?} selected_port_present={:?} same_vid_pid_present={:?} same_serial_present={:?} uno_r4_present={:?} observed_vid={:?} observed_pid={:?} observed_serial={:?} enumeration_error={:?}",
                error.stage.label(),
                error.stage.classification(),
                error.error.kind(),
                error.error.raw_os_error(),
                error.error,
                elapsed_ms,
                context.port,
                context.snapshot.sample_count,
                context.snapshot.integrity.received_packets,
                last_valid_packet_utc.as_deref().unwrap_or("none"),
                last_valid_sample_utc.as_deref().unwrap_or("none"),
                last_ping_attempt_utc.as_deref().unwrap_or("none"),
                last_successful_ping_utc.as_deref().unwrap_or("none"),
                context.progress.last_ping_write_duration.map(|duration| duration.as_millis()),
                last_pong_or_status_utc.as_deref().unwrap_or("none"),
                last_recording_flush_utc.as_deref().unwrap_or("none"),
                context.progress.last_recording_flush_duration.map(|duration| duration.as_millis()),
                last_disk_check_utc.as_deref().unwrap_or("none"),
                context.progress.last_disk_check_duration.map(|duration| duration.as_millis()),
                available_disk_bytes,
                port_state.selected_port_present,
                port_state.same_vid_pid_present,
                port_state.same_serial_present,
                port_state.uno_r4_present,
                port_state.observed_vid,
                port_state.observed_pid,
                port_state.observed_serial_number,
                port_state.enumeration_error,
            ),
        );

        let mut runtime = self.lock_runtime()?;
        let diagnostics = runtime
            .connection_diagnostics
            .get_or_insert_with(|| ConnectionDiagnostics::new(context.port));
        diagnostics.terminal_error_classification = Some(error.stage.classification().into());
        diagnostics.terminal_error_stage = Some(error.stage.label().into());
        diagnostics.terminal_error_kind = Some(format!("{:?}", error.error.kind()));
        diagnostics.terminal_error_raw_os_error = error.error.raw_os_error();
        diagnostics.terminal_error_detail = Some(error.error.to_string());
        diagnostics.terminal_error_elapsed_ms = Some(elapsed_ms);
        diagnostics.last_valid_packet_utc = last_valid_packet_utc;
        diagnostics.last_valid_sample_utc = last_valid_sample_utc;
        diagnostics.last_successful_ping_utc = last_successful_ping_utc;
        diagnostics.last_pong_or_status_utc = last_pong_or_status_utc;
        diagnostics.selected_port_present_after_error = port_state.selected_port_present;
        diagnostics.same_vid_pid_present_after_error = port_state.same_vid_pid_present;
        diagnostics.same_serial_present_after_error = port_state.same_serial_present;
        diagnostics.uno_r4_present_after_error = port_state.uno_r4_present;
        diagnostics.port_enumeration_error = port_state.enumeration_error;
        diagnostics.failure_category = Some(category.clone());
        diagnostics.recommended_action = recommended_action(&category).into();
        Ok(category)
    }

    fn drain_samples(
        &self,
        receiver: &Receiver<SynchronizedRecord>,
        raw: &mut BmegWriter,
    ) -> Result<(), SessionError> {
        for sample in receiver.try_iter() {
            raw.write_record(&sample)?;
            let mut runtime = self.lock_runtime()?;
            runtime
                .display_origin_timestamp_us
                .get_or_insert(sample.timestamp_us);
            if runtime.recent.len() == DISPLAY_CAPACITY {
                runtime.recent.pop_front();
            }
            runtime.recent.push_back(sample);
        }
        Ok(())
    }

    fn initial_metadata(
        &self,
        InitialMetadataRequest {
            simulator,
            source,
            bmeg,
            duration,
            profile,
            calibration,
            recording_path_context,
        }: InitialMetadataRequest<'_>,
    ) -> Result<RecordingMetadata, SessionError> {
        let (arduino_cli_version, uno_r4_core_version, board_serial) = if simulator {
            ("not applicable".into(), "not applicable".into(), None)
        } else {
            let cli = crate::arduino_cli::ArduinoCli::discover(None)
                .map_err(|error| SessionError::Protocol(error.to_string()))?;
            let version = cli
                .version()
                .map_err(|error| SessionError::Protocol(error.to_string()))?
                .stdout
                .trim()
                .to_owned();
            let core = cli
                .uno_r4_core_version()
                .map_err(|error| SessionError::Protocol(error.to_string()))?;
            let serial = cli
                .boards()
                .ok()
                .into_iter()
                .flatten()
                .find(|board| board.port.eq_ignore_ascii_case(source))
                .and_then(|board| board.serial_number);
            (version, core, serial)
        };
        let mut digital_output_mapping = std::collections::BTreeMap::new();
        for output in profile.profile.acquisition.resolved_digital_outputs() {
            let key = match output.label.to_ascii_lowercase().as_str() {
                "green led" => "green".into(),
                "red led" => "red".into(),
                "ir led" => "ir".into(),
                _ => output.label,
            };
            digital_output_mapping.insert(key, output.pin);
        }
        Ok(RecordingMetadata {
            utc_start: Utc::now(),
            local_start: Local::now(),
            board: if simulator {
                "Simulator".into()
            } else {
                "Arduino UNO R4 WiFi".into()
            },
            board_serial,
            com_port: source.into(),
            fqbn: if simulator {
                "simulator".into()
            } else {
                "arduino:renesas_uno:unor4wifi".into()
            },
            arduino_cli_version,
            uno_r4_core_version,
            firmware_build: REFERENCE_FIRMWARE_BUILD,
            protocol_version: "0.3".into(),
            analog_pin: profile.profile.acquisition.analog_pins().join(","),
            active_analog_pins: profile.profile.acquisition.analog_pins(),
            digital_output_mapping,
            adc_bits: profile.profile.acquisition.adc_resolution_bits,
            requested_sample_rate_hz: profile.profile.acquisition.sample_rate_hz,
            measured_sample_rate_hz: 0.0,
            total_samples: 0,
            integrity: IntegrityCounters::default(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            simulator,
            utc_stop: None,
            local_stop: None,
            host_elapsed_seconds: None,
            board_elapsed_seconds: None,
            recording_status: "active".into(),
            bmeg_filename: file_name_string(bmeg).unwrap_or_else(|_| "recording.bmeg".into()),
            csv_filename: None,
            notes: if simulator {
                match profile.profile.category.as_str() {
                    "ecg" => {
                        "Synthetic ECG-like teaching waveform; nonphysiological teaching data only."
                            .into()
                    }
                    "emg" => {
                        "Synthetic EMG-like teaching waveform; nonphysiological teaching data only."
                            .into()
                    }
                    _ => "Deterministic simulator waveform; no human signal.".into(),
                }
            } else {
                "A0 raw floating/uncalibrated engineering communication test; no human signal."
                    .into()
            },
            project_folder: recording_path_context.map(|context| context.project_folder.clone()),
            output_folder: recording_path_context.map(|context| context.output_folder.clone()),
            duration_mode: Some(duration.label().into()),
            requested_duration_seconds: duration.requested_seconds(),
            stop_reason: None,
            initial_free_disk_bytes: None,
            final_free_disk_bytes: None,
            completion_status: "active".into(),
            profile_snapshot: Some(profile.clone()),
            // Retained as an optional legacy metadata field. New course recordings
            // never create the retired validation context.
            validation_context: None,
            markers: Vec::new(),
            calibration: Some(calibration),
        })
    }

    fn allocate_paths(
        &self,
        output_dir: &Path,
        profile: &ProfileSnapshot,
    ) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), SessionError> {
        fs::create_dir_all(output_dir)?;
        let stamp = Local::now().format("%Y%m%d_%H%M%S");
        for run in 1..=99 {
            let name = match profile.profile.category.as_str() {
                "development" => "Phase1_A0".to_string(),
                category if category.starts_with("course_") => format!(
                    "Phase4_{}",
                    crate::profiles::safe_filename_component(
                        category.trim_start_matches("course_")
                    )
                ),
                category => format!(
                    "Phase3A_{}",
                    crate::profiles::safe_filename_component(category)
                ),
            };
            let base = output_dir.join(format!("{stamp}_{name}_Run{run:02}"));
            let bmeg = base.with_extension("bmeg");
            let csv = base.with_extension("csv");
            let metadata = base.with_extension("metadata.json");
            let temporary = base.with_extension("bmeg.part");
            if !bmeg.exists() && !csv.exists() && !metadata.exists() && !temporary.exists() {
                return Ok((temporary, bmeg, csv, metadata));
            }
        }
        Err(SessionError::State(
            "could not allocate a unique recording filename",
        ))
    }

    fn set_state(&self, state: SessionState) -> Result<(), SessionError> {
        self.lock_runtime()?.state = state;
        Ok(())
    }

    fn set_fault(&self, error: String) -> Result<(), SessionError> {
        let mut runtime = self.lock_runtime()?;
        runtime.state = SessionState::Faulted;
        runtime.last_error = Some(error);
        Ok(())
    }

    fn finish_worker_error(&self, result: &Result<(), SessionError>) -> Result<(), SessionError> {
        if let Err(error) = result {
            if self.cancel.load(Ordering::Acquire) {
                self.set_state(SessionState::Disconnected)?;
            } else {
                self.record_worker_failure_category(error)?;
                // Active capture may already have stored a more useful serial
                // stage/ErrorKind/Windows code. Do not replace it with the
                // generic `I/O: ...` rendering as the worker unwinds.
                let detailed_fault_is_present = {
                    let runtime = self.lock_runtime()?;
                    runtime.state == SessionState::Faulted && runtime.last_error.is_some()
                };
                if !detailed_fault_is_present {
                    self.set_fault(error.to_string())?;
                }
            }
        }
        Ok(())
    }

    fn record_worker_failure_category(&self, error: &SessionError) -> Result<(), SessionError> {
        let mut runtime = self.lock_runtime()?;
        if let Some(diagnostics) = runtime.connection_diagnostics.as_mut() {
            if diagnostics.failure_category.is_none() {
                let category = if !diagnostics.port_opened {
                    ConnectionFailureCategory::PortBusy
                } else if matches!(error, SessionError::Io(_)) {
                    // An I/O error by itself does not prove that Windows
                    // removed the USB device. Active capture records a
                    // read-only enumeration before assigning DeviceDisconnected.
                    ConnectionFailureCategory::SerialTransportError
                } else {
                    ConnectionFailureCategory::HandshakeIncomplete
                };
                diagnostics.failure_category = Some(category.clone());
                diagnostics.recommended_action = recommended_action(&category).into();
            }
        }
        Ok(())
    }

    fn set_summary(&self, summary: SessionSummary) -> Result<(), SessionError> {
        self.lock_runtime()?.last_summary = Some(summary);
        Ok(())
    }

    fn update_from_acquisition(
        &self,
        acquisition: &AcquisitionController,
    ) -> Result<(), SessionError> {
        self.update_snapshot(&acquisition.snapshot())
    }

    fn update_snapshot(&self, snapshot: &AcquisitionSnapshot) -> Result<(), SessionError> {
        let mut runtime = self.lock_runtime()?;
        runtime.samples = snapshot.sample_count;
        runtime.packets = snapshot.integrity.received_packets;
        runtime.measured_rate_hz = snapshot.measured_rate_hz;
        runtime.integrity = snapshot.integrity.clone();
        runtime.digital_output_mask = snapshot.digital_output_mask;
        Ok(())
    }

    fn mark_port_opened(&self, port: &str) -> Result<(), SessionError> {
        let mut runtime = self.lock_runtime()?;
        let diagnostics = runtime
            .connection_diagnostics
            .get_or_insert_with(|| ConnectionDiagnostics::new(port));
        diagnostics.selected_port = port.into();
        diagnostics.port_opened = true;
        Ok(())
    }

    fn update_handshake_diagnostics(
        &self,
        snapshot: &AcquisitionSnapshot,
        bytes_received: u64,
        ping_attempts: u32,
        elapsed: Duration,
        failure_category: Option<ConnectionFailureCategory>,
    ) -> Result<(), SessionError> {
        let mut runtime = self.lock_runtime()?;
        let selected_port = runtime.port.clone();
        let diagnostics = runtime
            .connection_diagnostics
            .get_or_insert_with(|| ConnectionDiagnostics::new(selected_port));
        diagnostics.bytes_received = bytes_received;
        diagnostics.valid_frames = snapshot.integrity.received_packets;
        diagnostics.crc_failures = snapshot.integrity.crc_failures;
        diagnostics.skipped_noise_bytes = snapshot.skipped_noise_bytes;
        diagnostics.hello_received = snapshot.hello_seen;
        diagnostics.capabilities_received = snapshot.capabilities_seen;
        diagnostics.pong_received = snapshot.pong_seen;
        diagnostics.protocol_version = if snapshot.integrity.received_packets > 0 {
            Some(format!(
                "{}.{}",
                crate::protocol::PROTOCOL_MAJOR,
                crate::protocol::PROTOCOL_MINOR
            ))
        } else {
            None
        };
        diagnostics.firmware_build = snapshot.firmware_build;
        diagnostics.firmware_board_id = snapshot.firmware_board_id;
        diagnostics.firmware_capabilities = snapshot.firmware_capabilities.clone();
        diagnostics.raw_byte_classification =
            raw_byte_classification(snapshot, bytes_received).into();
        diagnostics.ping_attempts = ping_attempts;
        diagnostics.handshake_elapsed_ms = elapsed.as_millis();
        diagnostics.failure_category = failure_category.clone();
        diagnostics.recommended_action = failure_category.as_ref().map_or_else(
            || "Protocol handshake succeeded.".into(),
            |category| recommended_action(category).into(),
        );
        Ok(())
    }

    fn set_reset_diagnostics(
        &self,
        diagnostics: ConnectionDiagnostics,
    ) -> Result<(), SessionError> {
        let mut runtime = self.lock_runtime()?;
        runtime.port = diagnostics
            .final_port
            .as_ref()
            .unwrap_or(&diagnostics.selected_port)
            .clone();
        runtime.board = diagnostics.board.clone();
        runtime.connection_diagnostics = Some(diagnostics);
        Ok(())
    }

    fn status_from_runtime(runtime: &SessionRuntime) -> SessionStatus {
        let elapsed_seconds = if matches!(
            runtime.state,
            SessionState::Acquiring | SessionState::Stopping
        ) {
            runtime
                .started_at
                .map(|started| started.elapsed().as_secs_f64())
                .unwrap_or(0.0)
        } else {
            // A completed or faulted recording must keep the capture duration
            // measured by the worker, rather than continuing to count from the
            // session's initial handshake/start time while the UI is reviewed.
            runtime
                .last_summary
                .as_ref()
                .map(|summary| summary.host_elapsed_seconds)
                .unwrap_or(0.0)
        };
        let remaining_seconds = match runtime.duration.as_ref() {
            Some(RecordingDuration::Timed { seconds }) => {
                Some((*seconds as f64 - elapsed_seconds).max(0.0))
            }
            Some(RecordingDuration::UntilStopped) | None => None,
        };
        SessionStatus {
            state: runtime.state,
            board: runtime.board.clone(),
            port: runtime.port.clone(),
            protocol_version: runtime.protocol_version.clone(),
            simulator: runtime.simulator,
            samples: runtime.samples,
            packets: runtime.packets,
            measured_rate_hz: runtime.measured_rate_hz,
            integrity: runtime.integrity.clone(),
            duration: runtime.duration.clone(),
            elapsed_seconds,
            remaining_seconds,
            available_disk_bytes: runtime.available_disk_bytes,
            storage_warning: runtime.storage_warning.clone(),
            stop_reason: runtime.stop_reason,
            connection_diagnostics: runtime.connection_diagnostics.clone(),
            last_error: runtime.last_error.clone(),
            last_summary: runtime.last_summary.clone(),
            profile: runtime.profile.clone(),
            calibration: runtime.calibration.clone(),
            digital_output_mask: runtime.digital_output_mask,
            display_origin_timestamp_us: runtime.display_origin_timestamp_us,
        }
    }

    fn free_disk_space(&self, output_dir: &Path) -> Result<u64, SessionError> {
        self.disk_space
            .available_space(output_dir)
            .map_err(|error| {
                SessionError::Storage(format!(
                    "could not determine available disk space for {}: {error}",
                    output_dir.display()
                ))
            })
    }

    fn update_disk_space(&self, bytes: u64) -> Result<(), SessionError> {
        let mut runtime = self.lock_runtime()?;
        runtime.available_disk_bytes = Some(bytes);
        runtime.storage_warning = if bytes < DISK_WARNING_BYTES {
            Some(format!(
                "Low disk space: {} MiB available; acquisition will stop below {} MiB.",
                bytes / (1024 * 1024),
                DISK_CRITICAL_BYTES / (1024 * 1024)
            ))
        } else {
            None
        };
        Ok(())
    }

    fn set_stop_reason_once(&self, reason: StopReason) -> Result<StopReason, SessionError> {
        let committed = {
            let mut stop_reason = self
                .stop_reason
                .lock()
                .map_err(|_| SessionError::State("stop reason lock poisoned"))?;
            *stop_reason.get_or_insert(reason)
        };
        self.set_runtime_stop_reason(committed)?;
        Ok(committed)
    }

    fn stop_reason(&self) -> Result<Option<StopReason>, SessionError> {
        self.stop_reason
            .lock()
            .map(|reason| *reason)
            .map_err(|_| SessionError::State("stop reason lock poisoned"))
    }

    fn set_runtime_stop_reason(&self, reason: StopReason) -> Result<(), SessionError> {
        self.lock_runtime()?.stop_reason = Some(reason);
        Ok(())
    }

    fn lock_runtime(&self) -> Result<std::sync::MutexGuard<'_, SessionRuntime>, SessionError> {
        self.runtime
            .lock()
            .map_err(|_| SessionError::State("session lock poisoned"))
    }
}

fn file_name_string(path: &Path) -> Result<String, SessionError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or(SessionError::State("recording filename is invalid"))
}

fn encode_command(
    message_type: MessageType,
    sequence: u32,
    payload: Vec<u8>,
) -> Result<Vec<u8>, SessionError> {
    encode_frame(&Frame {
        message_type,
        flags: 0,
        sequence,
        payload,
    })
    .map_err(|error| SessionError::Protocol(format!("{error:?}")))
}

fn capture_timestamp(
    started_utc: DateTime<Utc>,
    started: Instant,
    observed: Option<Instant>,
) -> Option<String> {
    let elapsed = observed?.checked_duration_since(started)?;
    let elapsed = ChronoDuration::from_std(elapsed).ok()?;
    Some((started_utc + elapsed).to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn default_general_profile() -> Result<ProfileSnapshot, SessionError> {
    let profile = built_in_profiles()
        .map_err(|error| SessionError::Protocol(error.to_string()))?
        .into_iter()
        .find(|profile| profile.category == "development")
        .ok_or(SessionError::State("the General A0 profile is unavailable"))?;
    Ok(profile.snapshot(false))
}

fn validate_profile_snapshot(snapshot: &ProfileSnapshot) -> Result<(), SessionError> {
    snapshot
        .profile
        .validate()
        .map_err(|error| SessionError::Protocol(error.to_string()))?;
    if snapshot.profile.status != ProfileStatus::Locked {
        return Err(SessionError::State(
            "only a validated locked profile may start acquisition",
        ));
    }
    if snapshot.profile.requires_bench_acknowledgement() && !snapshot.bench_notice_acknowledged {
        return Err(SessionError::State(
            "acknowledge the bench-only ECG/EMG notice before recording",
        ));
    }
    Ok(())
}

fn configure_payload(
    settings: &crate::profiles::AcquisitionSettings,
    capabilities: Option<&FirmwareCapabilities>,
) -> Result<Vec<u8>, SessionError> {
    let pin_id = |pin: &str| match pin {
        "A0" => Ok(0),
        "A1" => Ok(1),
        "A2" => Ok(2),
        "A3" => Ok(3),
        "A4" => Ok(4),
        "A5" => Ok(5),
        _ => Err(SessionError::State("profile analog pin is unsupported")),
    };
    let mut payload = Vec::with_capacity(16);
    match settings.acquisition_mode {
        crate::profiles::AcquisitionMode::Simultaneous => {
            let pins: Result<Vec<_>, _> = settings
                .analog_pins()
                .iter()
                .map(|pin| pin_id(pin))
                .collect();
            let pins = pins?;
            if !(1..=6).contains(&pins.len())
                || !matches!(settings.adc_resolution_bits, 12 | 14)
                || settings.sample_rate_hz == 0
                || settings.sample_rate_hz > 1_000
            {
                return Err(SessionError::State(
                    "profile requests an unsupported simultaneous acquisition configuration",
                ));
            }
            let output_mask = settings
                .resolved_digital_outputs()
                .into_iter()
                .filter(|output| {
                    output.behavior == crate::profiles::DigitalOutputBehavior::HighWhileRecording
                })
                .try_fold(0u8, |mask, output| match output.pin.as_str() {
                    "D4" => Ok(mask | 0x01),
                    "D5" => Ok(mask | 0x02),
                    "D6" => Ok(mask | 0x04),
                    _ => Err(SessionError::State("profile digital output is unsupported")),
                })?;
            validate_firmware_capabilities(
                capabilities,
                0x01,
                pins.len(),
                settings.adc_resolution_bits,
                Some(settings.sample_rate_hz),
                output_mask,
            )?;
            payload.extend_from_slice(&[0, settings.adc_resolution_bits]);
            payload.extend_from_slice(&settings.sample_rate_hz.to_le_bytes());
            payload.push(pins.len() as u8);
            payload.extend_from_slice(&pins);
            payload.push(output_mask);
        }
        crate::profiles::AcquisitionMode::Pulseox4State => {
            let inputs = settings.analog_inputs.as_ref().ok_or(SessionError::State(
                "pulse-ox profile is missing its TX/RX analog inputs",
            ))?;
            let outputs = settings.resolved_digital_outputs();
            let red = outputs
                .iter()
                .find(|output| output.label.eq_ignore_ascii_case("red led"))
                .ok_or(SessionError::State(
                    "pulse-ox profile is missing a RED output",
                ))?;
            let ir = outputs
                .iter()
                .find(|output| output.label.eq_ignore_ascii_case("ir led"))
                .ok_or(SessionError::State(
                    "pulse-ox profile is missing an IR output",
                ))?;
            if red.pin == ir.pin
                || red.behavior != crate::profiles::DigitalOutputBehavior::AcquisitionSequenced
                || ir.behavior != crate::profiles::DigitalOutputBehavior::AcquisitionSequenced
            {
                return Err(SessionError::State(
                    "pulse-ox RED and IR must use distinct acquisition-sequenced outputs",
                ));
            }
            let output_pin = |pin: &str| match pin {
                "D4" => Ok(4),
                "D5" => Ok(5),
                "D6" => Ok(6),
                _ => Err(SessionError::State(
                    "pulse-ox profile digital output is unsupported",
                )),
            };
            let dwell = settings.state_dwell_us.ok_or(SessionError::State(
                "pulse-ox profile is missing state dwell time",
            ))?;
            if !matches!(settings.adc_resolution_bits, 12 | 14)
                || !(250..=5_000).contains(&dwell)
                || settings.analog_pins().len() != 2
                || red.pin == ir.pin
            {
                return Err(SessionError::State(
                    "profile requests an unsupported fixed pulse-ox configuration",
                ));
            }
            let output_mask = match (red.pin.as_str(), ir.pin.as_str()) {
                ("D4", "D5") | ("D5", "D4") => 0x03,
                ("D4", "D6") | ("D6", "D4") => 0x05,
                ("D5", "D6") | ("D6", "D5") => 0x06,
                _ => {
                    return Err(SessionError::State(
                        "pulse-ox profile digital output is unsupported",
                    ))
                }
            };
            validate_firmware_capabilities(
                capabilities,
                0x02,
                2,
                settings.adc_resolution_bits,
                None,
                output_mask,
            )?;
            payload.extend_from_slice(&[1, settings.adc_resolution_bits]);
            payload.extend_from_slice(&dwell.to_le_bytes());
            payload.extend_from_slice(&[
                2,
                pin_id(&inputs.tx)?,
                pin_id(&inputs.rx)?,
                output_pin(&red.pin)?,
                output_pin(&ir.pin)?,
            ]);
        }
    }
    Ok(payload)
}

fn validate_firmware_capabilities(
    capabilities: Option<&FirmwareCapabilities>,
    required_mode: u8,
    channel_count: usize,
    adc_bits: u8,
    rate_hz: Option<u32>,
    output_mask: u8,
) -> Result<(), SessionError> {
    let Some(capabilities) = capabilities else {
        // Legacy CAPABILITIES packets did not carry enough detail to make a
        // Resource claim. The existing conservative configuration
        // checks above remain in effect for backward reader compatibility.
        return Ok(());
    };
    if !capabilities.supports_mode(required_mode) {
        return Err(SessionError::State(
            "connected firmware does not support this acquisition mode",
        ));
    }
    if channel_count > usize::from(capabilities.max_analog_channels) {
        return Err(SessionError::State(
            "connected firmware does not support this many analog channels",
        ));
    }
    if !capabilities.supports_adc_resolution(adc_bits) {
        return Err(SessionError::State(
            "connected firmware does not support this ADC resolution",
        ));
    }
    if rate_hz.is_some_and(|rate| !capabilities.supports_rate(rate)) {
        return Err(SessionError::State(
            "connected firmware does not support this frame/cycle rate",
        ));
    }
    if output_mask & !capabilities.supported_digital_output_mask != 0 {
        return Err(SessionError::State(
            "connected firmware does not support this controlled output mapping",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct UnoUsbPort {
    port: String,
    serial_number: Option<String>,
    role: UnoUsbRole,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnoUsbRole {
    Application,
    Bootloader,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ReturningPort {
    Found(UnoUsbPort),
    Absent,
    Ambiguous,
}

fn enumerate_uno_usb_ports() -> Vec<UnoUsbPort> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|candidate| match candidate.port_type {
            serialport::SerialPortType::UsbPort(info)
                if info.vid == 0x2341 && matches!(info.pid, 0x1002 | 0x006d) =>
            {
                Some(UnoUsbPort {
                    port: candidate.port_name,
                    serial_number: info.serial_number,
                    role: if info.pid == 0x006d {
                        UnoUsbRole::Bootloader
                    } else {
                        UnoUsbRole::Application
                    },
                })
            }
            _ => None,
        })
        .collect()
}

fn usb_port_identity(port_name: &str) -> Option<UsbPortIdentity> {
    serialport::available_ports()
        .ok()?
        .into_iter()
        .find_map(|candidate| {
            if !candidate.port_name.eq_ignore_ascii_case(port_name) {
                return None;
            }
            match candidate.port_type {
                serialport::SerialPortType::UsbPort(info) => Some(UsbPortIdentity {
                    port: candidate.port_name,
                    vid: info.vid,
                    pid: info.pid,
                    serial_number: info.serial_number,
                }),
                _ => None,
            }
        })
}

/// Enumerates only the operating system's current serial-port list after an
/// active-capture transport failure. It intentionally does not open the port,
/// run Arduino CLI, change DTR/RTS, or invoke the 1200-bps reset path.
fn diagnose_port_enumeration(
    selected_port: &str,
    expected: Option<&UsbPortIdentity>,
) -> PortEnumerationDiagnostic {
    let ports = match serialport::available_ports() {
        Ok(ports) => ports,
        Err(error) => {
            return PortEnumerationDiagnostic {
                selected_port_present: None,
                same_vid_pid_present: None,
                same_serial_present: None,
                uno_r4_present: None,
                observed_vid: None,
                observed_pid: None,
                observed_serial_number: None,
                enumeration_error: Some(error.to_string()),
            };
        }
    };

    let selected_port_present = ports
        .iter()
        .any(|candidate| candidate.port_name.eq_ignore_ascii_case(selected_port));
    let identities: Vec<UsbPortIdentity> = ports
        .iter()
        .filter_map(|candidate| match &candidate.port_type {
            serialport::SerialPortType::UsbPort(info) => Some(UsbPortIdentity {
                port: candidate.port_name.clone(),
                vid: info.vid,
                pid: info.pid,
                serial_number: info.serial_number.clone(),
            }),
            _ => None,
        })
        .collect();
    let observed = identities
        .iter()
        .find(|candidate| candidate.port.eq_ignore_ascii_case(selected_port));
    let same_vid_pid_present = expected.map(|identity| {
        identities
            .iter()
            .any(|candidate| candidate.vid == identity.vid && candidate.pid == identity.pid)
    });
    let same_serial_present = expected.and_then(|identity| {
        identity.serial_number.as_ref().map(|serial| {
            identities
                .iter()
                .any(|candidate| candidate.serial_number.as_ref() == Some(serial))
        })
    });
    let uno_r4_present = identities
        .iter()
        .any(|candidate| candidate.vid == 0x2341 && matches!(candidate.pid, 0x1002 | 0x006d));

    PortEnumerationDiagnostic {
        selected_port_present: Some(selected_port_present),
        same_vid_pid_present,
        same_serial_present,
        uno_r4_present: Some(uno_r4_present),
        observed_vid: observed.map(|identity| identity.vid),
        observed_pid: observed.map(|identity| identity.pid),
        observed_serial_number: observed.and_then(|identity| identity.serial_number.clone()),
        enumeration_error: None,
    }
}

fn target_matches_port(target: &ResetTarget, candidate: &UnoUsbPort) -> bool {
    candidate.role == UnoUsbRole::Application
        && candidate.port.eq_ignore_ascii_case(&target.port)
        && target
            .serial_number
            .as_ref()
            .is_none_or(|serial| candidate.serial_number.as_ref() == Some(serial))
}

fn select_returning_port(target: &ResetTarget, candidates: &[UnoUsbPort]) -> ReturningPort {
    let candidates: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate.role == UnoUsbRole::Application)
        .cloned()
        .collect();
    let serial_matches: Vec<_> = target
        .serial_number
        .as_ref()
        .map(|serial| {
            candidates
                .iter()
                .filter(|candidate| candidate.serial_number.as_ref() == Some(serial))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    match serial_matches.as_slice() {
        [port] => return ReturningPort::Found(port.clone()),
        [] => {}
        _ => return ReturningPort::Ambiguous,
    }

    let port_matches: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate.port.eq_ignore_ascii_case(&target.port))
        .cloned()
        .collect();
    match port_matches.as_slice() {
        [port] => ReturningPort::Found(port.clone()),
        [] if target.serial_number.is_none() && candidates.len() == 1 => {
            ReturningPort::Found(candidates[0].clone())
        }
        [] => ReturningPort::Absent,
        _ => ReturningPort::Ambiguous,
    }
}

fn board_elapsed_seconds(snapshot: &AcquisitionSnapshot) -> f64 {
    if snapshot.sample_count > 1 && snapshot.measured_rate_hz > 0.0 {
        (snapshot.sample_count - 1) as f64 / snapshot.measured_rate_hz
    } else {
        0.0
    }
}

fn handshake_failure_category(
    snapshot: &AcquisitionSnapshot,
    bytes_received: u64,
) -> ConnectionFailureCategory {
    if snapshot.integrity.unsupported_versions > 0 {
        ConnectionFailureCategory::WrongProtocolVersion
    } else if snapshot.integrity.crc_failures > 0 {
        ConnectionFailureCategory::ProtocolCrcFailure
    } else if bytes_received == 0 {
        ConnectionFailureCategory::PortOpenNoBytes
    } else if snapshot.integrity.received_packets == 0 && snapshot.skipped_noise_bytes > 0 {
        ConnectionFailureCategory::NonProtocolBytes
    } else {
        ConnectionFailureCategory::HandshakeIncomplete
    }
}

fn firmware_identity_failure_category(
    snapshot: &AcquisitionSnapshot,
) -> Option<ConnectionFailureCategory> {
    match (snapshot.firmware_build, snapshot.firmware_board_id) {
        (Some(build), Some(device))
            if build == REFERENCE_FIRMWARE_BUILD && device == REFERENCE_DEVICE_ID =>
        {
            None
        }
        (None, _) | (_, None) => Some(ConnectionFailureCategory::MissingFirmwareIdentity),
        _ => Some(ConnectionFailureCategory::IncompatibleFirmwareIdentity),
    }
}

fn raw_byte_classification(snapshot: &AcquisitionSnapshot, bytes_received: u64) -> &'static str {
    if bytes_received == 0 {
        "no bytes received"
    } else if snapshot.integrity.received_packets > 0 {
        "validated WVU binary frames"
    } else if snapshot.integrity.crc_failures > 0 {
        "CRC-invalid protocol-like bytes"
    } else {
        "nonprotocol bytes"
    }
}

fn recommended_action(category: &ConnectionFailureCategory) -> &'static str {
    match category {
        ConnectionFailureCategory::PortBusy => {
            "Close the application using this port, then retry or refresh devices."
        }
        ConnectionFailureCategory::PortOpenNoBytes => {
            "Retry handshake. If the UNO remains silent, select Reset board and retry."
        }
        ConnectionFailureCategory::NonProtocolBytes => {
            "Refresh devices and confirm the selected port is the UNO R4 WiFi."
        }
        ConnectionFailureCategory::ProtocolCrcFailure => {
            "Retry handshake after refreshing devices; reset only if CRC-valid frames do not resume."
        }
        ConnectionFailureCategory::WrongProtocolVersion => {
            "The connected sketch is incompatible with protocol v0.1; contact the instructor before firmware recovery."
        }
        ConnectionFailureCategory::MissingFirmwareIdentity => {
            "WVU protocol frames arrived without the required firmware identity. Install the controlled reference firmware for this application version."
        }
        ConnectionFailureCategory::IncompatibleFirmwareIdentity => {
            "WVU protocol firmware responded, but its build or device identity is incompatible. Install the controlled reference firmware for this application version."
        }
        ConnectionFailureCategory::HandshakeIncomplete => {
            "Retry handshake. If identity/PONG stays incomplete, select Reset board and retry."
        }
        ConnectionFailureCategory::SerialTransportError => {
            "The serial connection reported an error, but the board was not proven removed. Refresh the board and review Advanced details before starting a new session."
        }
        ConnectionFailureCategory::FirmwareProtocolError => {
            "The Arduino reported that it stopped the recording. Verify the firmware, then refresh the board before starting a new session."
        }
        ConnectionFailureCategory::NoDataTimeout => {
            "The Arduino stopped sending samples. Refresh the board and verify the firmware before starting a new session."
        }
        ConnectionFailureCategory::DeviceDisconnected => {
            "Reconnect the UNO R4 WiFi, refresh devices, and explicitly start a new session."
        }
        ConnectionFailureCategory::ResetPortDidNotReturn => {
            "Wait for the UNO to re-enumerate, then refresh devices. Do not upload firmware automatically."
        }
        ConnectionFailureCategory::ResetReturnedDifferentPort => {
            "Use the rediscovered UNO port shown below, then explicitly start a new recording."
        }
        ConnectionFailureCategory::AmbiguousReturningDevice => {
            "Multiple UNO candidates appeared; refresh devices and select the board by serial number."
        }
    }
}

/// Deterministic device transport. Commands are decoded as normal frames and cause the
/// matching firmware responses, while sample batches are generated lazily.
struct SimulatorIo {
    bytes: VecDeque<u8>,
    commands: FrameParser,
    packet_sequence: u32,
    sample_sequence: u32,
    sample_limit: Option<u64>,
    active: bool,
    next_batch_at: Instant,
    batch_interval: Option<Duration>,
    max_fragment: usize,
    profile_category: String,
    requested_duration_seconds: Option<u64>,
    sample_period_us: u32,
    channel_count: u8,
    pulseox: bool,
    adc_resolution_bits: u8,
    output_mask: u8,
}

impl SimulatorIo {
    fn new(profile: &ProfileSnapshot, duration: RecordingDuration) -> Result<Self, SessionError> {
        let mut simulator = Self {
            bytes: VecDeque::new(),
            commands: FrameParser::default(),
            packet_sequence: 0,
            sample_sequence: 0,
            sample_limit: None,
            active: false,
            next_batch_at: Instant::now(),
            batch_interval: Some(Duration::from_millis(10)),
            max_fragment: 7,
            profile_category: profile.profile.category.clone(),
            requested_duration_seconds: duration.requested_seconds(),
            sample_period_us: 1_000,
            channel_count: 1,
            pulseox: false,
            adc_resolution_bits: profile.profile.acquisition.adc_resolution_bits,
            output_mask: 0,
        };
        simulator.queue(
            MessageType::Hello,
            vec![3, 0, 1, 0, 0x34, 0x4f, 0x4e, 0x55, 1, 14, 6, 0],
        )?;
        simulator.queue(
            MessageType::Capabilities,
            vec![
                12, 14, 6, 0x03, 0x07, 5, 100, 0, 200, 0, 250, 0, 244, 1, 232, 3,
            ],
        )?;
        Ok(simulator)
    }

    #[cfg(test)]
    fn new_accelerated(duration: RecordingDuration) -> Result<Self, SessionError> {
        let profile = default_general_profile()?;
        Self::new_accelerated_with_layout(&profile, duration, 1, 1_000, false)
    }

    #[cfg(test)]
    fn new_accelerated_with_layout(
        profile: &ProfileSnapshot,
        duration: RecordingDuration,
        field_count: u8,
        logical_rate_hz: u32,
        pulseox: bool,
    ) -> Result<Self, SessionError> {
        let mut simulator = Self::new(profile, duration)?;
        simulator.batch_interval = None;
        simulator.max_fragment = 128;
        simulator.channel_count = field_count;
        simulator.pulseox = pulseox;
        simulator.sample_period_us = 1_000_000 / logical_rate_hz;
        Ok(simulator)
    }

    fn queue(&mut self, message_type: MessageType, payload: Vec<u8>) -> Result<(), SessionError> {
        let frame = encode_frame(&Frame {
            message_type,
            flags: 0,
            sequence: self.packet_sequence,
            payload,
        })
        .map_err(|error| SessionError::Protocol(format!("{error:?}")))?;
        self.packet_sequence = self.packet_sequence.wrapping_add(1);
        self.bytes.extend(frame);
        Ok(())
    }

    fn queue_sample_batch(&mut self) -> Result<(), SessionError> {
        if self
            .sample_limit
            .is_some_and(|limit| u64::from(self.sample_sequence) >= limit)
        {
            return Ok(());
        }
        let count = self
            .sample_limit
            .map(|limit| {
                limit
                    .saturating_sub(u64::from(self.sample_sequence))
                    .min(10) as u32
            })
            .unwrap_or(10);
        let mut samples = Vec::with_capacity(count as usize * usize::from(self.channel_count));
        for index in 0..count {
            for field in 0..self.channel_count {
                samples.push(self.signal_counts(self.sample_sequence + index, usize::from(field)));
            }
        }
        let payload = SampleBatch {
            first_sample_sequence: self.sample_sequence,
            first_timestamp_us: u64::from(self.sample_sequence) * u64::from(self.sample_period_us),
            sample_period_us: self.sample_period_us,
            channel_count: self.channel_count,
            samples,
            status_flags: 1,
        }
        .to_payload()
        .map_err(|error| SessionError::Protocol(format!("{error:?}")))?;
        self.sample_sequence = self.sample_sequence.wrapping_add(count);
        self.queue(MessageType::SampleBatch, payload)
    }

    fn signal_counts(&self, sequence: u32, field: usize) -> u16 {
        let rate = 1_000_000.0 / f64::from(self.sample_period_us);
        let phase = sequence as f64 * std::f64::consts::TAU / rate;
        let volts = match self.profile_category.as_str() {
            "course_emg_force" => match field {
                0 => 2.5 + (phase * 70.0).sin() * 0.7,
                1 => 2.5 + (phase * 70.0).sin().abs() * 0.7,
                2 => 2.5 + (phase * 1.5).sin() * 0.35,
                _ => 2.5 + (phase * 0.2).sin() * 0.8,
            },
            "course_blood_pressure" => {
                // The deterministic simulator has an explicit MPXV-to-XGZP
                // relationship so students can exercise the linear-fit
                // workflow without calling it a physical sensor validation.
                let mpxv_volts = 2.5 + (phase * 0.08).sin() * 0.9;
                match field {
                    0 => 2.4 + (phase * 1.2).sin() * 0.35,
                    1 => mpxv_volts,
                    _ => {
                        let mpxv_mmhg = ((mpxv_volts / 5.0 - 0.04) / 0.009) * 7.5006;
                        // Synthetic mapping: MPXV mmHg = 120 * XGZP volts - 10.
                        ((mpxv_mmhg + 10.0) / 120.0).clamp(0.0, 5.0)
                    }
                }
            }
            "course_pulseox" => {
                let state = field % 4;
                let dark = matches!(state, 1 | 3);
                let base = if field < 4 { 2.1 } else { 2.8 };
                if dark {
                    0.15
                } else if state == 0 {
                    base + 0.25
                } else {
                    base + 0.4
                }
            }
            _ => 2.5 + (phase * 2.0).sin() * 1.1,
        };
        let full_scale = f64::from((1u32 << self.adc_resolution_bits) - 1);
        (volts * full_scale / 5.0).round().clamp(0.0, full_scale) as u16
    }
}

impl Read for SimulatorIo {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.bytes.is_empty() && self.active {
            if let Some(interval) = self.batch_interval {
                let now = Instant::now();
                if now < self.next_batch_at {
                    // Match the blocking behavior of a serial read. This keeps an
                    // indefinitely running simulator from busy-spinning between
                    // deterministic 10 ms sample batches.
                    thread::sleep((self.next_batch_at - now).min(interval));
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "simulator waiting for next sample batch",
                    ));
                }
            }
        }
        if self.bytes.is_empty()
            && self.active
            && self
                .sample_limit
                .is_none_or(|limit| u64::from(self.sample_sequence) < limit)
            && self
                .batch_interval
                .is_none_or(|_| Instant::now() >= self.next_batch_at)
        {
            self.queue_sample_batch()
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            if let Some(interval) = self.batch_interval {
                self.next_batch_at += interval;
            }
        }
        if self.bytes.is_empty() {
            if !self.active && self.batch_interval.is_some() {
                // A CDC port blocks while a board is in its startup grace; the
                // simulator mirrors that behavior instead of busy-spinning.
                thread::sleep(Duration::from_millis(5));
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "simulator idle",
            ));
        }
        let take = self.bytes.len().min(buffer.len()).min(self.max_fragment);
        for destination in &mut buffer[..take] {
            *destination = self.bytes.pop_front().unwrap_or_default();
        }
        Ok(take)
    }
}

impl Write for SimulatorIo {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        for command in self.commands.push(buffer) {
            match command.message_type {
                MessageType::Ping => self.queue(MessageType::Pong, vec![]),
                MessageType::Configure => {
                    let payload = &command.payload;
                    if payload.len() < 8 {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "invalid simulator configure payload",
                        ));
                    }
                    match payload[0] {
                        0 => {
                            let channel_count = payload[6];
                            if channel_count == 0
                                || channel_count > 6
                                || payload.len() != 8 + usize::from(channel_count)
                            {
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::InvalidInput,
                                    "invalid simulator simultaneous channel layout",
                                ));
                            }
                            self.adc_resolution_bits = payload[1];
                            self.pulseox = false;
                            self.sample_period_us = 1_000_000
                                / u32::from_le_bytes([
                                    payload[2], payload[3], payload[4], payload[5],
                                ]);
                            self.channel_count = channel_count;
                            self.output_mask = payload[7 + usize::from(self.channel_count)] & 0x07;
                        }
                        1 => {
                            if payload.len() != 11 {
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::InvalidInput,
                                    "invalid simulator pulse-ox layout",
                                ));
                            }
                            self.adc_resolution_bits = payload[1];
                            self.pulseox = true;
                            self.sample_period_us = u32::from_le_bytes([
                                payload[2], payload[3], payload[4], payload[5],
                            ])
                            .saturating_mul(4);
                            self.channel_count = 8;
                            self.output_mask = 0;
                        }
                        _ => {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                "unsupported simulator acquisition mode",
                            ));
                        }
                    }
                    if self.sample_period_us == 0 || self.channel_count == 0 {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "invalid simulator acquisition rate or channels",
                        ));
                    }
                    self.sample_limit = self.requested_duration_seconds.map(|seconds| {
                        seconds.saturating_mul(1_000_000 / u64::from(self.sample_period_us))
                    });
                    self.batch_interval = Some(Duration::from_micros(
                        u64::from(self.sample_period_us).saturating_mul(10),
                    ));
                    self.queue(MessageType::ConfigAck, vec![])
                }
                MessageType::Start => {
                    self.active = true;
                    self.next_batch_at = Instant::now();
                    self.queue(MessageType::Status, vec![1, self.output_mask])
                }
                MessageType::Stop => {
                    self.active = false;
                    self.output_mask = 0;
                    self.queue(MessageType::Status, vec![0, self.output_mask])
                }
                _ => Ok(()),
            }
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::{BmegReader, BmegWriter};
    use std::sync::atomic::AtomicU64;
    use tempfile::tempdir;

    #[test]
    fn simulator_complete_session_is_bounded_and_recorded() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        let summary = session
            .capture_simulator_for_test(2, dir.path())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(summary.samples, 2_000);
        assert_eq!(summary.integrity.crc_failures, 0);
        assert_eq!(summary.integrity.missing_packet_sequences, 0);
        assert_eq!(session.recent_samples().unwrap_or_default().len(), 2_000);
        assert!(BmegReader::open(Path::new(&summary.bmeg_path)).is_ok());
        assert_eq!(
            std::fs::read_to_string(&summary.csv_path)
                .unwrap_or_default()
                .lines()
                .count(),
            2_001
        );
    }

    fn display_records(rate_hz: u64, seconds: u64) -> VecDeque<SynchronizedRecord> {
        let count = rate_hz * seconds;
        (0..count)
            .map(|sequence| SynchronizedRecord {
                sequence: sequence as u32,
                timestamp_us: sequence * 1_000_000 / rate_hz,
                status_flags: 0,
                counts: vec![sequence as u16],
            })
            .collect()
    }

    #[test]
    fn display_history_is_bounded_to_thirty_seconds_at_maximum_rate() {
        let mut recent = VecDeque::with_capacity(DISPLAY_CAPACITY);
        for sequence in 0..=DISPLAY_CAPACITY {
            if recent.len() == DISPLAY_CAPACITY {
                recent.pop_front();
            }
            recent.push_back(SynchronizedRecord {
                sequence: sequence as u32,
                timestamp_us: sequence as u64 * 1_000,
                status_flags: 0,
                counts: vec![0],
            });
        }
        assert_eq!(recent.len(), DISPLAY_CAPACITY);
        assert_eq!(recent.front().map(|record| record.sequence), Some(1));
        assert_eq!(
            recent.back().map(|record| record.sequence),
            Some(DISPLAY_CAPACITY as u32)
        );
    }

    #[test]
    fn display_windows_use_timestamps_at_every_supported_rate() {
        for rate_hz in [100, 250, 500, 1_000] {
            let records = display_records(rate_hz, 10);
            let selected = select_display_records(&records, 5.0, MAX_DISPLAY_RENDER_POINTS);
            let latest = records
                .back()
                .unwrap_or_else(|| panic!("missing display record"));
            assert!(selected.iter().all(|record| {
                record.timestamp_us >= latest.timestamp_us.saturating_sub(5_000_000)
            }));
            assert_eq!(selected.last(), Some(latest));
            assert!(selected
                .windows(2)
                .all(|pair| pair[0].timestamp_us <= pair[1].timestamp_us));
        }
    }

    #[test]
    fn display_decimation_is_bounded_and_keeps_the_exact_newest_record() {
        let records = display_records(1_000, 30);
        let selected = select_display_records(&records, 30.0, 2_000);
        assert_eq!(selected.len(), 2_000);
        assert_eq!(selected.first(), records.front());
        assert_eq!(selected.last(), records.back());
        assert!(selected
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence));
    }

    #[test]
    fn display_queries_do_not_mutate_session_or_integrity_state() {
        let session = SessionController::default();
        {
            let mut runtime = session
                .lock_runtime()
                .unwrap_or_else(|error| panic!("{error}"));
            runtime.state = SessionState::Acquiring;
            runtime.samples = 42;
            runtime.integrity.host_channel_overflows = 3;
            runtime.recent = display_records(1_000, 10);
        }
        let before = session.status().unwrap_or_else(|error| panic!("{error}"));
        let selected = session
            .recent_display_samples(5.0, 500)
            .unwrap_or_else(|error| panic!("{error}"));
        let after = session.status().unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(selected.len(), 500);
        assert_eq!(before.state, after.state);
        assert_eq!(before.samples, after.samples);
        assert_eq!(
            before.integrity.host_channel_overflows,
            after.integrity.host_channel_overflows
        );
    }

    #[test]
    fn transitions_are_explicit_and_idempotent() {
        let session = SessionController::default();
        assert!(session.request_stop().is_ok());
        assert!(session.disconnect().is_ok());
        assert!(session.disconnect().is_ok());
        assert_eq!(
            session.status().unwrap_or_else(|e| panic!("{e}")).state,
            SessionState::Disconnected
        );
    }

    #[test]
    fn faulted_handshake_does_not_own_the_serial_session_for_recovery() {
        let session = SessionController::default();
        session
            .set_fault("firmware did not respond".into())
            .unwrap_or_else(|error| panic!("{error}"));

        assert!(!session
            .is_recording()
            .unwrap_or_else(|error| panic!("{error}")));
    }

    #[test]
    fn successful_recovery_normalizes_an_idle_fault_before_the_next_recording() {
        let session = SessionController::default();
        session
            .set_fault("firmware did not respond".into())
            .unwrap_or_else(|error| panic!("{error}"));

        let status = session
            .prepare_for_new_recording()
            .unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(status.state, SessionState::Disconnected);
        assert!(status.last_error.is_none());
        assert!(status.connection_diagnostics.is_none());
    }

    #[test]
    fn recovery_normalization_never_interrupts_an_active_session() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let session = SessionController::default();
        session
            .start_simulator(RecordingDuration::Timed { seconds: 10 }, dir.path().into())
            .unwrap_or_else(|error| panic!("{error}"));

        assert!(matches!(
            session.prepare_for_new_recording(),
            Err(SessionError::State(_))
        ));
        session
            .disconnect()
            .unwrap_or_else(|error| panic!("{error}"));
    }

    fn uno(port: &str, serial: Option<&str>) -> UnoUsbPort {
        UnoUsbPort {
            port: port.into(),
            serial_number: serial.map(str::to_owned),
            role: UnoUsbRole::Application,
        }
    }

    fn target() -> ResetTarget {
        ResetTarget {
            port: "COM12".into(),
            serial_number: Some("48CA4360243C".into()),
        }
    }

    #[test]
    fn reset_matching_prefers_serial_and_handles_same_or_changed_port() {
        assert_eq!(
            select_returning_port(&target(), &[uno("COM12", Some("48CA4360243C"))]),
            ReturningPort::Found(uno("COM12", Some("48CA4360243C")))
        );
        assert_eq!(
            select_returning_port(&target(), &[uno("COM15", Some("48CA4360243C"))]),
            ReturningPort::Found(uno("COM15", Some("48CA4360243C")))
        );
    }

    #[test]
    fn reset_matching_ignores_unknown_delayed_and_ambiguous_candidates() {
        let unrelated = UnoUsbPort {
            port: "COM3".into(),
            serial_number: None,
            role: UnoUsbRole::Application,
        };
        assert_eq!(
            select_returning_port(&target(), &[unrelated]),
            ReturningPort::Absent
        );
        assert_eq!(select_returning_port(&target(), &[]), ReturningPort::Absent);
        assert_eq!(
            select_returning_port(
                &target(),
                &[
                    uno("COM14", Some("48CA4360243C")),
                    uno("COM15", Some("48CA4360243C")),
                ],
            ),
            ReturningPort::Ambiguous
        );
        assert_eq!(
            select_returning_port(
                &target(),
                &[UnoUsbPort {
                    port: "COM15".into(),
                    serial_number: Some("48CA4360243C".into()),
                    role: UnoUsbRole::Bootloader,
                }],
            ),
            ReturningPort::Absent
        );
    }

    struct HandshakeMock {
        bytes: VecDeque<u8>,
        commands: FrameParser,
        answer_on_ping: Option<u32>,
        pings: u32,
        hello_payload: Vec<u8>,
    }

    impl HandshakeMock {
        fn new(answer_on_ping: Option<u32>) -> Self {
            Self {
                bytes: VecDeque::new(),
                commands: FrameParser::default(),
                answer_on_ping,
                pings: 0,
                hello_payload: vec![3, 0, 1, 0, 0x34, 0x4f, 0x4e, 0x55, 1, 14, 6, 0],
            }
        }

        fn queue_identity(&mut self) -> std::io::Result<()> {
            for (sequence, message_type, payload) in [
                (0, MessageType::Hello, self.hello_payload.clone()),
                (1, MessageType::Capabilities, vec![12, 1, 6, 0]),
                (2, MessageType::Pong, vec![]),
            ] {
                let bytes = encode_frame(&Frame {
                    message_type,
                    flags: 0,
                    sequence,
                    payload,
                })
                .map_err(|error| std::io::Error::other(format!("{error:?}")))?;
                self.bytes.extend(bytes);
            }
            Ok(())
        }
    }

    impl Read for HandshakeMock {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.bytes.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "mock idle",
                ));
            }
            let count = self.bytes.len().min(buffer.len()).min(9);
            for byte in &mut buffer[..count] {
                *byte = self.bytes.pop_front().unwrap_or_default();
            }
            Ok(count)
        }
    }

    impl Write for HandshakeMock {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            for frame in self.commands.push(bytes) {
                if frame.message_type == MessageType::Ping {
                    self.pings += 1;
                    if self.answer_on_ping == Some(self.pings) {
                        self.queue_identity()?;
                    }
                }
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn handshake_session() -> (SessionController, AcquisitionController) {
        let session = SessionController::default();
        session
            .begin_session(
                false,
                "Arduino UNO R4 WiFi",
                "COM12",
                RecordingDuration::Timed { seconds: 10 },
                default_general_profile().unwrap_or_else(|e| panic!("{e}")),
                RecordingCalibration::default(),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        session
            .set_state(SessionState::Connected)
            .unwrap_or_else(|e| panic!("{e}"));
        session
            .mark_port_opened("COM12")
            .unwrap_or_else(|e| panic!("{e}"));
        let (sender, _receiver) = sync_channel(16);
        (session, AcquisitionController::new(sender))
    }

    #[test]
    fn handshake_succeeds_after_a_bounded_ping_retry() {
        let (session, mut acquisition) = handshake_session();
        let mut io = HandshakeMock::new(Some(2));
        session
            .wait_for_handshake_with_policy(
                &mut io,
                &mut acquisition,
                Duration::ZERO,
                Duration::from_millis(100),
                3,
                Duration::from_millis(25),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        let diagnostics = session
            .status()
            .unwrap_or_else(|e| panic!("{e}"))
            .connection_diagnostics
            .unwrap_or_else(|| panic!("missing diagnostics"));
        assert_eq!(diagnostics.ping_attempts, 2);
        assert!(diagnostics.hello_received);
        assert!(diagnostics.capabilities_received);
        assert!(diagnostics.pong_received);
        assert_eq!(diagnostics.failure_category, None);
    }

    #[test]
    fn silent_handshake_reports_port_open_no_bytes() {
        let (session, mut acquisition) = handshake_session();
        let mut io = HandshakeMock::new(None);
        assert!(session
            .wait_for_handshake_with_policy(
                &mut io,
                &mut acquisition,
                Duration::ZERO,
                Duration::from_millis(15),
                2,
                Duration::from_millis(1),
            )
            .is_err());
        let diagnostics = session
            .status()
            .unwrap_or_else(|e| panic!("{e}"))
            .connection_diagnostics
            .unwrap_or_else(|| panic!("missing diagnostics"));
        assert_eq!(
            diagnostics.failure_category,
            Some(ConnectionFailureCategory::PortOpenNoBytes)
        );
        assert_eq!(diagnostics.bytes_received, 0);
    }

    #[test]
    fn handshake_rejects_an_incompatible_firmware_identity() {
        let (session, mut acquisition) = handshake_session();
        let mut io = HandshakeMock::new(Some(1));
        io.hello_payload[0] = 4;
        assert!(session
            .wait_for_handshake_with_policy(
                &mut io,
                &mut acquisition,
                Duration::ZERO,
                Duration::from_millis(100),
                1,
                Duration::from_millis(1),
            )
            .is_err());
        let diagnostics = session
            .status()
            .unwrap_or_else(|e| panic!("{e}"))
            .connection_diagnostics
            .unwrap_or_else(|| panic!("missing diagnostics"));
        assert_eq!(
            diagnostics.failure_category,
            Some(ConnectionFailureCategory::IncompatibleFirmwareIdentity)
        );
        assert_eq!(
            diagnostics.raw_byte_classification,
            "validated WVU binary frames"
        );
    }

    #[test]
    fn reset_is_prohibited_while_acquiring() {
        let (session, _acquisition) = handshake_session();
        session
            .set_state(SessionState::Acquiring)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            session.reset_and_retry(target()),
            Err(SessionError::State(_))
        ));
    }

    #[test]
    fn until_stopped_never_uses_a_hidden_timer_and_finalizes_on_manual_stop() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        session
            .start_simulator(RecordingDuration::UntilStopped, dir.path().to_path_buf())
            .unwrap_or_else(|e| panic!("{e}"));
        let deadline = Instant::now() + Duration::from_secs(2);
        while session.status().unwrap_or_else(|e| panic!("{e}")).state != SessionState::Acquiring
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(5));
        }
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Acquiring);
        assert!(status.remaining_seconds.is_none());
        thread::sleep(Duration::from_millis(30));
        session.request_stop().unwrap_or_else(|e| panic!("{e}"));
        session.wait_for_worker().unwrap_or_else(|e| panic!("{e}"));
        let summary = session
            .status()
            .unwrap_or_else(|e| panic!("{e}"))
            .last_summary
            .unwrap_or_else(|| panic!("missing summary"));
        assert_eq!(summary.duration, RecordingDuration::UntilStopped);
        assert_eq!(summary.stop_reason, StopReason::User);
        assert_eq!(summary.completion_status, "complete");
        assert!(summary.samples > 0);
    }

    #[test]
    fn finalized_status_keeps_capture_elapsed_time_stable() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let session = SessionController::default();
        session
            .start_simulator(
                RecordingDuration::Timed { seconds: 10 },
                dir.path().to_path_buf(),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        let deadline = Instant::now() + Duration::from_secs(2);
        while session
            .status()
            .unwrap_or_else(|error| panic!("{error}"))
            .state
            != SessionState::Acquiring
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(5));
        }
        session
            .request_stop()
            .unwrap_or_else(|error| panic!("{error}"));
        session
            .wait_for_worker()
            .unwrap_or_else(|error| panic!("{error}"));
        let summary = session
            .status()
            .unwrap_or_else(|error| panic!("{error}"))
            .last_summary
            .unwrap_or_else(|| panic!("missing finalized summary"));
        thread::sleep(Duration::from_millis(50));
        let status = session.status().unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(status.state, SessionState::Disconnected);
        assert!((status.elapsed_seconds - summary.host_elapsed_seconds).abs() < 0.000_001);
    }

    #[test]
    fn first_stop_reason_wins_a_timed_manual_stop_race() {
        let session = SessionController::default();
        assert_eq!(
            session
                .set_stop_reason_once(StopReason::TimedComplete)
                .unwrap_or_else(|e| panic!("{e}")),
            StopReason::TimedComplete
        );
        session.request_stop().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            session.stop_reason().unwrap_or_else(|e| panic!("{e}")),
            Some(StopReason::TimedComplete)
        );
    }

    struct FixedDiskSpace(AtomicU64);

    impl DiskSpaceProvider for FixedDiskSpace {
        fn available_space(&self, _path: &Path) -> std::io::Result<u64> {
            Ok(self.0.load(Ordering::Relaxed))
        }
    }

    #[test]
    fn critical_low_storage_refuses_a_session_before_starting() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let disk = Arc::new(FixedDiskSpace(AtomicU64::new(DISK_CRITICAL_BYTES - 1)));
        let session = SessionController::with_disk_space(disk);
        let result = session.capture_simulator_for_test(1, dir.path());
        assert!(matches!(result, Err(SessionError::Storage(_))));
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.available_disk_bytes, Some(DISK_CRITICAL_BYTES - 1));
        assert!(status.storage_warning.is_some());
    }

    #[test]
    fn accelerated_fifteen_minute_equivalent_simulator_soak_preserves_bounds() {
        const SAMPLES: u64 = 15 * 60 * 1_000;
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        let duration = RecordingDuration::Timed { seconds: 15 * 60 };
        let bmeg = dir.path().join("soak.bmeg");
        let metadata = session
            .initial_metadata(InitialMetadataRequest {
                simulator: true,
                source: "SIM",
                bmeg: &bmeg,
                duration: &duration,
                profile: &default_general_profile().unwrap_or_else(|e| panic!("{e}")),
                calibration: RecordingCalibration::default(),
                recording_path_context: None,
            })
            .unwrap_or_else(|e| panic!("{e}"));
        let mut writer =
            BmegWriter::create_synchronized(&bmeg, &metadata, 1).unwrap_or_else(|e| panic!("{e}"));
        let (sender, receiver) = sync_channel(4_096);
        let mut acquisition = AcquisitionController::new(sender);
        acquisition.configure().unwrap_or_else(|e| panic!("{e}"));
        acquisition.start().unwrap_or_else(|e| panic!("{e}"));
        let mut simulator =
            SimulatorIo::new_accelerated(duration).unwrap_or_else(|e| panic!("{e}"));
        simulator.active = true;
        let mut recent = VecDeque::with_capacity(DISPLAY_CAPACITY);
        let mut buffer = [0u8; 512];

        while acquisition.sample_count < SAMPLES {
            let received = simulator
                .read(&mut buffer)
                .unwrap_or_else(|e| panic!("{e}"));
            acquisition.ingest_bytes(&buffer[..received]);
            for sample in receiver.try_iter() {
                writer
                    .write_record(&sample)
                    .unwrap_or_else(|e| panic!("{e}"));
                if recent.len() == DISPLAY_CAPACITY {
                    recent.pop_front();
                }
                recent.push_back(sample);
            }
        }
        writer.flush().unwrap_or_else(|e| panic!("{e}"));
        writer.finish().unwrap_or_else(|e| panic!("{e}"));
        let snapshot = acquisition.snapshot();
        assert_eq!(snapshot.sample_count, SAMPLES);
        assert_eq!(snapshot.integrity.crc_failures, 0);
        assert_eq!(snapshot.integrity.missing_packet_sequences, 0);
        assert_eq!(snapshot.integrity.missing_sample_sequences, 0);
        assert_eq!(snapshot.integrity.host_channel_overflows, 0);
        assert_eq!(recent.len(), DISPLAY_CAPACITY);
        assert!(BmegReader::open(&bmeg).is_ok());
    }

    fn accelerated_multifield_soak(
        profile: ProfileSnapshot,
        fields: u8,
        rate_hz: u32,
        records: u64,
    ) {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        let duration = RecordingDuration::Timed {
            seconds: records / u64::from(rate_hz),
        };
        let bmeg = dir.path().join("multifield-soak.bmeg");
        let metadata = session
            .initial_metadata(InitialMetadataRequest {
                simulator: true,
                source: "SIM",
                bmeg: &bmeg,
                duration: &duration,
                profile: &profile,
                calibration: RecordingCalibration::default(),
                recording_path_context: None,
            })
            .unwrap_or_else(|e| panic!("{e}"));
        let mut writer = BmegWriter::create_synchronized(&bmeg, &metadata, fields as usize)
            .unwrap_or_else(|e| panic!("{e}"));
        let (sender, receiver) = sync_channel(4_096);
        let mut acquisition = AcquisitionController::new(sender);
        acquisition.configure().unwrap_or_else(|e| panic!("{e}"));
        acquisition.start().unwrap_or_else(|e| panic!("{e}"));
        let mut simulator = SimulatorIo::new_accelerated_with_layout(
            &profile,
            duration,
            fields,
            rate_hz,
            fields == 8,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        simulator.active = true;
        let mut recent = VecDeque::with_capacity(DISPLAY_CAPACITY);
        let mut buffer = [0u8; 1024];
        while acquisition.sample_count < records {
            let received = simulator
                .read(&mut buffer)
                .unwrap_or_else(|e| panic!("{e}"));
            acquisition.ingest_bytes(&buffer[..received]);
            for record in receiver.try_iter() {
                assert_eq!(record.counts.len(), fields as usize);
                writer
                    .write_record(&record)
                    .unwrap_or_else(|e| panic!("{e}"));
                if recent.len() == DISPLAY_CAPACITY {
                    recent.pop_front();
                }
                recent.push_back(record);
            }
        }
        writer.finish().unwrap_or_else(|e| panic!("{e}"));
        let snapshot = acquisition.snapshot();
        assert_eq!(snapshot.sample_count, records);
        assert_eq!(snapshot.integrity.crc_failures, 0);
        assert_eq!(snapshot.integrity.missing_packet_sequences, 0);
        assert_eq!(snapshot.integrity.missing_sample_sequences, 0);
        assert_eq!(snapshot.integrity.host_channel_overflows, 0);
        assert_eq!(recent.len(), DISPLAY_CAPACITY);
        assert!(BmegReader::open(&bmeg).is_ok());
    }

    #[test]
    fn accelerated_six_channel_ten_minute_equivalent_soak_preserves_bounds() {
        let profile = default_general_profile().unwrap_or_else(|e| panic!("{e}"));
        accelerated_multifield_soak(profile, 6, 1_000, 10 * 60 * 1_000);
    }

    #[test]
    fn accelerated_pulseox_ten_minute_equivalent_soak_preserves_raw_cycles() {
        let profile = built_in_profiles()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .find(|candidate| candidate.category == "course_pulseox")
            .unwrap_or_else(|| panic!("missing pulse-ox course profile"))
            .snapshot(false);
        accelerated_multifield_soak(profile, 8, 250, 10 * 60 * 250);
    }

    #[test]
    fn duplicate_start_is_rejected_while_worker_runs() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        session
            .start_simulator(
                RecordingDuration::Timed { seconds: 10 },
                dir.path().to_path_buf(),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(session
            .start_simulator(
                RecordingDuration::Timed { seconds: 10 },
                dir.path().to_path_buf()
            )
            .is_err());
        session.request_stop().unwrap_or_else(|e| panic!("{e}"));
        session.wait_for_worker().unwrap_or_else(|e| panic!("{e}"));
    }

    #[test]
    fn repeated_simulator_sessions_allocate_distinct_recordings() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        let first = session
            .capture_simulator_for_test(1, dir.path())
            .unwrap_or_else(|e| panic!("{e}"));
        let second = session
            .capture_simulator_for_test(1, dir.path())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_ne!(first.bmeg_path, second.bmeg_path);
        assert_eq!(second.integrity.crc_failures, 0);
    }

    struct DisconnectingSimulator {
        inner: SimulatorIo,
        reads: usize,
        fail_after: usize,
    }

    impl Read for DisconnectingSimulator {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            if self.reads > self.fail_after {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "mock cable removed",
                ));
            }
            self.inner.read(buffer)
        }
    }

    impl Write for DisconnectingSimulator {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.inner.write(buffer)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }

    struct FirmwareStoppingSimulator {
        inner: SimulatorIo,
        error_sent: bool,
    }

    impl Read for FirmwareStoppingSimulator {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.inner.active && self.inner.sample_sequence >= 20 && !self.error_sent {
                self.inner.active = false;
                let _ = self.inner.queue(MessageType::ErrorMessage, vec![7]);
                self.error_sent = true;
            }
            self.inner.read(buffer)
        }
    }

    impl Write for FirmwareStoppingSimulator {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.inner.write(buffer)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }

    #[test]
    fn terminal_transport_error_finalizes_a_readable_faulted_recording_without_claiming_removal() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        session
            .begin_session(
                true,
                "Simulator",
                "SIM",
                RecordingDuration::Timed { seconds: 2 },
                default_general_profile().unwrap_or_else(|e| panic!("{e}")),
                RecordingCalibration::default(),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        session
            .set_state(SessionState::Connected)
            .unwrap_or_else(|e| panic!("{e}"));
        let mut transport = DisconnectingSimulator {
            inner: SimulatorIo::new(
                &default_general_profile().unwrap_or_else(|e| panic!("{e}")),
                RecordingDuration::Timed { seconds: 2 },
            )
            .unwrap_or_else(|e| panic!("{e}")),
            reads: 0,
            fail_after: 300,
        };
        let capture_error = session
            .capture_transport(
                &mut transport,
                CaptureRequest {
                    simulator: true,
                    source: "SIM".into(),
                    profile: default_general_profile().unwrap_or_else(|e| panic!("{e}")),
                    duration: RecordingDuration::Timed { seconds: 2 },
                    output_dir: dir.path().to_path_buf(),
                    calibration: RecordingCalibration::default(),
                    recording_path_context: None,
                    expected_port_identity: None,
                },
            )
            .err()
            .unwrap_or_else(|| panic!("disconnecting transport unexpectedly completed"));
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        let summary = status.last_summary.unwrap_or_else(|| {
            panic!(
                "missing interrupted summary after {capture_error}; state={:?}, reads={}",
                status.state, transport.reads
            )
        });
        assert_eq!(summary.recording_status, "faulted");
        assert_eq!(summary.integrity.disconnect_events, 0);
        assert!(summary
            .error
            .as_deref()
            .is_some_and(|detail| detail.contains("SERIAL_READ [SerialReadError]")));
        let diagnostics = status
            .connection_diagnostics
            .unwrap_or_else(|| panic!("missing serial failure diagnostics"));
        assert_eq!(
            diagnostics.terminal_error_stage.as_deref(),
            Some("SERIAL_READ")
        );
        assert_eq!(
            diagnostics.terminal_error_classification.as_deref(),
            Some("SerialReadError")
        );
        assert_eq!(
            diagnostics.terminal_error_kind.as_deref(),
            Some("ConnectionAborted")
        );
        assert_eq!(
            diagnostics.failure_category,
            Some(ConnectionFailureCategory::SerialTransportError)
        );
        assert!(BmegReader::open(Path::new(&summary.bmeg_path)).is_ok());
    }

    #[test]
    fn firmware_error_message_ends_an_active_session_instead_of_leaving_it_acquiring() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let profile = default_general_profile().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        session
            .begin_session(
                true,
                "Simulator",
                "SIM",
                RecordingDuration::UntilStopped,
                profile.clone(),
                RecordingCalibration::default(),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        session
            .set_state(SessionState::Connected)
            .unwrap_or_else(|e| panic!("{e}"));
        let mut transport = FirmwareStoppingSimulator {
            inner: SimulatorIo::new(&profile, RecordingDuration::UntilStopped)
                .unwrap_or_else(|e| panic!("{e}")),
            error_sent: false,
        };
        let error = session
            .capture_transport(
                &mut transport,
                CaptureRequest {
                    simulator: true,
                    source: "SIM".into(),
                    profile,
                    duration: RecordingDuration::UntilStopped,
                    output_dir: dir.path().to_path_buf(),
                    calibration: RecordingCalibration::default(),
                    recording_path_context: None,
                    expected_port_identity: None,
                },
            )
            .err()
            .unwrap_or_else(|| panic!("firmware error unexpectedly continued recording"));
        assert!(error.to_string().contains("host-command watchdog expired"));
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Faulted);
        let diagnostics = status
            .connection_diagnostics
            .unwrap_or_else(|| panic!("missing firmware failure diagnostics"));
        assert_eq!(
            diagnostics.failure_category,
            Some(ConnectionFailureCategory::FirmwareProtocolError)
        );
        assert_eq!(
            diagnostics.terminal_error_stage.as_deref(),
            Some("FIRMWARE_ERROR")
        );
        assert_eq!(
            status
                .last_summary
                .as_ref()
                .map(|summary| summary.stop_reason),
            Some(StopReason::Fault)
        );
    }

    struct FailingCommandWriter {
        fail_write: bool,
        fail_flush: bool,
    }

    impl Write for FailingCommandWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            if self.fail_write {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "mock PING write failure",
                ));
            }
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            if self.fail_flush {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "mock PING flush failure",
                ));
            }
            Ok(())
        }
    }

    #[test]
    fn periodic_ping_diagnostics_distinguish_write_and_flush_failures() {
        let session = SessionController::default();
        let mut write_failure = FailingCommandWriter {
            fail_write: true,
            fail_flush: false,
        };
        let write_error = session
            .send_capture_command(&mut write_failure, MessageType::Ping, 3, vec![])
            .err()
            .unwrap_or_else(|| panic!("expected PING write failure"));
        match write_error {
            CaptureCommandError::Serial(error) => {
                assert!(matches!(error.stage, SerialFailureStage::PingWrite));
                assert_eq!(error.stage.classification(), "PingWriteError");
                assert_eq!(error.error.kind(), std::io::ErrorKind::BrokenPipe);
            }
            CaptureCommandError::Protocol(detail) => panic!("unexpected protocol error: {detail}"),
        }

        let mut flush_failure = FailingCommandWriter {
            fail_write: false,
            fail_flush: true,
        };
        let flush_error = session
            .send_capture_command(&mut flush_failure, MessageType::Ping, 3, vec![])
            .err()
            .unwrap_or_else(|| panic!("expected PING flush failure"));
        match flush_error {
            CaptureCommandError::Serial(error) => {
                assert!(matches!(error.stage, SerialFailureStage::PingFlush));
                assert_eq!(error.stage.classification(), "PingWriteError");
                assert_eq!(error.error.kind(), std::io::ErrorKind::ConnectionReset);
            }
            CaptureCommandError::Protocol(detail) => panic!("unexpected protocol error: {detail}"),
        }
    }

    #[test]
    fn inactive_sample_stream_becomes_a_bounded_no_data_fault() {
        let now = Instant::now();
        let started = now
            .checked_sub(SAMPLE_STREAM_STALL_TIMEOUT + Duration::from_millis(1))
            .unwrap_or(now);
        let failure = sample_stream_stall_failure(&CaptureProgress::default(), started, now)
            .unwrap_or_else(|| panic!("expected a no-data timeout"));
        assert!(matches!(failure.stage, SerialFailureStage::NoDataTimeout));
        assert_eq!(failure.stage.classification(), "NoDataTimeout");
        assert_eq!(failure.error.kind(), std::io::ErrorKind::TimedOut);
        assert!(failure
            .error
            .to_string()
            .contains("no synchronized sample batch"));
    }

    #[test]
    fn firmware_watchdog_error_is_reported_as_a_protocol_failure() {
        let detail = firmware_error_detail(Some(7), None);
        let failure = SerialTerminalFailure::new(
            SerialFailureStage::FirmwareError,
            std::io::Error::other(detail),
        );
        assert_eq!(failure.stage.classification(), "ProtocolFailure");
        assert!(failure
            .error
            .to_string()
            .contains("host-command watchdog expired"));
    }

    #[test]
    fn adc_timeout_error_retains_firmware_diagnostic_counters() {
        let payload = [8, 4, 3, 20, 44, 0, 0, 0, 43, 0, 0, 0, 1, 0, 0, 0];
        let detail = firmware_error_detail(Some(8), Some(&payload));
        assert!(detail.contains("ADC conversion deadline expired"));
        assert!(detail.contains("stage=4"));
        assert!(detail.contains("channel=A3"));
        assert!(detail.contains("fsp_error=20"));
        assert!(detail.contains("adc_starts=44"));
        assert!(detail.contains("adc_completions=43"));
        assert!(detail.contains("adc_timeouts=1"));
    }

    #[test]
    fn active_serial_timeout_allows_windows_write_jitter_without_threatening_keepalive() {
        assert!(ACTIVE_SERIAL_IO_TIMEOUT > Duration::from_millis(25));
        assert!(ACTIVE_SERIAL_IO_TIMEOUT < Duration::from_secs(1));
    }

    #[test]
    fn transient_windows_ping_timeout_with_current_samples_is_deferred() {
        let now = Instant::now();
        let failure = SerialTerminalFailure::new(
            SerialFailureStage::PingWrite,
            std::io::Error::from_raw_os_error(121),
        );
        assert_eq!(failure.error.kind(), std::io::ErrorKind::TimedOut);
        assert!(may_defer_ping_failure(
            &failure,
            now,
            now.checked_sub(Duration::from_secs(1)).unwrap_or(now),
            now.checked_sub(Duration::from_millis(500)),
            1,
        ));
    }

    #[test]
    fn ping_timeout_is_not_deferred_without_keepalive_margin_or_current_samples() {
        let now = Instant::now();
        let failure = SerialTerminalFailure::new(
            SerialFailureStage::PingFlush,
            std::io::Error::new(std::io::ErrorKind::WouldBlock, "mock CDC busy"),
        );
        assert!(!may_defer_ping_failure(
            &failure,
            now,
            now.checked_sub(PING_KEEPALIVE_RETRY_WINDOW).unwrap_or(now),
            Some(now),
            1,
        ));
        assert!(!may_defer_ping_failure(
            &failure,
            now,
            now.checked_sub(Duration::from_secs(1)).unwrap_or(now),
            now.checked_sub(PING_RECENT_SAMPLE_GRACE + Duration::from_millis(1)),
            1,
        ));
    }

    #[test]
    fn non_timeout_or_repeated_ping_failures_remain_terminal() {
        let now = Instant::now();
        let broken_pipe = SerialTerminalFailure::new(
            SerialFailureStage::PingWrite,
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "mock disconnect"),
        );
        assert!(!may_defer_ping_failure(
            &broken_pipe,
            now,
            now.checked_sub(Duration::from_secs(1)).unwrap_or(now),
            Some(now),
            1,
        ));
        let timed_out = SerialTerminalFailure::new(
            SerialFailureStage::PingWrite,
            std::io::Error::from_raw_os_error(121),
        );
        assert!(!may_defer_ping_failure(
            &timed_out,
            now,
            now.checked_sub(Duration::from_secs(1)).unwrap_or(now),
            Some(now),
            MAX_DEFERRED_PING_FAILURES.saturating_add(1),
        ));
    }

    #[test]
    fn profile_acknowledgement_and_snapshot_are_enforced_before_acquisition() {
        let session = SessionController::default();
        let ecg = built_in_profiles()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .find(|profile| profile.category == "course_ecg")
            .unwrap_or_else(|| panic!("missing ECG profile"));
        assert!(session
            .begin_session(
                true,
                "Simulator",
                "SIM",
                RecordingDuration::Timed { seconds: 10 },
                ecg.snapshot(false),
                RecordingCalibration::default(),
            )
            .is_ok());
        session.disconnect().unwrap_or_else(|e| panic!("{e}"));
        let snapshot = ecg.snapshot(true);
        session
            .begin_session(
                true,
                "Simulator",
                "SIM",
                RecordingDuration::Timed { seconds: 10 },
                snapshot.clone(),
                RecordingCalibration::default(),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.profile, Some(snapshot));
        session.disconnect().unwrap_or_else(|e| panic!("{e}"));
    }

    #[test]
    fn bp_simulator_supports_a_deterministic_xgzp_reference_fit() {
        let profile = built_in_profiles()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .find(|profile| profile.category == "course_blood_pressure")
            .unwrap_or_else(|| panic!("missing BP profile"))
            .snapshot(false);
        let simulator = SimulatorIo::new(&profile, RecordingDuration::Timed { seconds: 10 })
            .unwrap_or_else(|e| panic!("{e}"));
        let points = (0..1_000)
            .map(|sequence| {
                let reference_volts = crate::calibration::counts_to_volts(
                    simulator.signal_counts(sequence, 1),
                    profile.profile.acquisition.adc_resolution_bits,
                    5.0,
                )
                .unwrap_or_else(|e| panic!("{e}"));
                let xgzp_volts = crate::calibration::counts_to_volts(
                    simulator.signal_counts(sequence, 2),
                    profile.profile.acquisition.adc_resolution_bits,
                    5.0,
                )
                .unwrap_or_else(|e| panic!("{e}"));
                crate::calibration::CalibrationPoint {
                    input_voltage: xgzp_volts,
                    reference_value: crate::calibration::mpxv_mmhg(reference_volts, 5.0)
                        .unwrap_or_else(|e| panic!("{e}")),
                }
            })
            .collect::<Vec<_>>();
        let fit = crate::calibration::fit_linear(&points).unwrap_or_else(|e| panic!("{e}"));
        assert!((fit.slope - 120.0).abs() < 0.25);
        assert!((fit.offset + 10.0).abs() < 1.0);
        assert!(fit.r_squared > 0.999);
    }

    #[test]
    fn completed_bp_bmeg_fit_uses_the_requested_interval() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let profile = built_in_profiles()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .find(|profile| profile.category == "course_blood_pressure")
            .unwrap_or_else(|| panic!("missing BP profile"))
            .snapshot(false);
        let bmeg = dir.path().join("bp-calibration.bmeg");
        let session = SessionController::default();
        let metadata = session
            .initial_metadata(InitialMetadataRequest {
                simulator: true,
                source: "SIM",
                bmeg: &bmeg,
                duration: &RecordingDuration::Timed { seconds: 10 },
                profile: &profile,
                calibration: RecordingCalibration::default(),
                recording_path_context: None,
            })
            .unwrap_or_else(|e| panic!("{e}"));
        let simulator = SimulatorIo::new(&profile, RecordingDuration::Timed { seconds: 10 })
            .unwrap_or_else(|e| panic!("{e}"));
        let mut writer =
            BmegWriter::create_synchronized(&bmeg, &metadata, 3).unwrap_or_else(|e| panic!("{e}"));
        for sequence in 0..2_000 {
            writer
                .write_record(&SynchronizedRecord {
                    sequence,
                    timestamp_us: u64::from(sequence) * 5_000,
                    status_flags: 0,
                    counts: vec![
                        simulator.signal_counts(sequence, 0),
                        simulator.signal_counts(sequence, 1),
                        simulator.signal_counts(sequence, 2),
                    ],
                })
                .unwrap_or_else(|e| panic!("{e}"));
        }
        writer.finish().unwrap_or_else(|e| panic!("{e}"));
        let fit =
            crate::calibration::fit_xgzp_from_recording(&crate::calibration::XgzpFitRequest {
                bmeg_path: bmeg.display().to_string(),
                start_seconds: 2.0,
                end_seconds: 8.0,
                adc_reference_v: 5.0,
                mpxv_sensor_supply_v: 5.0,
            })
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fit.paired_samples, 1_201);
        assert!((fit.slope - 120.0).abs() < 0.25);
        assert!((fit.offset + 10.0).abs() < 1.0);
        assert!(fit.r_squared > 0.999);
    }

    #[test]
    fn recording_metadata_snapshots_project_and_relative_trial_folder() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let session = SessionController::default();
        let metadata = session
            .initial_metadata(InitialMetadataRequest {
                simulator: true,
                source: "SIM",
                bmeg: &dir.path().join("project-context.bmeg"),
                duration: &RecordingDuration::Timed { seconds: 10 },
                profile: &default_general_profile().unwrap_or_else(|error| panic!("{error}")),
                calibration: RecordingCalibration::default(),
                recording_path_context: Some(&RecordingPathContext {
                    project_folder: "C:\\Users\\Student\\Documents\\BMEG 420L".into(),
                    output_folder: "Lab6\\Trial1".into(),
                }),
            })
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            metadata.project_folder.as_deref(),
            Some("C:\\Users\\Student\\Documents\\BMEG 420L")
        );
        assert_eq!(metadata.output_folder.as_deref(), Some("Lab6\\Trial1"));
    }

    #[test]
    fn course_configuration_payloads_bind_channel_maps_and_leds() {
        let profiles = built_in_profiles().unwrap_or_else(|error| panic!("{error}"));
        let lookup = |category: &str| {
            profiles
                .iter()
                .find(|profile| profile.category == category)
                .unwrap_or_else(|| panic!("missing {category}"))
        };
        let emg = configure_payload(&lookup("course_emg_force").acquisition, None)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(emg, vec![0, 14, 232, 3, 0, 0, 4, 0, 1, 2, 3, 0]);
        let bp = configure_payload(&lookup("course_blood_pressure").acquisition, None)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(bp, vec![0, 14, 200, 0, 0, 0, 3, 0, 1, 2, 1]);
        let pulseox = configure_payload(&lookup("course_pulseox").acquisition, None)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(pulseox, vec![1, 14, 232, 3, 0, 0, 2, 0, 1, 5, 6]);
    }

    #[test]
    fn configuration_uses_advertised_firmware_capabilities() {
        let profiles = built_in_profiles().unwrap_or_else(|error| panic!("{error}"));
        let emg = profiles
            .iter()
            .find(|profile| profile.category == "course_emg_force")
            .unwrap_or_else(|| panic!("missing EMG profile"));
        let capabilities = FirmwareCapabilities {
            supported_adc_resolutions: vec![12, 14],
            max_analog_channels: 6,
            supported_modes: 0x03,
            supported_digital_output_mask: 0x07,
            supported_rates_hz: vec![100, 200, 250, 500, 1_000],
        };
        assert!(configure_payload(&emg.acquisition, Some(&capabilities)).is_ok());

        let rate_limited = FirmwareCapabilities {
            supported_rates_hz: vec![100, 200, 250, 500],
            ..capabilities.clone()
        };
        assert!(configure_payload(&emg.acquisition, Some(&rate_limited)).is_err());

        let channel_limited = FirmwareCapabilities {
            max_analog_channels: 3,
            ..capabilities
        };
        assert!(configure_payload(&emg.acquisition, Some(&channel_limited)).is_err());
    }

    #[test]
    fn instructor_authored_course_maps_encode_without_changing_the_fixed_protocol_modes() {
        let profiles = built_in_profiles().unwrap_or_else(|error| panic!("{error}"));
        let lookup = |category: &str| {
            profiles
                .iter()
                .find(|profile| profile.category == category)
                .cloned()
                .unwrap_or_else(|| panic!("missing {category}"))
        };
        let capabilities = FirmwareCapabilities {
            supported_adc_resolutions: vec![12, 14],
            max_analog_channels: 6,
            supported_modes: 0x03,
            supported_digital_output_mask: 0x07,
            supported_rates_hz: vec![100, 200, 250, 500, 1_000],
        };

        let mut ecg = lookup("course_ecg");
        ecg.acquisition.channels[0].pin = "A2".into();
        ecg.acquisition.sample_rate_hz = 500;
        assert_eq!(
            configure_payload(&ecg.acquisition, Some(&capabilities))
                .unwrap_or_else(|error| panic!("{error}")),
            vec![0, 14, 244, 1, 0, 0, 1, 2, 0]
        );

        let mut emg = lookup("course_emg_force");
        for (channel, pin) in emg
            .acquisition
            .channels
            .iter_mut()
            .zip(["A1", "A2", "A4", "A5"])
        {
            channel.pin = pin.into();
        }
        emg.acquisition.sample_rate_hz = 500;
        emg.acquisition.adc_resolution_bits = 14;
        assert_eq!(
            configure_payload(&emg.acquisition, Some(&capabilities))
                .unwrap_or_else(|error| panic!("{error}")),
            vec![0, 14, 244, 1, 0, 0, 4, 1, 2, 4, 5, 0]
        );

        let mut bp = lookup("course_blood_pressure");
        for (channel, pin) in bp.acquisition.channels.iter_mut().zip(["A3", "A4", "A5"]) {
            channel.pin = pin.into();
        }
        bp.acquisition.sample_rate_hz = 250;
        assert_eq!(
            configure_payload(&bp.acquisition, Some(&capabilities))
                .unwrap_or_else(|error| panic!("{error}")),
            vec![0, 14, 250, 0, 0, 0, 3, 3, 4, 5, 1]
        );

        let mut pulse = lookup("course_pulseox");
        pulse.acquisition.analog_inputs = Some(crate::profiles::PulseOxInputs {
            tx: "A2".into(),
            rx: "A3".into(),
        });
        pulse.acquisition.digital_outputs = vec![
            crate::profiles::DigitalOutput {
                pin: "D4".into(),
                label: "Red LED".into(),
                behavior: crate::profiles::DigitalOutputBehavior::AcquisitionSequenced,
            },
            crate::profiles::DigitalOutput {
                pin: "D6".into(),
                label: "IR LED".into(),
                behavior: crate::profiles::DigitalOutputBehavior::AcquisitionSequenced,
            },
        ];
        pulse.acquisition.state_dwell_us = Some(2_000);
        assert_eq!(
            configure_payload(&pulse.acquisition, Some(&capabilities))
                .unwrap_or_else(|error| panic!("{error}")),
            vec![1, 14, 208, 7, 0, 0, 2, 2, 3, 4, 6]
        );
    }

    #[test]
    fn simulator_emits_four_channel_and_eight_field_pulseox_records() {
        let profiles = built_in_profiles().unwrap_or_else(|error| panic!("{error}"));
        for (category, fields, period) in [
            ("course_emg_force", 4u8, 1_000u32),
            ("course_pulseox", 8u8, 4_000u32),
        ] {
            let profile = profiles
                .iter()
                .find(|profile| profile.category == category)
                .unwrap_or_else(|| panic!("missing {category}"))
                .snapshot(false);
            let mut simulator =
                SimulatorIo::new(&profile, RecordingDuration::Timed { seconds: 10 })
                    .unwrap_or_else(|error| panic!("{error}"));
            let configure = configure_payload(&profile.profile.acquisition, None)
                .unwrap_or_else(|error| panic!("{error}"));
            simulator
                .write_all(
                    &encode_frame(&Frame {
                        message_type: MessageType::Configure,
                        flags: 0,
                        sequence: 1,
                        payload: configure,
                    })
                    .unwrap_or_else(|error| panic!("{error:?}")),
                )
                .unwrap_or_else(|error| panic!("{error}"));
            simulator
                .write_all(
                    &encode_frame(&Frame {
                        message_type: MessageType::Start,
                        flags: 0,
                        sequence: 2,
                        payload: vec![],
                    })
                    .unwrap_or_else(|error| panic!("{error:?}")),
                )
                .unwrap_or_else(|error| panic!("{error}"));
            simulator.batch_interval = None;
            let mut parser = FrameParser::default();
            let mut buffer = [0u8; 256];
            let mut batch = None;
            for _ in 0..100 {
                let read = simulator
                    .read(&mut buffer)
                    .unwrap_or_else(|error| panic!("{error}"));
                for frame in parser.push(&buffer[..read]) {
                    if frame.message_type == MessageType::SampleBatch {
                        batch = Some(
                            SampleBatch::from_payload(&frame.payload)
                                .unwrap_or_else(|error| panic!("{error:?}")),
                        );
                    }
                }
                if batch.is_some() {
                    break;
                }
            }
            let batch = batch.unwrap_or_else(|| panic!("no batch for {category}"));
            assert_eq!(batch.channel_count, fields);
            assert_eq!(batch.sample_period_us, period);
            assert!(batch.samples.len() >= usize::from(fields));
        }
    }
}
