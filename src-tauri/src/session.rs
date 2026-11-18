//! Production acquisition-session controller.
//!
//! The controller is shared by Tauri commands, but a worker owns every blocking
//! transport read and disk write. Status and the bounded display history are copied
//! under a short mutex so polling the UI never waits for serial I/O.
use crate::{
    acquisition::{
        AcquisitionController, AcquisitionSnapshot, AcquisitionState, FirmwareCapabilities,
    },
    calibration::{CalibrationType, RecordingCalibration},
    profiles::{built_in_profiles, ProfileSnapshot, ProfileStatus},
    protocol::{
        encode_frame, Frame, FrameParser, IntegrityCounters, MessageType, SampleBatch,
        CONTROLLED_SERIAL_BAUD, REFERENCE_DEVICE_ID, REFERENCE_FIRMWARE_BUILD,
    },
    recording::{
        check_synchronized_header, export_bmeg_csv, synchronized_record_bytes, BmegWriter,
        RecordLayout, RecordingDuration, RecordingError, RecordingMarker, RecordingMetadata,
        RecordingSafetyContext, StopReason, SynchronizedRecord,
    },
    serial_lease::{SerialBusy, SerialLease, SerialOwner, SerialOwnership},
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
/// How long a claim waits for a previous owner of the serial lease that has
/// already published its final state and is returning. A real conflict (a
/// recording, restore, or handshake still running) is refused after this.
const SERIAL_CLAIM_GRACE: Duration = Duration::from_millis(500);
const DISK_WARNING_BYTES: u64 = 1024 * 1024 * 1024;
const DISK_CRITICAL_BYTES: u64 = 250 * 1024 * 1024;
const DISK_CHECK_INTERVAL: Duration = Duration::from_secs(15);
const RECORDING_FLUSH_INTERVAL: Duration = Duration::from_secs(5);
/// Bounded wait, after STOP, for the firmware's terminal STATUS. The firmware
/// sends that STATUS only after its final partial batch and every queued
/// batch, so receiving it also proves the recording's tail was read.
const STOP_STATUS_TIMEOUT: Duration = Duration::from_millis(300);
/// OneDrive, Windows Search, and antivirus scanners briefly open files that
/// were just closed. A bounded retry turns those sharing violations into a
/// short delay instead of a lost rename.
const RENAME_ATTEMPTS: u32 = 5;
const RENAME_INITIAL_BACKOFF: Duration = Duration::from_millis(50);
/// Provenance strings (Arduino CLI/core versions, board serial) are known only
/// inside the worker. The synchronous Start precheck reserves room for them.
const HEADER_PROVENANCE_RESERVE_BYTES: usize = 2 * 1024;
/// Conservative upper bounds for one exported CSV row, used so the storage
/// guard keeps room for the CSV that finalization writes from the BMEG.
/// Prefix: record/cycle index (<= 10 digits), t_us (<= 20 digits), commas, newline.
const CSV_ROW_PREFIX_BYTES: u64 = 34;
/// ",65535" raw ADC counts.
const CSV_COUNTS_FIELD_BYTES: u64 = 6;
/// ",d.dddddd" volts with headroom for unusual reference voltages.
const CSV_VOLTS_FIELD_BYTES: u64 = 12;
/// A signed calibrated value with six decimals.
const CSV_DERIVED_FIELD_BYTES: u64 = 24;
/// Column header plus slack for filesystem allocation granularity.
const CSV_HEADER_RESERVE_BYTES: u64 = 64 * 1024;
/// Reason shown by `powercfg /requests` while a recording keeps Windows awake.
const KEEP_AWAKE_REASON: &str =
    "WVU Bioinstrumentation Studio is recording data from an Arduino or the simulator.";
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
///
/// Timestamps strictly increase within a session (non-increasing ones are
/// rejected at ingest and `recent` is cleared by `begin_session`), so the
/// window is a suffix located by binary search. Only the at most `max_points`
/// selected records are cloned while the runtime lock is held.
fn select_display_records(
    records: &VecDeque<SynchronizedRecord>,
    window_seconds: f64,
    max_points: usize,
) -> Vec<SynchronizedRecord> {
    let Some(latest) = records.back() else {
        return Vec::new();
    };
    if max_points == 0 {
        return Vec::new();
    }
    let window_us = (window_seconds * 1_000_000.0).round() as u64;
    let earliest_timestamp = latest.timestamp_us.saturating_sub(window_us);
    let start = records.partition_point(|record| record.timestamp_us < earliest_timestamp);
    let available = records.len() - start;

    if available <= max_points {
        return records.range(start..).cloned().collect();
    }
    if max_points == 1 {
        return vec![latest.clone()];
    }

    // Include both interval endpoints with evenly distributed indices. The
    // newest exact raw display record is therefore never replaced by a stride
    // approximation, while recorded BMEG/CSV frames are never decimated.
    let last_offset = available - 1;
    (0..max_points)
        .map(|index| records[start + index * last_offset / (max_points - 1)].clone())
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

/// File operations used while capturing and finalizing a recording. The
/// production implementation is the real file system; tests inject failures
/// to prove that STOP, metadata, and an honest fault always follow.
trait RecordingStorage: Send + Sync {
    /// Periodic in-capture flush of buffered BMEG records.
    fn flush(&self, writer: &mut BmegWriter) -> Result<(), RecordingError> {
        writer.flush()
    }

    /// Final flush of the BMEG before it is renamed (fsync is best effort).
    fn finish(&self, writer: BmegWriter) -> Result<(), RecordingError> {
        writer.finish()
    }

    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        fs::rename(from, to)
    }

    /// Streams the finished BMEG into a CSV and returns the row count.
    fn export_csv(&self, bmeg: &Path, csv: &Path) -> Result<u64, RecordingError> {
        export_bmeg_csv(bmeg, csv)
    }

    /// Creates or truncates `path` and writes `bytes`. A write error fails;
    /// the fsync that follows is best effort, as for the BMEG and CSV.
    fn write_durably(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let mut file = fs::File::create(path)?;
        file.write_all(bytes)?;
        crate::recording::sync_best_effort(&file, "metadata");
        Ok(())
    }
}

struct SystemRecordingStorage;

impl RecordingStorage for SystemRecordingStorage {}

/// Capture cadence. Production values are fixed; tests shorten them so the
/// storage guard, flush, and rename paths run within a test's time budget.
/// The seven-second no-data timeout is deliberately not part of this seam.
#[derive(Clone, Copy, Debug)]
struct CaptureTiming {
    disk_check_interval: Duration,
    flush_interval: Duration,
    rename_attempts: u32,
    rename_initial_backoff: Duration,
}

impl Default for CaptureTiming {
    fn default() -> Self {
        Self {
            disk_check_interval: DISK_CHECK_INTERVAL,
            flush_interval: RECORDING_FLUSH_INTERVAL,
            rename_attempts: RENAME_ATTEMPTS,
            rename_initial_backoff: RENAME_INITIAL_BACKOFF,
        }
    }
}

/// Keeps Windows from idle-sleeping while a recording worker runs, from
/// Connecting until finalization ends. A sleeping host stops sending PINGs,
/// so the firmware watchdog would end the recording as a fault. Creating the
/// request may fail (for example under restrictive policy); that is logged
/// and never blocks recording. The request is cleared and its handle closed
/// on drop, on every exit path.
struct KeepAwakeGuard {
    #[cfg(windows)]
    request: Option<windows_sys::Win32::Foundation::HANDLE>,
}

#[cfg(windows)]
impl KeepAwakeGuard {
    fn acquire(reason: &str) -> Self {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Power::{
            PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest,
        };
        use windows_sys::Win32::System::Threading::{
            POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
        };

        let mut reason_text: Vec<u16> = reason.encode_utf16().chain(std::iter::once(0)).collect();
        let context = REASON_CONTEXT {
            // POWER_REQUEST_CONTEXT_VERSION (DIAGNOSTIC_REASON_VERSION) is 0.
            Version: 0,
            Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
            Reason: REASON_CONTEXT_0 {
                SimpleReasonString: reason_text.as_mut_ptr(),
            },
        };
        // SAFETY: `context` and the NUL-terminated `reason_text` it points to
        // are valid for the duration of the call; Windows copies the reason.
        let request = unsafe { PowerCreateRequest(&context) };
        if request.is_null() || request == INVALID_HANDLE_VALUE {
            crate::app_log::record(
                "WARN",
                &format!(
                    "KEEP_AWAKE_UNAVAILABLE stage=create detail={}",
                    std::io::Error::last_os_error()
                ),
            );
            return Self { request: None };
        }
        // SAFETY: `request` is a valid power-request handle created above.
        if unsafe { PowerSetRequest(request, PowerRequestSystemRequired) } == 0 {
            crate::app_log::record(
                "WARN",
                &format!(
                    "KEEP_AWAKE_UNAVAILABLE stage=set detail={}",
                    std::io::Error::last_os_error()
                ),
            );
            // SAFETY: the handle is valid and owned here; it is closed once.
            unsafe {
                CloseHandle(request);
            }
            return Self { request: None };
        }
        crate::app_log::record("INFO", "KEEP_AWAKE_ACTIVE");
        Self {
            request: Some(request),
        }
    }
}

#[cfg(windows)]
impl Drop for KeepAwakeGuard {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Power::{PowerClearRequest, PowerRequestSystemRequired};

        if let Some(request) = self.request.take() {
            // SAFETY: `request` was created and set by `acquire`, is owned by
            // this guard, and is cleared and closed exactly once.
            unsafe {
                PowerClearRequest(request, PowerRequestSystemRequired);
                CloseHandle(request);
            }
            crate::app_log::record("INFO", "KEEP_AWAKE_RELEASED");
        }
    }
}

#[cfg(not(windows))]
impl KeepAwakeGuard {
    fn acquire(_reason: &str) -> Self {
        Self {}
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
    /// The recording folder could not be used, or the recording could not be
    /// written or finalized completely. This is a storage problem, not a
    /// serial or firmware one, so it must not suggest verifying firmware.
    RecordingStorageError,
    /// The selected lab's channels, outputs, or rate cannot be configured on
    /// the connected firmware (checked before CONFIGURE is sent).
    LabNotSupportedByFirmware,
    /// The firmware rejected CONFIGURE or did not acknowledge it in time.
    ConfigurationRejected,
    /// The firmware rejected START or did not confirm acquisition in time.
    StartNotConfirmed,
}

/// Which operation raised the current fault, so the UI does not describe a
/// failed Verify or connection attempt as a recording that ended, and does
/// not send a student to firmware recovery for a storage problem.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultOrigin {
    /// Verify Firmware, including the verification step of a restore.
    Verification,
    /// Retry connection, or a hardware start that failed before START.
    Connection,
    /// Acquisition ended by a serial, firmware, or session failure.
    Recording,
    /// The recording folder or files could not be used or saved.
    Storage,
    /// The recording request itself was refused before START (for example a
    /// lab header too large for a recording). No recording ran, and the board
    /// and connection are not at fault; `last_error` is the student text.
    Request,
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
    /// The reset/re-enumeration fields below are retained for the stable
    /// diagnostics shape. "Reset board and retry" was retired, so they are
    /// always false/empty apart from `original_port`.
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
            recommended_action: "Select Retry connection, or Refresh Board.".into(),
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
pub struct HandshakeRetryResult {
    pub handshake_succeeded: bool,
    pub diagnostics: ConnectionDiagnostics,
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
    /// Output mask from a STATUS received after STOP; `None` when STOP was
    /// not confirmed (for example after a transport fault).
    pub final_digital_output_mask: Option<u8>,
    /// Why no CSV was produced. `csv_path` is empty in that case; the raw
    /// BMEG and metadata are kept.
    pub csv_error: Option<String>,
    /// Flush/rename/metadata problems while saving. `bmeg_path` names the raw
    /// file that was actually kept (a `.bmeg.part` if it could not be renamed).
    pub finalize_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionStatus {
    pub state: SessionState,
    /// True while a recording (a Start) owns the session, from `Connecting`
    /// through `Stopping`. False while Retry connection, Verify Firmware, or a
    /// restore's verification runs its handshake: their `Connecting` and
    /// `Connected` states describe a board check, not a recording.
    pub recording_active: bool,
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
    /// Which operation raised `last_error`; present whenever it is.
    pub fault_origin: Option<FaultOrigin>,
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
    /// A requested stop arrived before acquisition began (during connection,
    /// handshake, or configuration). It is the only worker error that ends a
    /// session quietly; every other error, including one raised while saving
    /// after a user Stop, is reported as a fault.
    #[error("session was cancelled")]
    Cancelled,
    /// The recording could not be written or finalized completely. Files that
    /// were saved are kept and described in the summary and metadata.
    #[error("save: {0}")]
    Save(String),
    /// The recording request itself cannot be honored (for example a lab
    /// header that exceeds the BMEG limit). The message is student-facing, so
    /// it carries no prefix, and it is not a connection or firmware problem.
    #[error("{0}")]
    InvalidRequest(&'static str),
    /// Another operation owns the serial port and session. The message is
    /// student-facing.
    #[error("{0}")]
    Busy(#[from] SerialBusy),
}

#[derive(Clone)]
pub struct SessionController {
    runtime: Arc<Mutex<SessionRuntime>>,
    cancel: Arc<AtomicBool>,
    worker: Arc<Mutex<Option<JoinHandle<()>>>>,
    stop_reason: Arc<Mutex<Option<StopReason>>>,
    disk_space: Arc<dyn DiskSpaceProvider>,
    storage: Arc<dyn RecordingStorage>,
    timing: CaptureTiming,
    /// One owner at a time for the COM port and this session's state, shared
    /// with the firmware workflow.
    serial: SerialOwnership,
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
    fault_origin: Option<FaultOrigin>,
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
    /// Set while `retry_handshake` runs (see `SessionStatus::recording_active`).
    handshake_check: bool,
}

/// Clears `SessionRuntime::handshake_check` when a handshake check returns,
/// whichever way it returns.
struct HandshakeCheckGuard<'a> {
    runtime: &'a Mutex<SessionRuntime>,
}

impl Drop for HandshakeCheckGuard<'_> {
    fn drop(&mut self) {
        let mut runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        runtime.handshake_check = false;
    }
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
    recording_context: RecordingContext,
    /// Identity observed before the worker owns the serial handle. It is later
    /// compared with a read-only Windows enumeration if a serial operation
    /// fails; no reset, reopen, or Arduino CLI scan is performed at that time.
    expected_port_identity: Option<UsbPortIdentity>,
    /// Tooling provenance collected before the port was opened. `None` lets
    /// capture collect it (simulator, or a direct synchronous capture).
    provenance: Option<ToolingProvenance>,
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

/// A host-side failure that ends acquisition: the BMEG sink could not accept
/// or flush records, or session state could not be updated. Like a serial
/// failure it is a terminal cause; STOP, the post-STOP drain, and
/// finalization still run.
#[derive(Clone, Copy, Debug)]
enum HostFailureStage {
    RecordingWrite,
    RecordingFlush,
    SessionState,
}

impl HostFailureStage {
    fn label(self) -> &'static str {
        match self {
            Self::RecordingWrite => "RECORDING_WRITE",
            Self::RecordingFlush => "RECORDING_FLUSH",
            Self::SessionState => "SESSION_STATE",
        }
    }
}

#[derive(Debug)]
struct HostCaptureFailure {
    stage: HostFailureStage,
    detail: String,
}

impl HostCaptureFailure {
    fn new(stage: HostFailureStage, detail: impl Into<String>) -> Self {
        Self {
            stage,
            detail: detail.into(),
        }
    }

    fn describe(&self) -> String {
        format!("{}: {}", self.stage.label(), self.detail)
    }
}

/// The four files of one recording. The raw data are written to the `.part`
/// path and renamed to `bmeg` only after they were flushed and fsynced.
#[derive(Clone, Debug)]
struct RecordingPaths {
    temporary_bmeg: PathBuf,
    bmeg: PathBuf,
    csv: PathBuf,
    metadata: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FinalizeStage {
    Flush,
    Rename,
    Metadata,
    Csv,
}

impl FinalizeStage {
    fn label(self) -> &'static str {
        match self {
            Self::Flush => "flush",
            Self::Rename => "rename",
            Self::Metadata => "metadata",
            Self::Csv => "csv",
        }
    }
}

#[derive(Clone, Debug)]
struct FinalizeFailure {
    stage: FinalizeStage,
    detail: String,
}

impl FinalizeFailure {
    fn new(stage: FinalizeStage, detail: impl Into<String>) -> Self {
        Self {
            stage,
            detail: detail.into(),
        }
    }
}

/// Facts that finalization needs but does not derive from the files.
struct FinalizePlan {
    reason: StopReason,
    /// Set when acquisition ended because records could not be written.
    recording_failure: Option<String>,
    records_written: u64,
    csv_bytes_estimate: u64,
    free_bytes: Option<u64>,
}

/// What finalization actually left on disk. Every step is attempted even
/// when an earlier one failed, and nothing that holds samples is deleted.
struct FinalizeOutcome {
    /// The file holding the raw recording: the final `.bmeg`, or the `.part`
    /// when it could not be flushed completely or renamed.
    raw_path: PathBuf,
    metadata_written: bool,
    csv_written: bool,
    csv_error: Option<String>,
    /// Every non-CSV problem, including a failed final metadata write that
    /// the sidecar itself cannot record.
    finalize_error: Option<String>,
    failures: Vec<FinalizeFailure>,
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

/// Operator-entered provenance that is frozen at Start. An empty note selects
/// a profile-derived default; every nonempty note is preserved exactly.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordingContext {
    pub recording_note: String,
}

/// Immutable non-device inputs for starting a recording. Grouping them keeps
/// the public start boundary stable as recording provenance evolves.
#[derive(Clone)]
pub struct RecordingStartOptions {
    pub duration: RecordingDuration,
    pub output_dir: PathBuf,
    pub calibration: RecordingCalibration,
    pub recording_path_context: Option<RecordingPathContext>,
    pub recording_context: RecordingContext,
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
    recording_context: &'a RecordingContext,
}

impl Default for SessionController {
    fn default() -> Self {
        Self::with_disk_space(Arc::new(SystemDiskSpace))
    }
}

impl SessionController {
    fn with_disk_space(disk_space: Arc<dyn DiskSpaceProvider>) -> Self {
        Self::with_environment(
            disk_space,
            Arc::new(SystemRecordingStorage),
            CaptureTiming::default(),
        )
    }

    fn with_environment(
        disk_space: Arc<dyn DiskSpaceProvider>,
        storage: Arc<dyn RecordingStorage>,
        timing: CaptureTiming,
    ) -> Self {
        Self {
            runtime: Arc::new(Mutex::new(SessionRuntime {
                state: SessionState::Disconnected,
                board: String::new(),
                port: String::new(),
                simulator: false,
                protocol_version: format!(
                    "{}.{}",
                    crate::protocol::PROTOCOL_MAJOR,
                    crate::protocol::PROTOCOL_MINOR
                ),
                recent: VecDeque::with_capacity(DISPLAY_CAPACITY),
                samples: 0,
                packets: 0,
                measured_rate_hz: 0.0,
                integrity: IntegrityCounters::default(),
                last_error: None,
                fault_origin: None,
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
                handshake_check: false,
            })),
            cancel: Arc::new(AtomicBool::new(false)),
            worker: Arc::new(Mutex::new(None)),
            stop_reason: Arc::new(Mutex::new(None)),
            disk_space,
            storage,
            timing,
            serial: SerialOwnership::default(),
        }
    }

    /// The serial-ownership lease shared by recordings, Retry connection,
    /// Verify Firmware, and firmware restore jobs.
    pub fn serial_ownership(&self) -> SerialOwnership {
        self.serial.clone()
    }

    /// Claims the serial port and session for `owner`, or refuses with a
    /// student-readable busy error. It waits briefly only for an owner that is
    /// already returning (see `SERIAL_CLAIM_GRACE`).
    pub fn claim_serial(&self, owner: SerialOwner) -> Result<SerialLease, SessionError> {
        self.serial
            .claim_within(owner, SERIAL_CLAIM_GRACE)
            .map_err(SessionError::Busy)
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
        Ok(is_active_state(self.lock_runtime()?.state))
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
            RecordingContext::default(),
        )
    }

    pub fn start_simulator_with_profile_calibration_and_path_context(
        &self,
        profile: ProfileSnapshot,
        duration: RecordingDuration,
        output_dir: PathBuf,
        calibration: RecordingCalibration,
        recording_path_context: Option<RecordingPathContext>,
        recording_context: RecordingContext,
    ) -> Result<SessionStatus, SessionError> {
        let lease = self.claim_serial(SerialOwner::Session)?;
        self.start_simulator_leased(
            lease,
            profile,
            RecordingStartOptions {
                duration,
                output_dir,
                calibration,
                recording_path_context,
                recording_context,
            },
        )
    }

    /// Starts a simulator recording under a session lease the caller already
    /// claimed. The worker keeps the lease until it has finished saving.
    pub fn start_simulator_leased(
        &self,
        lease: SerialLease,
        profile: ProfileSnapshot,
        options: RecordingStartOptions,
    ) -> Result<SessionStatus, SessionError> {
        require_session_lease(&lease)?;
        let RecordingStartOptions {
            duration,
            output_dir,
            calibration,
            recording_path_context,
            recording_context,
        } = options;
        self.validate_duration(&duration)?;
        validate_recording_context(&recording_context)?;
        calibration
            .validate()
            .map_err(|error| SessionError::Protocol(error.to_string()))?;
        check_header_budget(HeaderBudgetRequest {
            simulator: true,
            source: "SIM",
            duration: &duration,
            profile: &profile,
            calibration: &calibration,
            recording_path_context: recording_path_context.as_ref(),
            recording_context: &recording_context,
        })?;
        self.begin_session(
            true,
            "Simulator",
            "SIM",
            duration.clone(),
            profile.clone(),
            calibration.clone(),
        )?;
        let controller = self.clone();
        self.spawn_worker(lease, move || {
            controller.capture_simulator_worker(
                profile,
                duration,
                output_dir,
                calibration,
                recording_path_context,
                recording_context,
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
        self.start_serial_with_profile_and_options(
            profile,
            port_name,
            RecordingStartOptions {
                duration,
                output_dir,
                calibration,
                recording_path_context: None,
                recording_context: RecordingContext::default(),
            },
        )
    }

    pub fn start_serial_with_profile_and_options(
        &self,
        profile: ProfileSnapshot,
        port_name: String,
        options: RecordingStartOptions,
    ) -> Result<SessionStatus, SessionError> {
        let lease = self.claim_serial(SerialOwner::Session)?;
        self.start_serial_leased(lease, profile, port_name, options, None)
    }

    /// Starts a hardware recording under a session lease the caller already
    /// claimed; the worker keeps it until it has finished saving.
    /// `board_serial` comes from the caller's board scan, so Start does not
    /// scan again. Tool versions come from the per-run cache and are collected
    /// here, before the worker opens the port.
    pub fn start_serial_leased(
        &self,
        lease: SerialLease,
        profile: ProfileSnapshot,
        port_name: String,
        options: RecordingStartOptions,
        board_serial: Option<String>,
    ) -> Result<SessionStatus, SessionError> {
        require_session_lease(&lease)?;
        let RecordingStartOptions {
            duration,
            output_dir,
            calibration,
            recording_path_context,
            recording_context,
        } = options;
        self.validate_duration(&duration)?;
        validate_recording_context(&recording_context)?;
        calibration
            .validate()
            .map_err(|error| SessionError::Protocol(error.to_string()))?;
        check_header_budget(HeaderBudgetRequest {
            simulator: false,
            source: &port_name,
            duration: &duration,
            profile: &profile,
            calibration: &calibration,
            recording_path_context: recording_path_context.as_ref(),
            recording_context: &recording_context,
        })?;
        let provenance = tooling_provenance(false, board_serial);
        self.begin_session(
            false,
            "Arduino UNO R4 WiFi",
            &port_name,
            duration.clone(),
            profile.clone(),
            calibration.clone(),
        )?;
        let request = CaptureRequest {
            simulator: false,
            source: port_name,
            profile,
            duration,
            output_dir,
            calibration,
            recording_path_context,
            recording_context,
            expected_port_identity: None,
            provenance: Some(provenance),
        };
        let controller = self.clone();
        self.spawn_worker(lease, move || controller.capture_serial_worker(request))?;
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
        let _lease = self.claim_serial(SerialOwner::Session)?;
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
            RecordingContext::default(),
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
        let _lease = self.claim_serial(SerialOwner::Session)?;
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
            RecordingContext::default(),
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
        let _lease = self.claim_serial(SerialOwner::Session)?;
        self.validate_duration(&duration)?;
        let provenance = tooling_provenance(false, None);
        self.begin_session(
            false,
            "Arduino UNO R4 WiFi",
            port_name,
            duration.clone(),
            profile.clone(),
            RecordingCalibration::default(),
        )?;
        self.capture_serial_worker(CaptureRequest {
            simulator: false,
            source: port_name.to_owned(),
            profile,
            duration,
            output_dir: output_dir.to_path_buf(),
            calibration: RecordingCalibration::default(),
            recording_path_context: None,
            recording_context: RecordingContext::default(),
            expected_port_identity: None,
            provenance: Some(provenance),
        })?;
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
                SessionState::Acquiring
                    | SessionState::Configured
                    | SessionState::Connected
                    | SessionState::Connecting
            ) {
                runtime.state = SessionState::Stopping;
            }
        }
        self.set_stop_reason_once(reason)?;
        self.cancel.store(true, Ordering::Release);
        self.status()
    }

    /// Idempotent disconnect. The worker is joined without holding the runtime mutex.
    ///
    /// If this call stopped a running worker and that worker ended `Faulted`
    /// (including a save failure after this Stop), the fault and its
    /// `last_error` are kept so the UI can explain what was not saved. An idle
    /// historical fault is cleared to `Disconnected`, as before.
    pub fn disconnect(&self) -> Result<SessionStatus, SessionError> {
        let stopped_an_active_session = self.is_recording()?;
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
        if !(stopped_an_active_session && runtime.state == SessionState::Faulted) {
            runtime.state = SessionState::Disconnected;
        }
        Ok(Self::status_from_runtime(&runtime))
    }

    /// Disconnect for the UI: it never waits for a recording to finish.
    ///
    /// An active recording is asked to stop and returns `Stopping`; its worker
    /// saves the files and publishes the final state (`Disconnected`, or
    /// `Faulted` if saving failed), which status polling then shows. With no
    /// active session, an idle fault is cleared to `Disconnected`. Verify,
    /// Retry connection, and a firmware restore are never interrupted: the
    /// request is refused as busy while one of them owns the board.
    pub fn request_disconnect(&self) -> Result<SessionStatus, SessionError> {
        let _lease = match self.serial.try_claim(SerialOwner::Session) {
            Ok(lease) => lease,
            Err(busy) if busy.holder == SerialOwner::Session => {
                if self.is_recording()? {
                    return self.request_stop_with_reason(StopReason::User);
                }
                // The session lease is held but nothing is recording: either
                // a worker that has published its final state and is
                // returning, or a Start that has not begun its session yet.
                // A stop flag set now would be cleared by that Start, so the
                // Disconnect would be lost while the recording starts anyway.
                // Wait briefly for a finishing worker; otherwise refuse.
                match self
                    .serial
                    .claim_within(SerialOwner::Session, SERIAL_CLAIM_GRACE)
                {
                    Ok(lease) => lease,
                    Err(_) if self.is_recording()? => {
                        return self.request_stop_with_reason(StopReason::User);
                    }
                    Err(busy) => return Err(SessionError::Busy(busy)),
                }
            }
            Err(busy) => return Err(SessionError::Busy(busy)),
        };
        if self.is_recording()? {
            // Only a session begun without a lease (tests) reaches this.
            return self.request_stop_with_reason(StopReason::User);
        }
        // Holding the lease proves any previous worker released it on its way
        // out, so this join does not wait for a recording.
        self.wait_for_worker()?;
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
        runtime.fault_origin = None;
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

    /// Handshake-only connection check that never resets or uploads the board.
    /// It backs Retry connection, Verify Firmware, and the verification step
    /// of a restore. The caller holds the serial lease for that operation;
    /// `origin` says which one, so a failure is not described as a recording.
    ///
    /// Every failure leaves a `failure_category`, including a port that could
    /// not be opened (busy, or no longer listed by Windows).
    pub fn retry_handshake(
        &self,
        port: &str,
        origin: FaultOrigin,
    ) -> Result<HandshakeRetryResult, SessionError> {
        if self.is_recording()? {
            return Err(SessionError::State(
                "cannot retry a handshake while a recording or connection is active",
            ));
        }
        self.wait_for_worker()?;
        {
            // A fresh attempt replaces the previous fault; its diagnostics are
            // replaced below as well. The states it passes through are a board
            // check, never reported as a recording.
            let mut runtime = self.lock_runtime()?;
            runtime.state = SessionState::Connecting;
            runtime.handshake_check = true;
            runtime.last_error = None;
            runtime.fault_origin = None;
        }
        let _check = HandshakeCheckGuard {
            runtime: &self.runtime,
        };
        self.cancel.store(false, Ordering::Release);
        self.set_connection_diagnostics(ConnectionDiagnostics::new(port))?;

        // The active-capture timeout, not a 25 ms probe timeout: Windows may
        // abort a short PING write while frames are arriving (see
        // ACTIVE_SERIAL_IO_TIMEOUT).
        let mut serial = match serialport::new(port, CONTROLLED_SERIAL_BAUD)
            .timeout(ACTIVE_SERIAL_IO_TIMEOUT)
            .open()
        {
            Ok(serial) => serial,
            Err(error) => {
                // Windows reports a port held by another program and a
                // removed device alike, so a read-only enumeration decides.
                let category = port_open_failure_category(port);
                crate::app_log::record(
                    "WARN",
                    &format!(
                        "HANDSHAKE_PORT_OPEN_FAIL port={port} category={category:?} detail={error}"
                    ),
                );
                self.note_failure_category(category, true)?;
                self.set_fault(origin, format!("could not open {port}: {error}"))?;
                return Ok(HandshakeRetryResult {
                    handshake_succeeded: false,
                    diagnostics: self.current_diagnostics(port)?,
                });
            }
        };
        let result = (|| {
            self.mark_port_opened(port)?;
            serial.clear(serialport::ClearBuffer::Input)?;
            serial.write_data_terminal_ready(true)?;
            serial.write_request_to_send(true)?;
            self.set_state(SessionState::Connected)?;
            let (sender, _receiver) = sync_channel(16);
            let mut acquisition = AcquisitionController::new(sender);
            self.wait_for_handshake(&mut serial, &mut acquisition)
        })();
        drop(serial);
        match result {
            Ok(()) => {
                self.set_state(SessionState::Disconnected)?;
                Ok(HandshakeRetryResult {
                    handshake_succeeded: true,
                    diagnostics: self.current_diagnostics(port)?,
                })
            }
            Err(error) => {
                // The handshake records its own category; control-line or
                // other serial errors after opening are transport problems.
                if !matches!(error, SessionError::Cancelled) {
                    self.note_failure_category(
                        ConnectionFailureCategory::SerialTransportError,
                        false,
                    )?;
                }
                self.set_fault(origin, error.to_string())?;
                Ok(HandshakeRetryResult {
                    handshake_succeeded: false,
                    diagnostics: self.current_diagnostics(port)?,
                })
            }
        }
    }

    fn current_diagnostics(&self, port: &str) -> Result<ConnectionDiagnostics, SessionError> {
        Ok(self
            .status()?
            .connection_diagnostics
            .unwrap_or_else(|| ConnectionDiagnostics::new(port)))
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
        runtime.handshake_check = false;
        runtime.board = board.to_owned();
        runtime.port = port.to_owned();
        runtime.simulator = simulator;
        runtime.protocol_version = format!(
            "{}.{}",
            crate::protocol::PROTOCOL_MAJOR,
            crate::protocol::PROTOCOL_MINOR
        );
        runtime.recent.clear();
        runtime.samples = 0;
        runtime.packets = 0;
        runtime.measured_rate_hz = 0.0;
        runtime.integrity = IntegrityCounters::default();
        runtime.last_error = None;
        runtime.fault_origin = None;
        runtime.last_summary = None;
        runtime.duration = Some(duration);
        // Elapsed/remaining are measured from START, set with `Acquiring` by
        // `set_acquiring`; connection setup is not part of the recording.
        runtime.started_at = None;
        // A previous session's output mask must not be shown while connecting.
        runtime.digital_output_mask = None;
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

    /// Runs `capture` on the session worker, which owns `lease` until it
    /// returns: the lease is released only after the recording is saved (or
    /// when the worker unwinds), never when the start command returns.
    fn spawn_worker<F>(&self, lease: SerialLease, capture: F) -> Result<(), SessionError>
    where
        F: FnOnce() -> Result<(), SessionError> + Send + 'static,
    {
        require_session_lease(&lease)?;
        let mut worker = self
            .worker
            .lock()
            .map_err(|_| SessionError::State("worker lock poisoned"))?;
        // Every worker holds the session lease until it returns, and the
        // caller holds it now, so a previous worker is already exiting.
        if let Some(previous) = worker.take() {
            let _ = previous.join();
        }
        *worker = Some(thread::spawn(move || {
            let _lease = lease;
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
        recording_context: RecordingContext,
    ) -> Result<(), SessionError> {
        // Held until this function returns, i.e. through finalization.
        let _keep_awake = KeepAwakeGuard::acquire(KEEP_AWAKE_REASON);
        let result = (|| {
            let mut simulator = SimulatorIo::new(&profile, duration.clone())?;
            self.set_progress_state(SessionState::Connected)?;
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
                    recording_context,
                    expected_port_identity: None,
                    provenance: None,
                },
            )
        })();
        self.finish_worker_error(&result)?;
        result
    }

    fn capture_serial_worker(&self, mut request: CaptureRequest) -> Result<(), SessionError> {
        // Held until this function returns, i.e. through finalization.
        let _keep_awake = KeepAwakeGuard::acquire(KEEP_AWAKE_REASON);
        let result = (|| {
            let port_name = request.source.clone();
            // This is read-only Windows serial-port enumeration. The captured
            // identity is used only if a later active-capture error occurs.
            request.expected_port_identity = usb_port_identity(&port_name);
            // Board serial provenance normally comes from the Start command's
            // Arduino CLI scan; the operating system's USB serial is the fallback.
            if let Some(provenance) = request.provenance.as_mut() {
                if provenance.board_serial.is_none() {
                    provenance.board_serial = request
                        .expected_port_identity
                        .as_ref()
                        .and_then(|identity| identity.serial_number.clone());
                }
            }
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
            self.set_progress_state(SessionState::Connected)?;
            self.capture_transport(&mut port, request)
        })();
        self.finish_worker_error(&result)?;
        result
    }

    /// Runs one recording on an already opened transport.
    ///
    /// Invariants: once START has been written, every exit sends a best-effort
    /// STOP. Once acquisition is active, no error returns early: a serial,
    /// firmware, storage, or session-state failure becomes the terminal cause
    /// of the loop, and STOP, the bounded post-STOP drain, and finalization
    /// (BMEG, metadata, CSV) always run. A failed recording is never restarted
    /// or appended to.
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
            recording_context,
            expected_port_identity,
            provenance,
        } = request;
        // Tooling provenance is resolved before START (normally before the
        // port was opened) so no external command can delay raw intake.
        let provenance = provenance.unwrap_or_else(|| tooling_provenance(simulator, None));
        crate::app_log::record("INFO", "OPEN_RECORDING_FILE_BEGIN");
        let paths = self
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
        self.update_disk_space(initial_free_disk_bytes, 0)?;
        if initial_free_disk_bytes < DISK_CRITICAL_BYTES {
            return Err(SessionError::Storage(format!(
                "only {} MiB free in {}; recording requires at least {} MiB",
                initial_free_disk_bytes / (1024 * 1024),
                output_dir.display(),
                DISK_CRITICAL_BYTES / (1024 * 1024)
            )));
        }
        let field_count = profile.profile.acquisition.record_field_names().len();
        let csv_row_bytes = estimated_csv_row_bytes(&profile, &calibration);
        let mut initial_meta = build_initial_metadata(
            InitialMetadataRequest {
                simulator,
                source: &source,
                bmeg: &paths.bmeg,
                duration: &duration,
                profile: &profile,
                calibration,
                recording_path_context: recording_path_context.as_ref(),
                recording_context: &recording_context,
            },
            provenance,
        );
        initial_meta.initial_free_disk_bytes = Some(initial_free_disk_bytes);
        initial_meta.record_layout = Some(RecordLayout::Synchronized { field_count });
        // The 64 KiB header limit is proven before the board is configured, so
        // an oversized lab or note is never reported as a connection problem.
        check_synchronized_header(&initial_meta, field_count).map_err(|error| {
            crate::app_log::record(
                "WARN",
                &format!("START_FAIL stage=PREPARE_METADATA detail={error}"),
            );
            header_budget_error(error)
        })?;
        let (tx, rx) = sync_channel(4_096);
        let mut acquisition = AcquisitionController::new(tx);
        acquisition.set_expected_field_count(field_count);
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
        // Frames received so far may have been queued by a previous session
        // that ended without STOP (for example a watchdog ERROR 7). They are
        // logged, never counted against this recording; only errors after this
        // baseline (including a rejected CONFIGURE or START) end it.
        let handshake_snapshot = acquisition.snapshot();
        let firmware_error_baseline = handshake_snapshot.firmware_error_count;
        if firmware_error_baseline > 0 {
            crate::app_log::record(
                "WARN",
                &format!(
                    "STALE_FIRMWARE_ERROR count={} detail={}",
                    firmware_error_baseline,
                    firmware_error_detail(
                        handshake_snapshot.firmware_error_code,
                        handshake_snapshot.firmware_error_payload.as_deref(),
                    )
                ),
            );
        }
        let negotiated_capabilities = handshake_snapshot.firmware_capabilities;
        // An idle-safe STOP quiesces a board that may still be acquiring for a
        // previous session: its outputs go LOW now and CONFIGURE is never
        // applied mid-acquisition. The firmware answers with an acquiring=0
        // STATUS, which START confirmation ignores because it requires
        // acquiring=1.
        crate::app_log::record("INFO", "QUIESCE_STOP_SEND");
        self.send_command(io, MessageType::Stop, 0, vec![])
            .map_err(|error| {
                crate::app_log::record(
                    "WARN",
                    &format!("START_FAIL stage=QUIESCE_STOP detail={error}"),
                );
                error
            })?;
        crate::app_log::record("INFO", "CONFIGURE_SEND");
        let config_ack_baseline = acquisition.snapshot().config_ack_count;
        let configuration = configure_payload(
            &profile.profile.acquisition,
            negotiated_capabilities.as_ref(),
        )
        .map_err(|error| {
            crate::app_log::record(
                "WARN",
                &format!("START_FAIL stage=CONFIGURE_PAYLOAD detail={error}"),
            );
            self.note_start_failure(&error, ConnectionFailureCategory::LabNotSupportedByFirmware);
            error
        })?;
        self.send_command(io, MessageType::Configure, 1, configuration)
            .map_err(|error| {
                crate::app_log::record(
                    "WARN",
                    &format!("START_FAIL stage=CONFIGURE_SEND detail={error}"),
                );
                error
            })?;
        self.wait_until(
            io,
            &mut acquisition,
            move |snapshot| snapshot.config_ack_count > config_ack_baseline,
            "CONFIG_ACK",
            firmware_error_baseline,
        )
        .map_err(|error| {
            crate::app_log::record(
                "WARN",
                &format!("START_FAIL stage=CONFIG_ACK detail={error}"),
            );
            self.note_start_failure(&error, ConnectionFailureCategory::ConfigurationRejected);
            error
        })?;
        crate::app_log::record("INFO", "CONFIG_ACK_RECEIVED");
        acquisition.configure().map_err(SessionError::State)?;
        self.set_progress_state(SessionState::Configured)?;
        if self.cancel.load(Ordering::Acquire) {
            return Err(SessionError::Cancelled);
        }
        // `utc_start`/`local_start` describe acquisition, so they are stamped
        // as late as the header allows: immediately before START. The
        // pre-handshake time is kept as `session_opened_utc`.
        let start_stamp = Utc::now();
        initial_meta.utc_start = start_stamp;
        initial_meta.local_start = start_stamp.with_timezone(&Local);
        // The recording is open before START, so every validated post-start sample has a sink.
        let mut raw =
            BmegWriter::create_synchronized(&paths.temporary_bmeg, &initial_meta, field_count)
                .map_err(|error| {
                    crate::app_log::record(
                        "WARN",
                        &format!("START_FAIL stage=OPEN_RECORDING_FILE detail={error}"),
                    );
                    error
                })?;
        if let Err(error) = self.send_start_and_wait(io, &mut acquisition, firmware_error_baseline)
        {
            self.abandon_unstarted_recording(io, &rx, raw, &paths.temporary_bmeg);
            return Err(error);
        }
        // One Instant is the origin of the live elapsed/remaining display, the
        // timed stop, and host_elapsed_seconds.
        let started = Instant::now();
        let started_utc = Utc::now();
        self.note_runtime_error("SET_ACQUIRING", self.set_acquiring(started));
        crate::app_log::record("INFO", "RECORDING_ACTIVE");
        let active_digital_output_mask = acquisition.snapshot().digital_output_mask;
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
        let mut last_known_free_disk_bytes = initial_free_disk_bytes;
        let mut buffer = [0u8; 512];
        let mut terminal_error: Option<SerialTerminalFailure> = None;
        let mut host_failure: Option<HostCaptureFailure> = None;
        let mut terminal_failure_category: Option<ConnectionFailureCategory> = None;
        let mut stop_reason = None;
        let mut progress = CaptureProgress::default();
        progress.observe(&acquisition.snapshot(), started);

        loop {
            if self.cancel.load(Ordering::Acquire) {
                stop_reason = Some(
                    self.stop_reason()
                        .ok()
                        .flatten()
                        .unwrap_or(StopReason::User),
                );
                break;
            }
            if let RecordingDuration::Timed { seconds } = &duration {
                if started.elapsed() >= Duration::from_secs(*seconds) {
                    stop_reason = Some(
                        self.set_stop_reason_once(StopReason::TimedComplete)
                            .unwrap_or(StopReason::TimedComplete),
                    );
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
            if let Err(error) = self.drain_samples(&rx, &mut raw) {
                host_failure = Some(HostCaptureFailure::new(
                    HostFailureStage::RecordingWrite,
                    error.to_string(),
                ));
                break;
            }
            if let Err(error) = self.update_from_acquisition(&acquisition) {
                host_failure = Some(HostCaptureFailure::new(
                    HostFailureStage::SessionState,
                    error.to_string(),
                ));
                break;
            }
            let active_snapshot = acquisition.snapshot();
            let observed_now = Instant::now();
            progress.observe(&active_snapshot, observed_now);
            if active_snapshot.firmware_error_count > firmware_error_baseline {
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
            if last_flush.elapsed() >= self.timing.flush_interval {
                let flush_started = Instant::now();
                if let Err(error) = self.storage.flush(&mut raw) {
                    host_failure = Some(HostCaptureFailure::new(
                        HostFailureStage::RecordingFlush,
                        error.to_string(),
                    ));
                    break;
                }
                let now = Instant::now();
                last_flush = now;
                progress.last_recording_flush = Some(now);
                progress.last_recording_flush_duration = Some(now.duration_since(flush_started));
            }
            if last_disk_check.elapsed() >= self.timing.disk_check_interval {
                let disk_check_started = Instant::now();
                let csv_reserve_bytes = projected_csv_bytes(raw.records, csv_row_bytes);
                match self.free_disk_space(&output_dir) {
                    Ok(free) => {
                        last_known_free_disk_bytes = free;
                        self.note_runtime_error(
                            "DISK_STATUS",
                            self.update_disk_space(free, csv_reserve_bytes),
                        );
                        // Room is kept for the CSV that finalization exports
                        // from the BMEG, not only for the raw recording.
                        if free < storage_stop_threshold(csv_reserve_bytes) {
                            crate::app_log::record(
                                "WARN",
                                &format!(
                                    "STORAGE_GUARD free_disk_bytes={free} critical_bytes={DISK_CRITICAL_BYTES} csv_reserve_bytes={csv_reserve_bytes} records={}",
                                    raw.records
                                ),
                            );
                            stop_reason = Some(
                                self.set_stop_reason_once(StopReason::StorageGuard)
                                    .unwrap_or(StopReason::StorageGuard),
                            );
                            break;
                        }
                    }
                    Err(error) => {
                        // A transient query failure (removable, network, or
                        // cloud-backed folder) is not evidence of low space.
                        // Keep the last confirmed value and retry next interval.
                        crate::app_log::record(
                            "WARN",
                            &format!(
                                "DISK_SPACE_QUERY_FAILED detail={error} last_known_free_disk_bytes={last_known_free_disk_bytes}"
                            ),
                        );
                    }
                }
                let now = Instant::now();
                last_disk_check = now;
                progress.last_disk_check = Some(now);
                progress.last_disk_check_duration = Some(now.duration_since(disk_check_started));
            }
        }

        // The worker owns finalization from here, whatever ended the loop, so
        // the UI shows Stopping/saving and markers can no longer be added.
        self.note_runtime_error("SET_STOPPING", self.set_state(SessionState::Stopping));
        if let Some(error) = terminal_error.as_ref() {
            // This reads only the operating system's port list. It does not
            // reopen, reset, upload to, or otherwise touch the selected board.
            terminal_failure_category = self
                .record_active_serial_failure(
                    error,
                    ActiveSerialFailureContext {
                        port: &source,
                        expected_identity: expected_port_identity.as_ref(),
                        started,
                        started_utc,
                        progress: &progress,
                        snapshot: &acquisition.snapshot(),
                    },
                )
                .ok();
        }
        if let Some(failure) = host_failure.as_ref() {
            crate::app_log::record(
                "ERROR",
                &format!(
                    "RECORDING_WRITE_FAIL stage={} records_written={} detail={}",
                    failure.stage.label(),
                    raw.records,
                    failure.detail
                ),
            );
        }
        let reason = if terminal_error.is_some() {
            if matches!(
                terminal_failure_category,
                Some(ConnectionFailureCategory::DeviceDisconnected)
            ) {
                StopReason::Disconnect
            } else {
                StopReason::Fault
            }
        } else if host_failure.is_some() {
            StopReason::Fault
        } else {
            stop_reason.unwrap_or(StopReason::Fault)
        };
        // Committed now so a Stop click during finalization cannot relabel a
        // fault as a user stop.
        self.note_runtime_error("STOP_REASON", self.commit_terminal_stop_reason(reason));

        let stop_status_baseline = acquisition.snapshot().status_count;
        let stop_sent = match self.send_capture_command(io, MessageType::Stop, 4, vec![]) {
            Ok(()) => true,
            Err(CaptureCommandError::Serial(error)) => {
                self.record_serial_cleanup_failure(
                    &error,
                    &source,
                    started,
                    started_utc,
                    &progress,
                );
                false
            }
            Err(CaptureCommandError::Protocol(detail)) => {
                crate::app_log::record(
                    "WARN",
                    &format!("SERIAL_CLEANUP_ERROR stage=STOP_PROTOCOL detail={detail}"),
                );
                false
            }
        };
        // Read until the firmware's STATUS sent after this STOP arrives (or
        // the bounded deadline passes). It follows the final partial batch and
        // every queued batch, so the recording's tail is drained too. An
        // existing mask of 0 proves nothing: labs with no outputs report 0 at
        // START.
        let host_failure_before_stop = host_failure.is_some();
        let mut stop_confirmed = false;
        let stop_status_deadline = Instant::now() + STOP_STATUS_TIMEOUT;
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
            // Drain while waiting so tail batches never fill the bounded channel.
            if host_failure.is_none() {
                if let Err(error) = self.drain_samples(&rx, &mut raw) {
                    host_failure = Some(HostCaptureFailure::new(
                        HostFailureStage::RecordingWrite,
                        error.to_string(),
                    ));
                }
            }
            if stop_is_confirmed(&acquisition.snapshot(), stop_status_baseline) {
                stop_confirmed = true;
                break;
            }
        }
        // After a sink failure nothing more is written: the buffer then holds
        // only whole records, and further writes could not be trusted.
        if host_failure.is_none() {
            if let Err(error) = self.drain_samples(&rx, &mut raw) {
                host_failure = Some(HostCaptureFailure::new(
                    HostFailureStage::RecordingWrite,
                    error.to_string(),
                ));
            }
        }
        if !host_failure_before_stop {
            if let Some(failure) = host_failure.as_ref() {
                crate::app_log::record(
                    "ERROR",
                    &format!(
                        "RECORDING_WRITE_FAIL stage={} during=post_stop records_written={} detail={}",
                        failure.stage.label(),
                        raw.records,
                        failure.detail
                    ),
                );
            }
        }
        let records_written = raw.records;
        crate::app_log::record(
            if stop_confirmed { "INFO" } else { "WARN" },
            &format!(
                "STOP_CONFIRMATION confirmed={stop_confirmed} stop_sent={stop_sent} records_written={records_written}"
            ),
        );
        let mut snapshot = acquisition.snapshot();
        let final_digital_output_mask = if stop_confirmed {
            snapshot.digital_output_mask
        } else {
            None
        };
        if terminal_error.is_some()
            && matches!(
                terminal_failure_category,
                Some(ConnectionFailureCategory::DeviceDisconnected)
            )
        {
            snapshot.integrity.disconnect_events += 1;
        }
        self.note_runtime_error("SNAPSHOT", self.update_snapshot(&snapshot));
        if !stop_confirmed {
            // Without a STATUS after STOP the outputs are unconfirmed; do not
            // keep showing the mask reported at START.
            self.note_runtime_error("OUTPUT_MASK", self.set_digital_output_mask(None));
        }
        let host_elapsed = started.elapsed().as_secs_f64();
        let board_elapsed = board_elapsed_seconds(&snapshot);
        let csv_bytes_estimate = projected_csv_bytes(records_written, csv_row_bytes);
        let final_free_disk_bytes = self.free_disk_space(&output_dir).ok();
        if let Some(free) = final_free_disk_bytes {
            self.note_runtime_error(
                "DISK_STATUS",
                self.update_disk_space(free, csv_bytes_estimate),
            );
        }
        let terminal_error_detail = terminal_error.as_ref().map(SerialTerminalFailure::detail);
        let recording_failure_detail = host_failure.as_ref().map(HostCaptureFailure::describe);
        let terminal_trigger = if terminal_error_detail.is_some() {
            "serial_error"
        } else if recording_failure_detail.is_some() {
            "recording_write_error"
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
            if terminal_error_detail.is_some() || recording_failure_detail.is_some() {
                "ERROR"
            } else {
                "INFO"
            },
            &format!(
                "RECORDING_TERMINAL trigger={} stop_reason={:?} cancel={} terminal_error={} recording_error={} elapsed_ms={} port={} samples={} records_written={} packets={} free_disk_bytes={:?} stop_confirmed={}",
                terminal_trigger,
                reason,
                self.cancel.load(Ordering::Acquire),
                terminal_error_detail.as_deref().unwrap_or("none"),
                recording_failure_detail.as_deref().unwrap_or("none"),
                (host_elapsed * 1_000.0).round() as u128,
                source,
                snapshot.sample_count,
                records_written,
                snapshot.integrity.received_packets,
                final_free_disk_bytes,
                stop_confirmed,
            ),
        );
        let mut final_meta = initial_meta;
        final_meta.utc_stop = Some(Utc::now());
        final_meta.local_stop = Some(Local::now());
        final_meta.acquisition_started_utc = Some(started_utc);
        final_meta.host_elapsed_seconds = Some(host_elapsed);
        final_meta.board_elapsed_seconds = Some(board_elapsed);
        final_meta.measured_sample_rate_hz = snapshot.measured_rate_hz;
        final_meta.timing_diagnostics = Some(acquisition.timing_diagnostics(
            profile.profile.acquisition.sample_rate_hz,
            matches!(
                profile.profile.acquisition.acquisition_mode,
                crate::profiles::AcquisitionMode::Pulseox4State
            ),
        ));
        // The count of records actually in the BMEG. Without a sink failure
        // it equals every sample the acquisition accepted.
        final_meta.total_samples = records_written;
        final_meta.integrity = snapshot.integrity.clone();
        final_meta.recording_status = reason.recording_status().into();
        final_meta.duration_mode = Some(duration.label().into());
        final_meta.requested_duration_seconds = duration.requested_seconds();
        final_meta.stop_reason = Some(reason);
        final_meta.initial_free_disk_bytes = Some(initial_free_disk_bytes);
        final_meta.final_free_disk_bytes = final_free_disk_bytes;
        // State is Stopping, so no marker can be added after this copy.
        final_meta.markers = self
            .lock_runtime()
            .map(|runtime| runtime.markers.clone())
            .unwrap_or_default();
        let FinalizeOutcome {
            raw_path,
            metadata_written,
            csv_written,
            csv_error,
            finalize_error,
            failures,
        } = self.finalize_recording(
            raw,
            &paths,
            &mut final_meta,
            FinalizePlan {
                reason,
                recording_failure: recording_failure_detail.clone(),
                records_written,
                csv_bytes_estimate,
                free_bytes: final_free_disk_bytes,
            },
        );
        let save_problem = save_failure_message(
            recording_failure_detail.as_deref(),
            &failures,
            &raw_path,
            &paths,
        );
        let summary = SessionSummary {
            state: if terminal_error_detail.is_some() || save_problem.is_some() {
                SessionState::Faulted
            } else {
                SessionState::Disconnected
            },
            samples: records_written,
            packets: snapshot.integrity.received_packets,
            measured_rate_hz: snapshot.measured_rate_hz,
            board_elapsed_seconds: board_elapsed,
            host_elapsed_seconds: host_elapsed,
            bmeg_path: raw_path.display().to_string(),
            csv_path: if csv_written {
                paths.csv.display().to_string()
            } else {
                String::new()
            },
            metadata_path: if metadata_written {
                paths.metadata.display().to_string()
            } else {
                String::new()
            },
            recording_status: reason.recording_status().into(),
            duration: duration.clone(),
            stop_reason: reason,
            completion_status: final_meta.completion_status.clone(),
            initial_free_disk_bytes: Some(initial_free_disk_bytes),
            final_free_disk_bytes,
            integrity: snapshot.integrity,
            error: terminal_error_detail
                .clone()
                .or_else(|| recording_failure_detail.clone()),
            profile,
            calibration: final_meta.calibration.clone().unwrap_or_default(),
            active_digital_output_mask,
            final_digital_output_mask,
            csv_error,
            finalize_error,
        };
        self.note_runtime_error("SUMMARY", self.set_summary(summary));
        self.note_runtime_error("STOP_REASON", self.set_runtime_stop_reason(reason));
        if let Some(error) = terminal_error {
            // Preserve the stage/kind/Windows error in Advanced details rather
            // than overwriting it with the generic I/O display string.
            let detail = terminal_error_detail.unwrap_or_else(|| error.detail());
            self.set_fault(
                FaultOrigin::Recording,
                match save_problem {
                    Some(problem) => format!("{detail} Saving: {problem}"),
                    None => detail,
                },
            )?;
            return Err(error.into_session_error());
        }
        if let Some(problem) = save_problem {
            self.note_runtime_error("FAILURE_CATEGORY", self.mark_recording_storage_failure());
            self.set_fault(FaultOrigin::Storage, problem.clone())?;
            return Err(SessionError::Save(problem));
        }
        self.set_state(SessionState::Disconnected)?;
        Ok(())
    }

    /// Arms the controller, sends START, and waits for the firmware's
    /// acquiring STATUS (newer than any stale frame by construction).
    fn send_start_and_wait<T: Read + Write>(
        &self,
        io: &mut T,
        acquisition: &mut AcquisitionController,
        firmware_error_baseline: u64,
    ) -> Result<(), SessionError> {
        acquisition.arm_start().map_err(SessionError::State)?;
        crate::app_log::record("INFO", "START_SEND");
        self.send_command(io, MessageType::Start, 2, vec![])
            .map_err(|error| {
                crate::app_log::record(
                    "WARN",
                    &format!("START_FAIL stage=START_SEND detail={error}"),
                );
                error
            })?;
        self.wait_until(
            io,
            acquisition,
            |snapshot| snapshot.state == AcquisitionState::Running,
            "START status",
            firmware_error_baseline,
        )
        .map_err(|error| {
            crate::app_log::record(
                "WARN",
                &format!("START_FAIL stage=START_ACK_OR_FIRST_SAMPLE detail={error}"),
            );
            self.note_start_failure(&error, ConnectionFailureCategory::StartNotConfirmed);
            error
        })
    }

    /// START may have reached the firmware even though its acknowledgement
    /// did not arrive (timeout, cancel, or a write error). A best-effort STOP
    /// returns the outputs LOW now instead of at the firmware watchdog, and
    /// the header-only `.part` is removed. A file that holds records is kept.
    fn abandon_unstarted_recording<T: Write>(
        &self,
        io: &mut T,
        rx: &Receiver<SynchronizedRecord>,
        mut raw: BmegWriter,
        temporary_bmeg: &Path,
    ) {
        match self.send_capture_command(io, MessageType::Stop, 4, vec![]) {
            Ok(()) => crate::app_log::record("INFO", "START_ABORT_STOP_SENT"),
            Err(CaptureCommandError::Serial(error)) => crate::app_log::record(
                "WARN",
                &format!(
                    "SERIAL_CLEANUP_ERROR stage=START_ABORT_STOP detail={}",
                    error.detail()
                ),
            ),
            Err(CaptureCommandError::Protocol(detail)) => crate::app_log::record(
                "WARN",
                &format!("SERIAL_CLEANUP_ERROR stage=START_ABORT_STOP_PROTOCOL detail={detail}"),
            ),
        }
        if let Err(error) = self.drain_samples(rx, &mut raw) {
            crate::app_log::record("WARN", &format!("START_ABORT_DRAIN_FAILED detail={error}"));
        }
        let records = raw.records;
        if let Err(error) = self.storage.finish(raw) {
            crate::app_log::record("WARN", &format!("START_ABORT_FLUSH_FAILED detail={error}"));
        }
        if records == 0 {
            match fs::remove_file(temporary_bmeg) {
                Ok(()) => crate::app_log::record(
                    "INFO",
                    &format!(
                        "START_ABORT_EMPTY_RECORDING_REMOVED path={}",
                        temporary_bmeg.display()
                    ),
                ),
                Err(error) => crate::app_log::record(
                    "WARN",
                    &format!(
                        "START_ABORT_EMPTY_RECORDING_REMOVE_FAILED path={} detail={error}",
                        temporary_bmeg.display()
                    ),
                ),
            }
        } else {
            crate::app_log::record(
                "WARN",
                &format!(
                    "START_ABORT_PARTIAL_RECORDING_KEPT path={} records={records}",
                    temporary_bmeg.display()
                ),
            );
        }
    }

    /// Finalizes in a crash-safe order: finish and fsync the BMEG, rename it
    /// (bounded retry), write the metadata sidecar, export the CSV as best
    /// effort, then rewrite the sidecar with the CSV result. Every step is
    /// attempted, failures are logged as `FINALIZE_FAIL stage=...`, and raw
    /// data are never deleted.
    fn finalize_recording(
        &self,
        raw: BmegWriter,
        paths: &RecordingPaths,
        meta: &mut RecordingMetadata,
        plan: FinalizePlan,
    ) -> FinalizeOutcome {
        let mut failures = Vec::new();
        let mut raw_path = paths.temporary_bmeg.clone();
        let data_flushed = match self.storage.finish(raw) {
            Ok(()) => true,
            Err(error) => {
                failures.push(FinalizeFailure::new(
                    FinalizeStage::Flush,
                    error.to_string(),
                ));
                false
            }
        };
        // A file whose final flush failed may end in a partial record; it
        // keeps its `.part` name so it is never mistaken for a finished BMEG.
        if data_flushed {
            match self.rename_with_retry(&paths.temporary_bmeg, &paths.bmeg) {
                Ok(()) => raw_path = paths.bmeg.clone(),
                Err(error) => failures.push(FinalizeFailure::new(
                    FinalizeStage::Rename,
                    describe_io_error(&error),
                )),
            }
        }
        if let Ok(name) = file_name_string(&raw_path) {
            meta.bmeg_filename = name;
        }
        let data_complete = data_flushed && plan.recording_failure.is_none();
        meta.completion_status = if plan.reason.is_complete() && data_complete {
            "complete"
        } else {
            "incomplete"
        }
        .into();
        meta.csv_filename = None;
        meta.csv_error = None;
        meta.finalize_error = finalize_error_text(plan.recording_failure.as_deref(), &failures);
        // The sidecar is written before the CSV so a CSV failure (for example
        // a full disk) can never leave the recording without metadata.
        let first_metadata = self.write_metadata(&paths.metadata, meta);

        let mut csv_written = false;
        let csv_error = if data_flushed {
            if plan
                .free_bytes
                .is_some_and(|free| free < plan.csv_bytes_estimate)
            {
                crate::app_log::record(
                    "WARN",
                    &format!(
                        "CSV_EXPORT_LOW_SPACE free_disk_bytes={:?} estimated_csv_bytes={}",
                        plan.free_bytes, plan.csv_bytes_estimate
                    ),
                );
            }
            match self.export_csv_atomically(&raw_path, &paths.csv, plan.records_written) {
                Ok(()) => {
                    csv_written = true;
                    None
                }
                Err(detail) => {
                    failures.push(FinalizeFailure::new(FinalizeStage::Csv, detail.clone()));
                    Some(detail)
                }
            }
        } else {
            Some(
                "CSV export skipped because the raw recording could not be completely written"
                    .into(),
            )
        };
        meta.csv_filename = if csv_written {
            file_name_string(&paths.csv).ok()
        } else {
            None
        };
        meta.csv_error = csv_error.clone();
        meta.finalize_error = finalize_error_text(plan.recording_failure.as_deref(), &failures);
        let metadata_written = match (first_metadata, self.write_metadata(&paths.metadata, meta)) {
            (_, Ok(())) => true,
            (Ok(()), Err(error)) => {
                failures.push(FinalizeFailure::new(
                    FinalizeStage::Metadata,
                    format!("the CSV result could not be added to the metadata: {error}"),
                ));
                true
            }
            (Err(_), Err(error)) => {
                failures.push(FinalizeFailure::new(FinalizeStage::Metadata, error));
                false
            }
        };
        // Recomputed after the last sidecar write so the summary also reports
        // a metadata failure that the sidecar on disk could not record.
        let finalize_error = finalize_error_text(plan.recording_failure.as_deref(), &failures);
        for failure in &failures {
            crate::app_log::record(
                "ERROR",
                &format!(
                    "FINALIZE_FAIL stage={} raw_file={} detail={}",
                    failure.stage.label(),
                    raw_path.display(),
                    failure.detail
                ),
            );
        }
        FinalizeOutcome {
            raw_path,
            metadata_written,
            csv_written,
            csv_error,
            finalize_error,
            failures,
        }
    }

    /// Writes the sidecar to a temporary sibling, fsyncs it, and renames it
    /// into place, so rewriting it with the CSV result can never destroy a
    /// valid sidecar that was already written (for example on a full disk).
    fn write_metadata(&self, path: &Path, meta: &RecordingMetadata) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(meta).map_err(|error| error.to_string())?;
        let temporary = sibling_with_suffix(path, ".part");
        if let Err(error) = self.storage.write_durably(&temporary, &bytes) {
            let _ = fs::remove_file(&temporary);
            return Err(describe_io_error(&error));
        }
        self.rename_with_retry(&temporary, path).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            describe_io_error(&error)
        })
    }

    /// Exports to a temporary sibling and renames it only when the row count
    /// matches the raw records, so a partial CSV never appears under the final
    /// name. A failed export's temporary file is removed; the BMEG is not.
    fn export_csv_atomically(
        &self,
        raw_path: &Path,
        csv: &Path,
        expected_rows: u64,
    ) -> Result<(), String> {
        let temporary = sibling_with_suffix(csv, ".part");
        let result = match self.storage.export_csv(raw_path, &temporary) {
            Ok(rows) if rows == expected_rows => {
                self.rename_with_retry(&temporary, csv).map_err(|error| {
                    format!(
                        "the exported CSV could not be renamed: {}",
                        describe_io_error(&error)
                    )
                })
            }
            Ok(rows) => Err(format!(
                "the CSV has {rows} rows but the raw recording has {expected_rows} records"
            )),
            Err(error) => Err(error.to_string()),
        };
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    /// Bounded retry with exponential backoff for transient sharing
    /// violations (OneDrive, indexer, antivirus) on a just-closed file.
    fn rename_with_retry(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        let attempts = self.timing.rename_attempts.max(1);
        let mut backoff = self.timing.rename_initial_backoff;
        let mut attempt = 1;
        loop {
            match self.storage.rename(from, to) {
                Ok(()) => return Ok(()),
                Err(error) if attempt < attempts => {
                    crate::app_log::record(
                        "WARN",
                        &format!(
                            "FINALIZE_RENAME_RETRY attempt={attempt} from={} to={} detail={}",
                            from.display(),
                            to.display(),
                            describe_io_error(&error)
                        ),
                    );
                    thread::sleep(backoff);
                    backoff = backoff.saturating_mul(2);
                    attempt += 1;
                }
                Err(error) => return Err(error),
            }
        }
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
        let mut ping_write_timeouts = 0u32;
        let mut bytes_received = 0u64;
        let mut valid_packets_seen = 0u64;
        let mut buffer = [0u8; 256];

        while Instant::now() < deadline {
            if self.cancel.load(Ordering::Acquire) {
                return Err(SessionError::Cancelled);
            }
            if ping_attempts < ping_attempt_limit && Instant::now() >= next_ping {
                let sequence = ping_attempts;
                ping_attempts += 1;
                next_ping += ping_interval;
                match self.send_command(io, MessageType::Ping, sequence, vec![]) {
                    Ok(()) => {}
                    // A timed-out PING write is one failed attempt, not the
                    // end of the handshake: Windows can abort a short CDC
                    // write while the board is still sending frames.
                    Err(SessionError::Io(error))
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                        ) =>
                    {
                        ping_write_timeouts += 1;
                        crate::app_log::record(
                            "WARN",
                            &format!(
                                "HANDSHAKE_PING_WRITE_TIMEOUT attempt={ping_attempts} error_kind={:?} raw_os_error={:?} detail={error}",
                                error.kind(),
                                error.raw_os_error(),
                            ),
                        );
                    }
                    Err(error) => {
                        let category = if matches!(error, SessionError::Io(_)) {
                            self.handshake_io_failure_category()
                        } else {
                            ConnectionFailureCategory::HandshakeIncomplete
                        };
                        self.update_handshake_diagnostics(
                            &acquisition.snapshot(),
                            bytes_received,
                            ping_attempts,
                            started.elapsed(),
                            Some(category),
                        )?;
                        return Err(error);
                    }
                }
            }
            match io.read(&mut buffer) {
                Ok(n) if n > 0 => {
                    bytes_received += n as u64;
                    acquisition.ingest_bytes(&buffer[..n]);
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => {
                    // A read error alone does not prove the board was removed;
                    // a read-only enumeration decides, as in active capture.
                    let category = self.handshake_io_failure_category();
                    let snapshot = acquisition.snapshot();
                    self.update_handshake_diagnostics(
                        &snapshot,
                        bytes_received,
                        ping_attempts,
                        started.elapsed(),
                        Some(category),
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
        // When nothing arrived and PING writes kept timing out, the serial
        // transport, not the firmware, is the likely problem.
        let category = if ping_write_timeouts > 0 && bytes_received == 0 {
            ConnectionFailureCategory::SerialTransportError
        } else {
            handshake_failure_category(&snapshot, bytes_received)
        };
        self.update_handshake_diagnostics(
            &snapshot,
            bytes_received,
            ping_attempts,
            started.elapsed(),
            Some(category.clone()),
        )?;
        Err(SessionError::Protocol(format!(
            "handshake failed on {} after {} ms: {:?}; bytes={}, ping_write_timeouts={}, valid_frames={}, CRC failures={}, HELLO={}, CAPABILITIES={}, PONG={}. {}",
            self.status()?.port,
            started.elapsed().as_millis(),
            category,
            bytes_received,
            ping_write_timeouts,
            snapshot.integrity.received_packets,
            snapshot.integrity.crc_failures,
            snapshot.hello_seen,
            snapshot.capabilities_seen,
            snapshot.pong_seen,
            recommended_action(&category),
        )))
    }

    /// Waits for a command's response, named by `name` (the awaited frame,
    /// not the command). `predicate` must compare against a baseline taken
    /// before the command was sent, so a stale frame from a previous session
    /// cannot satisfy it. A firmware ERROR newer than
    /// `firmware_error_baseline` fails the wait immediately.
    fn wait_until<T: Read + Write, F: Fn(&AcquisitionSnapshot) -> bool>(
        &self,
        io: &mut T,
        acquisition: &mut AcquisitionController,
        predicate: F,
        name: &str,
        firmware_error_baseline: u64,
    ) -> Result<(), SessionError> {
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        let mut buffer = [0u8; 256];
        while Instant::now() < deadline {
            if self.cancel.load(Ordering::Acquire) {
                return Err(SessionError::Cancelled);
            }
            match io.read(&mut buffer) {
                Ok(n) if n > 0 => acquisition.ingest_bytes(&buffer[..n]),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => return Err(SessionError::Io(error)),
            }
            let snapshot = acquisition.snapshot();
            self.update_snapshot(&snapshot)?;
            if snapshot.firmware_error_count > firmware_error_baseline {
                return Err(SessionError::Protocol(format!(
                    "firmware ERROR while waiting for {name}: {}",
                    firmware_error_detail(
                        snapshot.firmware_error_code,
                        snapshot.firmware_error_payload.as_deref(),
                    )
                )));
            }
            if predicate(&snapshot) {
                return Ok(());
            }
        }
        let snapshot = acquisition.snapshot();
        Err(SessionError::Protocol(format!(
            "timed out waiting for {name} (HELLO={}, CAPABILITIES={}, PONG={}, CONFIG_ACK={}, STATUS={}, acquiring={:?})",
            snapshot.hello_seen,
            snapshot.capabilities_seen,
            snapshot.pong_seen,
            snapshot.config_ack_seen,
            snapshot.status_seen,
            snapshot.last_status_acquiring,
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

    #[cfg(test)]
    fn initial_metadata(
        &self,
        request: InitialMetadataRequest<'_>,
    ) -> Result<RecordingMetadata, SessionError> {
        let provenance = tooling_provenance(request.simulator, None);
        Ok(build_initial_metadata(request, provenance))
    }

    fn allocate_paths(
        &self,
        output_dir: &Path,
        profile: &ProfileSnapshot,
    ) -> Result<RecordingPaths, SessionError> {
        // A folder problem is a storage problem. It must not surface as
        // `SessionError::Io`, which is how serial transport errors arrive.
        fs::create_dir_all(output_dir).map_err(|error| {
            SessionError::Storage(format!(
                "could not create the recording folder {}: {}",
                output_dir.display(),
                describe_io_error(&error)
            ))
        })?;
        let stamp = Local::now().format("%Y%m%d_%H%M%S");
        for run in 1..=99 {
            let name =
                crate::profiles::safe_filename_component(&profile.profile.export.signal_name);
            let base = output_dir.join(format!("{stamp}_{name}_Run{run:02}"));
            let paths = RecordingPaths {
                temporary_bmeg: base.with_extension("bmeg.part"),
                bmeg: base.with_extension("bmeg"),
                csv: base.with_extension("csv"),
                metadata: base.with_extension("metadata.json"),
            };
            // Existing files, including temporary siblings left by an
            // interrupted save, are never overwritten.
            let occupied = [
                &paths.temporary_bmeg,
                &paths.bmeg,
                &paths.csv,
                &paths.metadata,
            ]
            .into_iter()
            .any(|path| path.exists() || sibling_with_suffix(path, ".part").exists());
            if !occupied {
                return Ok(paths);
            }
        }
        Err(SessionError::Storage(
            "could not allocate a unique recording filename; choose another Output folder".into(),
        ))
    }

    fn set_state(&self, state: SessionState) -> Result<(), SessionError> {
        self.lock_runtime()?.state = state;
        Ok(())
    }

    /// Advances a connecting worker's state without undoing a Stop request
    /// that arrived meanwhile (which already set `Stopping`).
    fn set_progress_state(&self, state: SessionState) -> Result<(), SessionError> {
        let mut runtime = self.lock_runtime()?;
        if runtime.state != SessionState::Stopping {
            runtime.state = state;
        }
        Ok(())
    }

    /// Enters `Acquiring` and fixes the live elapsed-time origin under one
    /// lock, using the same Instant the worker uses for the timed stop and
    /// `host_elapsed_seconds`, so Elapsed/Remaining are measured from START.
    fn set_acquiring(&self, started: Instant) -> Result<(), SessionError> {
        let mut runtime = self.lock_runtime()?;
        runtime.started_at = Some(started);
        if runtime.state != SessionState::Stopping {
            runtime.state = SessionState::Acquiring;
        }
        Ok(())
    }

    fn set_digital_output_mask(&self, mask: Option<u8>) -> Result<(), SessionError> {
        self.lock_runtime()?.digital_output_mask = mask;
        Ok(())
    }

    /// Best-effort runtime updates after START must never skip STOP or
    /// finalization; a failure (only possible through lock poisoning) is logged.
    fn note_runtime_error(&self, stage: &str, result: Result<(), SessionError>) {
        if let Err(error) = result {
            crate::app_log::record(
                "WARN",
                &format!("SESSION_STATE_UPDATE_FAILED stage={stage} detail={error}"),
            );
        }
    }

    /// Labels a save failure as a storage problem so the recommended action
    /// does not send the student to firmware or serial recovery.
    fn mark_recording_storage_failure(&self) -> Result<(), SessionError> {
        let mut runtime = self.lock_runtime()?;
        if let Some(diagnostics) = runtime.connection_diagnostics.as_mut() {
            let category = ConnectionFailureCategory::RecordingStorageError;
            diagnostics.recommended_action = recommended_action(&category).into();
            diagnostics.failure_category = Some(category);
        }
        Ok(())
    }

    fn set_fault(&self, origin: FaultOrigin, error: String) -> Result<(), SessionError> {
        let mut runtime = self.lock_runtime()?;
        runtime.state = SessionState::Faulted;
        runtime.last_error = Some(error);
        runtime.fault_origin = Some(origin);
        Ok(())
    }

    /// Records a connection failure category for a hardware session.
    /// `replace` overwrites a category recorded earlier in the same attempt;
    /// otherwise the first, more specific category is kept.
    fn note_failure_category(
        &self,
        category: ConnectionFailureCategory,
        replace: bool,
    ) -> Result<(), SessionError> {
        let mut runtime = self.lock_runtime()?;
        if let Some(diagnostics) = runtime.connection_diagnostics.as_mut() {
            if replace || diagnostics.failure_category.is_none() {
                diagnostics.recommended_action = recommended_action(&category).into();
                diagnostics.failure_category = Some(category);
            }
        }
        Ok(())
    }

    /// Labels a pre-START configuration failure by its stage. Serial I/O and
    /// cancellation keep their own handling (transport category, quiet stop).
    fn note_start_failure(&self, error: &SessionError, category: ConnectionFailureCategory) {
        if matches!(error, SessionError::Protocol(_) | SessionError::State(_)) {
            self.note_runtime_error(
                "FAILURE_CATEGORY",
                self.note_failure_category(category, false),
            );
        }
    }

    /// Category for a serial read/write error during the handshake. Only a
    /// read-only enumeration that no longer lists the port proves removal.
    fn handshake_io_failure_category(&self) -> ConnectionFailureCategory {
        let port = self
            .lock_runtime()
            .map(|runtime| runtime.port.clone())
            .unwrap_or_default();
        category_after_io_failure(&diagnose_port_enumeration(&port, None))
    }

    /// Only a genuine cancellation (a stop requested before acquisition began)
    /// ends a worker quietly. Every other error, including a save failure
    /// after the user pressed Stop, leaves the session `Faulted` with a
    /// message, even though the cancel flag is set.
    fn finish_worker_error(&self, result: &Result<(), SessionError>) -> Result<(), SessionError> {
        let Err(error) = result else {
            return Ok(());
        };
        if matches!(error, SessionError::Cancelled) {
            self.set_state(SessionState::Disconnected)?;
            return Ok(());
        }
        // Active capture may already have stored a more useful serial stage,
        // ErrorKind, Windows code, or save stage. Do not replace it (or its
        // category) with a generic rendering as the worker unwinds.
        let detailed_fault_is_present = {
            let runtime = self.lock_runtime()?;
            runtime.state == SessionState::Faulted && runtime.last_error.is_some()
        };
        if !detailed_fault_is_present {
            self.record_worker_failure_category(error)?;
            crate::app_log::record("ERROR", &format!("SESSION_WORKER_FAIL detail={error}"));
            self.set_fault(worker_fault_origin(error), error.to_string())?;
        }
        Ok(())
    }

    fn record_worker_failure_category(&self, error: &SessionError) -> Result<(), SessionError> {
        let (port, port_opened) = {
            let mut runtime = self.lock_runtime()?;
            let port = runtime.port.clone();
            let Some(diagnostics) = runtime.connection_diagnostics.as_mut() else {
                return Ok(());
            };
            if let SessionError::InvalidRequest(message) = error {
                // The request (for example an oversized lab header) was
                // refused; the board and connection are not at fault, so no
                // connection category or firmware advice is recorded.
                diagnostics.recommended_action = (*message).into();
                return Ok(());
            }
            if diagnostics.failure_category.is_some() {
                return Ok(());
            }
            (port, diagnostics.port_opened)
        };
        let category = if matches!(
            error,
            SessionError::Storage(_) | SessionError::Recording(_) | SessionError::Save(_)
        ) {
            ConnectionFailureCategory::RecordingStorageError
        } else if !port_opened {
            // Windows reports a port held by another program and a removed
            // device alike; a read-only enumeration (outside the lock) decides.
            port_open_failure_category(&port)
        } else if matches!(error, SessionError::Io(_)) {
            // An I/O error by itself does not prove that Windows
            // removed the USB device. Active capture records a
            // read-only enumeration before assigning DeviceDisconnected.
            ConnectionFailureCategory::SerialTransportError
        } else {
            ConnectionFailureCategory::HandshakeIncomplete
        };
        self.note_failure_category(category, false)
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

    fn set_connection_diagnostics(
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
            recording_active: is_active_state(runtime.state) && !runtime.handshake_check,
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
            fault_origin: runtime.fault_origin,
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

    /// Publishes free space and the low-storage warning. `csv_reserve_bytes`
    /// is the room kept for the CSV export, so the stop threshold is
    /// `DISK_CRITICAL_BYTES + csv_reserve_bytes`. The warning keeps the 1 GiB
    /// rule and also appears with the same margin above a larger threshold,
    /// so a student is warned before the guard stops a long recording.
    fn update_disk_space(&self, bytes: u64, csv_reserve_bytes: u64) -> Result<(), SessionError> {
        let stop_threshold = storage_stop_threshold(csv_reserve_bytes);
        let warning_threshold = DISK_WARNING_BYTES
            .max(stop_threshold.saturating_add(DISK_WARNING_BYTES - DISK_CRITICAL_BYTES));
        let mut runtime = self.lock_runtime()?;
        runtime.available_disk_bytes = Some(bytes);
        runtime.storage_warning = if bytes < warning_threshold {
            Some(if csv_reserve_bytes == 0 {
                format!(
                    "Low disk space: {} MiB available; acquisition will stop below {} MiB.",
                    bytes / (1024 * 1024),
                    DISK_CRITICAL_BYTES / (1024 * 1024)
                )
            } else {
                format!(
                    "Low disk space: {} MiB available; acquisition will stop below {} MiB ({} MiB plus room for the CSV export).",
                    bytes / (1024 * 1024),
                    stop_threshold / (1024 * 1024),
                    DISK_CRITICAL_BYTES / (1024 * 1024)
                )
            })
        } else {
            None
        };
        Ok(())
    }

    /// Records the reason a recording actually ended, overriding an earlier
    /// request for Fault/Disconnect. A Stop click after the loop ended (during
    /// finalization) then cannot relabel the recording as a user stop.
    fn commit_terminal_stop_reason(&self, reason: StopReason) -> Result<(), SessionError> {
        *self
            .stop_reason
            .lock()
            .map_err(|_| SessionError::State("stop reason lock poisoned"))? = Some(reason);
        self.set_runtime_stop_reason(reason)
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

/// A session (a recording, or a handshake check) owns the serial transport.
fn is_active_state(state: SessionState) -> bool {
    matches!(
        state,
        SessionState::Connecting
            | SessionState::Connected
            | SessionState::Configured
            | SessionState::Acquiring
            | SessionState::Stopping
    )
}

fn file_name_string(path: &Path) -> Result<String, SessionError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or(SessionError::State("recording filename is invalid"))
}

/// `X.csv` + `.part` -> `X.csv.part`, used for files written then renamed.
fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn describe_io_error(error: &std::io::Error) -> String {
    format!(
        "{error} (kind={:?}, raw_os_error={:?})",
        error.kind(),
        error.raw_os_error()
    )
}

fn display_file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The sidecar's `finalize_error`: problems that affect the raw file or the
/// sidecar itself. CSV problems are reported separately in `csv_error`.
fn finalize_error_text(
    recording_failure: Option<&str>,
    failures: &[FinalizeFailure],
) -> Option<String> {
    let parts: Vec<String> = recording_failure
        .map(|detail| format!("recording: {detail}"))
        .into_iter()
        .chain(
            failures
                .iter()
                .filter(|failure| failure.stage != FinalizeStage::Csv)
                .map(|failure| format!("{}: {}", failure.stage.label(), failure.detail)),
        )
        .collect();
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// Student-facing, stage-specific explanation of what could not be saved and
/// which files were kept. `None` when the recording was saved completely.
fn save_failure_message(
    recording_failure: Option<&str>,
    failures: &[FinalizeFailure],
    raw_path: &Path,
    paths: &RecordingPaths,
) -> Option<String> {
    let raw_name = display_file_name(raw_path);
    let mut messages = Vec::new();
    if let Some(detail) = recording_failure {
        messages.push(format!(
            "The recording stopped early because the recording file could not be written ({detail}). The samples already saved were kept in {raw_name}."
        ));
    }
    for failure in failures {
        let detail = &failure.detail;
        messages.push(match failure.stage {
            FinalizeStage::Flush => format!(
                "The end of the recording could not be written to disk ({detail}). The partial raw file {raw_name} was kept; check that the drive is connected and has free space."
            ),
            FinalizeStage::Rename => format!(
                "The recording was kept as {raw_name} because it could not be renamed to {} ({detail}). Another program, such as OneDrive or antivirus, may have been using it.",
                display_file_name(&paths.bmeg)
            ),
            FinalizeStage::Metadata => format!(
                "The metadata file {} could not be written ({detail}).",
                display_file_name(&paths.metadata)
            ),
            FinalizeStage::Csv => format!(
                "The CSV export failed ({detail}). The raw recording {raw_name} and its metadata were kept."
            ),
        });
    }
    (!messages.is_empty()).then(|| messages.join(" "))
}

/// STOP is confirmed by a STATUS newer than the baseline taken before STOP
/// was written that reports acquisition stopped with every output LOW.
fn stop_is_confirmed(snapshot: &AcquisitionSnapshot, status_baseline: u64) -> bool {
    snapshot.status_count > status_baseline
        && snapshot.last_status_acquiring == Some(false)
        && snapshot.digital_output_mask == Some(0)
}

/// A conservative byte estimate for one exported CSV row of this lab and
/// calibration (see `export_bmeg_csv` for the column layout).
fn estimated_csv_row_bytes(profile: &ProfileSnapshot, calibration: &RecordingCalibration) -> u64 {
    let acquisition = &profile.profile.acquisition;
    match acquisition.acquisition_mode {
        crate::profiles::AcquisitionMode::Pulseox4State => {
            let fields = acquisition.record_field_names().len() as u64;
            CSV_ROW_PREFIX_BYTES + fields * CSV_COUNTS_FIELD_BYTES
        }
        crate::profiles::AcquisitionMode::Simultaneous => acquisition
            .resolved_channels()
            .iter()
            .map(|channel| {
                let derived = match calibration.for_channel(&channel.id) {
                    // kPa and, depending on the channel, mmHg.
                    Some(preset) if preset.calibration_type == CalibrationType::FixedFormula => 2,
                    Some(_) => 1,
                    None => 0,
                };
                CSV_COUNTS_FIELD_BYTES + CSV_VOLTS_FIELD_BYTES + derived * CSV_DERIVED_FIELD_BYTES
            })
            .sum::<u64>()
            .saturating_add(CSV_ROW_PREFIX_BYTES),
    }
}

fn projected_csv_bytes(records: u64, row_bytes: u64) -> u64 {
    records
        .saturating_mul(row_bytes)
        .saturating_add(CSV_HEADER_RESERVE_BYTES)
}

/// Free space below which acquisition stops: the documented 250 MiB critical
/// reserve plus the projected size of the CSV still to be exported.
fn storage_stop_threshold(csv_reserve_bytes: u64) -> u64 {
    DISK_CRITICAL_BYTES.saturating_add(csv_reserve_bytes)
}

/// Upper bound for a BMEG header (its length field is a u16) plus its prefix.
const BMEG_HEADER_RESERVE_BYTES: u64 = 64 * 1024;

/// Projected disk use of one recording, shown before Start so a student can
/// choose a folder with room for the raw BMEG and the CSV exported from it.
/// The CSV uses the same conservative per-row estimate as the storage guard.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RecordingStorageEstimate {
    /// Records per second; one pulse-ox record is a full RED/DARK 1/IR/DARK 2
    /// cycle, so its rate follows the state dwell time.
    pub records_per_second: u64,
    pub bmeg_bytes_per_second: u64,
    pub csv_bytes_per_second: u64,
    /// BMEG plus CSV for a timed recording; `None` when it runs until stopped.
    pub projected_bytes: Option<u64>,
    /// Free space a timed recording needs to finish without the storage guard
    /// stopping it: the projected files plus the critical reserve.
    pub required_free_bytes: Option<u64>,
    /// Acquisition stops when free space falls below this reserve plus the
    /// CSV still to be exported.
    pub critical_free_bytes: u64,
}

/// Estimates the disk use of a recording of `profile` with `calibration` for
/// `duration`. It reads nothing from disk and opens no port.
pub fn estimate_recording_storage(
    profile: &ProfileSnapshot,
    calibration: &RecordingCalibration,
    duration: &RecordingDuration,
) -> RecordingStorageEstimate {
    let acquisition = &profile.profile.acquisition;
    let records_per_second = match acquisition.acquisition_mode {
        crate::profiles::AcquisitionMode::Simultaneous => u64::from(acquisition.sample_rate_hz),
        crate::profiles::AcquisitionMode::Pulseox4State => acquisition
            .state_dwell_us
            .filter(|dwell| *dwell > 0)
            .map(|dwell| 1_000_000u64.div_ceil(4 * u64::from(dwell)))
            .unwrap_or(0),
    };
    let field_count = acquisition.record_field_names().len();
    let bmeg_record_bytes = synchronized_record_bytes(field_count) as u64;
    let csv_row_bytes = estimated_csv_row_bytes(profile, calibration);
    let projected_bytes = duration.requested_seconds().map(|seconds| {
        let records = records_per_second.saturating_mul(seconds);
        records
            .saturating_mul(bmeg_record_bytes)
            .saturating_add(BMEG_HEADER_RESERVE_BYTES)
            .saturating_add(projected_csv_bytes(records, csv_row_bytes))
    });
    RecordingStorageEstimate {
        records_per_second,
        bmeg_bytes_per_second: records_per_second.saturating_mul(bmeg_record_bytes),
        csv_bytes_per_second: records_per_second.saturating_mul(csv_row_bytes),
        projected_bytes,
        required_free_bytes: projected_bytes.map(|bytes| bytes.saturating_add(DISK_CRITICAL_BYTES)),
        critical_free_bytes: DISK_CRITICAL_BYTES,
    }
}

const HEADER_TOO_LARGE_MESSAGE: &str = "the lab description, notices, and recording note are too long to store in a recording header (64 KiB); shorten the lab description or the note";

fn header_budget_error(error: RecordingError) -> SessionError {
    match error {
        RecordingError::HeaderTooLarge => SessionError::InvalidRequest(HEADER_TOO_LARGE_MESSAGE),
        other => SessionError::Recording(other),
    }
}

/// Inputs to the synchronous Start-time header precheck.
struct HeaderBudgetRequest<'a> {
    simulator: bool,
    source: &'a str,
    duration: &'a RecordingDuration,
    profile: &'a ProfileSnapshot,
    calibration: &'a RecordingCalibration,
    recording_path_context: Option<&'a RecordingPathContext>,
    recording_context: &'a RecordingContext,
}

/// Rejects, before any port is opened or the session begins, a lab and note
/// whose recording header could not fit the 64 KiB BMEG limit. Provenance
/// strings resolved later by the worker are covered by a fixed reserve; the
/// worker re-checks the exact header before configuring the board.
fn check_header_budget(request: HeaderBudgetRequest<'_>) -> Result<(), SessionError> {
    let field_count = request
        .profile
        .profile
        .acquisition
        .record_field_names()
        .len();
    let placeholder = PathBuf::from(format!(
        "20000101_000000_{}_Run99.bmeg",
        crate::profiles::safe_filename_component(&request.profile.profile.export.signal_name)
    ));
    let mut metadata = build_initial_metadata(
        InitialMetadataRequest {
            simulator: request.simulator,
            source: request.source,
            bmeg: &placeholder,
            duration: request.duration,
            profile: request.profile,
            calibration: request.calibration.clone(),
            recording_path_context: request.recording_path_context,
            recording_context: request.recording_context,
        },
        ToolingProvenance::placeholder(request.simulator),
    );
    metadata.record_layout = Some(RecordLayout::Synchronized { field_count });
    let header_bytes =
        check_synchronized_header(&metadata, field_count).map_err(header_budget_error)?;
    if header_bytes.saturating_add(HEADER_PROVENANCE_RESERVE_BYTES) > usize::from(u16::MAX) {
        return Err(SessionError::InvalidRequest(HEADER_TOO_LARGE_MESSAGE));
    }
    Ok(())
}

/// Arduino tooling facts recorded in the header of a hardware recording.
#[derive(Clone, Debug)]
struct ToolingProvenance {
    arduino_cli_version: String,
    uno_r4_core_version: String,
    board_serial: Option<String>,
}

impl ToolingProvenance {
    fn not_applicable() -> Self {
        Self {
            arduino_cli_version: "not applicable".into(),
            uno_r4_core_version: "not applicable".into(),
            board_serial: None,
        }
    }

    /// Short stand-ins for the precheck; `HEADER_PROVENANCE_RESERVE_BYTES`
    /// covers the real values.
    fn placeholder(simulator: bool) -> Self {
        if simulator {
            Self::not_applicable()
        } else {
            Self {
                arduino_cli_version: "pending".into(),
                uno_r4_core_version: "pending".into(),
                board_serial: None,
            }
        }
    }
}

/// Value recorded when a tool version could not be determined.
const PROVENANCE_UNAVAILABLE: &str = "unavailable";

/// Tooling provenance for a recording header, without a board scan.
///
/// Provenance is best effort: if no version is known, "unavailable" is
/// recorded and the recording proceeds. Firmware identity is still proven
/// strictly by the handshake.
fn tooling_provenance(simulator: bool, board_serial: Option<String>) -> ToolingProvenance {
    if simulator {
        return ToolingProvenance::not_applicable();
    }
    let versions = provenance_tool_versions(
        crate::arduino_cli::known_tool_versions(),
        crate::arduino_runtime::active_runtime(),
        crate::arduino_cli::cached_tool_versions,
    );
    match versions {
        Ok(versions) => ToolingProvenance {
            arduino_cli_version: versions.arduino_cli_version,
            uno_r4_core_version: versions.uno_r4_core_version,
            board_serial,
        },
        Err(error) => {
            crate::app_log::record("WARN", &format!("PROVENANCE_UNAVAILABLE detail={error}"));
            ToolingProvenance {
                arduino_cli_version: PROVENANCE_UNAVAILABLE.into(),
                uno_r4_core_version: PROVENANCE_UNAVAILABLE.into(),
                board_serial,
            }
        }
    }
}

/// The tool versions for a recording header. Start holds the session lease,
/// so it must not wait on Arduino CLI probes (up to 30 s each) every time a
/// previous probe failed. In order: the versions probed once after tool
/// preparation; otherwise the bundled runtime's pinned manifest versions
/// (production runs only those tools); a probe only in a development build
/// that has no bundled runtime.
fn provenance_tool_versions(
    known: Option<crate::arduino_cli::ToolVersions>,
    runtime: Option<&crate::arduino_runtime::RuntimePaths>,
    probe: impl FnOnce() -> Result<crate::arduino_cli::ToolVersions, crate::arduino_cli::CliError>,
) -> Result<crate::arduino_cli::ToolVersions, crate::arduino_cli::CliError> {
    if let Some(versions) = known {
        return Ok(versions);
    }
    if let Some(runtime) = runtime {
        return Ok(crate::arduino_cli::ToolVersions {
            arduino_cli_version: runtime.cli_version.clone(),
            uno_r4_core_version: runtime.core_version.clone(),
        });
    }
    probe()
}

/// Every session worker must own the session lease.
fn require_session_lease(lease: &SerialLease) -> Result<(), SessionError> {
    if lease.owner() == SerialOwner::Session {
        Ok(())
    } else {
        Err(SessionError::State(
            "a recording requires the session lease",
        ))
    }
}

/// Which operation a worker error that escaped capture (one raised before
/// START, so no recording ended) belongs to.
fn worker_fault_origin(error: &SessionError) -> FaultOrigin {
    match error {
        SessionError::Storage(_) | SessionError::Recording(_) | SessionError::Save(_) => {
            FaultOrigin::Storage
        }
        SessionError::InvalidRequest(_) => FaultOrigin::Request,
        _ => FaultOrigin::Connection,
    }
}

/// Windows reports "access denied" (another program owns the port) and "file
/// not found" (the device is gone) with the same serialport error kind, so a
/// failed open is classified by a read-only enumeration instead.
fn port_open_failure_category(port: &str) -> ConnectionFailureCategory {
    if diagnose_port_enumeration(port, None).selected_port_present == Some(false) {
        ConnectionFailureCategory::DeviceDisconnected
    } else {
        ConnectionFailureCategory::PortBusy
    }
}

/// A serial I/O error proves removal only when the port is no longer listed.
fn category_after_io_failure(port_state: &PortEnumerationDiagnostic) -> ConnectionFailureCategory {
    if port_state.selected_port_present == Some(false) {
        ConnectionFailureCategory::DeviceDisconnected
    } else {
        ConnectionFailureCategory::SerialTransportError
    }
}

/// Builds the immutable recording header. `utc_start`/`local_start` are
/// provisional here and re-stamped immediately before START by capture;
/// `session_opened_utc` keeps this preparation time.
fn build_initial_metadata(
    InitialMetadataRequest {
        simulator,
        source,
        bmeg,
        duration,
        profile,
        calibration,
        recording_path_context,
        recording_context,
    }: InitialMetadataRequest<'_>,
    provenance: ToolingProvenance,
) -> RecordingMetadata {
    let ToolingProvenance {
        arduino_cli_version,
        uno_r4_core_version,
        board_serial,
    } = provenance;
    let mut digital_output_mapping = std::collections::BTreeMap::new();
    for output in profile.profile.acquisition.resolved_digital_outputs() {
        let key = match output.effective_pulse_ox_role() {
            Some(role) => role.mapping_key().to_string(),
            None if output.label.eq_ignore_ascii_case("green led") => "green".into(),
            None => output.label,
        };
        digital_output_mapping.insert(key, output.pin);
    }
    let opened = Utc::now();
    RecordingMetadata {
        utc_start: opened,
        local_start: opened.with_timezone(&Local),
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
        protocol_version: format!(
            "{}.{}",
            crate::protocol::PROTOCOL_MAJOR,
            crate::protocol::PROTOCOL_MINOR
        ),
        analog_pin: profile.profile.acquisition.analog_pins().join(","),
        active_analog_pins: profile.profile.acquisition.analog_pins(),
        digital_output_mapping,
        adc_bits: profile.profile.acquisition.adc_resolution_bits,
        requested_sample_rate_hz: profile.profile.acquisition.sample_rate_hz,
        measured_sample_rate_hz: 0.0,
        timing_diagnostics: None,
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
        notes: if recording_context.recording_note.is_empty() {
            default_recording_note(simulator, profile)
        } else {
            recording_context.recording_note.clone()
        },
        user_recording_note: (!recording_context.recording_note.is_empty())
            .then(|| recording_context.recording_note.clone()),
        recording_safety: recording_safety_context(profile),
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
        record_layout: None,
        session_opened_utc: Some(opened),
        acquisition_started_utc: None,
        csv_error: None,
        finalize_error: None,
    }
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

fn validate_recording_context(context: &RecordingContext) -> Result<(), SessionError> {
    if context.recording_note.chars().count() > 4_096 {
        return Err(SessionError::State(
            "recording note must be 4096 characters or fewer",
        ));
    }
    Ok(())
}

fn default_recording_note(simulator: bool, snapshot: &ProfileSnapshot) -> String {
    let profile = &snapshot.profile;
    if simulator {
        return format!(
            "Deterministic simulator recording for {}. No physical Arduino input was acquired.",
            profile.display_name
        );
    }
    match profile.acquisition.acquisition_mode {
        crate::profiles::AcquisitionMode::Pulseox4State => {
            let inputs = profile.acquisition.analog_inputs.as_ref();
            let tx = inputs.map_or("unspecified", |value| value.tx.as_str());
            let rx = inputs.map_or("unspecified", |value| value.rx.as_str());
            format!(
                "Hardware recording for {}: {tx} TX and {rx} RX in RED, DARK 1, IR, DARK 2 raw acquisition order.",
                profile.display_name
            )
        }
        crate::profiles::AcquisitionMode::Simultaneous => {
            let channels = profile
                .acquisition
                .resolved_channels()
                .into_iter()
                .map(|channel| format!("{} = {}", channel.pin, channel.label))
                .collect::<Vec<_>>()
                .join("; ");
            format!(
                "Hardware recording for {}: {channels}; synchronized analog acquisition.",
                profile.display_name
            )
        }
    }
}

fn recording_safety_context(snapshot: &ProfileSnapshot) -> RecordingSafetyContext {
    RecordingSafetyContext {
        bench_only_profile: snapshot.profile.safety.bench_only,
        human_connection_authorized: snapshot.profile.safety.human_connection_authorized,
        bench_notice_acknowledged: snapshot.bench_notice_acknowledged,
        medical_validation_status: "not_medical_device".into(),
    }
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
            // The same RED/IR selection rule as profile validation: an
            // explicit `pulse_ox_role`, otherwise the "Red LED"/"IR LED" label.
            let outputs = settings.resolved_digital_outputs();
            let red = crate::profiles::pulse_ox_output(&outputs, crate::profiles::PulseOxRole::Red)
                .ok_or(SessionError::State(
                    "pulse-ox profile is missing a RED output",
                ))?;
            let ir = crate::profiles::pulse_ox_output(&outputs, crate::profiles::PulseOxRole::Ir)
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
/// run Arduino CLI, change DTR/RTS, or reset the board.
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

/// Student-facing next step for each category. Texts name the visible
/// buttons (Retry connection, Verify Firmware, Restore WVU Firmware, Refresh
/// Board); the recovery order is Retry connection, then Verify Firmware, then
/// Restore WVU Firmware.
fn recommended_action(category: &ConnectionFailureCategory) -> &'static str {
    match category {
        ConnectionFailureCategory::PortBusy => {
            "Another program may be using the board. Close Arduino IDE, Serial Monitor, or Arduino Cloud Agent, then select Retry connection."
        }
        ConnectionFailureCategory::PortOpenNoBytes => {
            "The board did not answer. Select Retry connection, then Verify Firmware. If it still does not answer, use Restore WVU Firmware."
        }
        ConnectionFailureCategory::NonProtocolBytes => {
            "The board is not running the WVU firmware. Check that the right board is selected, then use Verify Firmware or Restore WVU Firmware."
        }
        ConnectionFailureCategory::ProtocolCrcFailure => {
            "The data from the board was damaged. Check the USB cable, select Retry connection, then Verify Firmware."
        }
        ConnectionFailureCategory::WrongProtocolVersion => {
            "The board has a different WVU firmware version. Use Restore WVU Firmware."
        }
        ConnectionFailureCategory::MissingFirmwareIdentity => {
            "The board did not identify its WVU firmware. Use Restore WVU Firmware."
        }
        ConnectionFailureCategory::IncompatibleFirmwareIdentity => {
            "The board has a different WVU firmware version. Use Restore WVU Firmware."
        }
        ConnectionFailureCategory::HandshakeIncomplete => {
            "The board did not finish connecting. Select Retry connection, then Verify Firmware. If it still fails, use Restore WVU Firmware."
        }
        ConnectionFailureCategory::SerialTransportError => {
            "The USB serial connection reported an error, but the board was not proven removed. Select Refresh Board, then Retry connection."
        }
        ConnectionFailureCategory::FirmwareProtocolError => {
            "The Arduino stopped the recording, often because the computer paused or slept. Select Refresh Board and start a new recording; use Verify Firmware if it happens again."
        }
        ConnectionFailureCategory::NoDataTimeout => {
            "The Arduino stopped sending samples. Select Refresh Board, then Verify Firmware before starting a new recording."
        }
        ConnectionFailureCategory::DeviceDisconnected => {
            "The board is no longer connected. Reconnect the UNO R4 WiFi, then select Refresh Board."
        }
        ConnectionFailureCategory::RecordingStorageError => {
            "The recording folder could not be used or written. Check that its drive is connected and has free space, then start a new recording. Files already saved were kept."
        }
        ConnectionFailureCategory::LabNotSupportedByFirmware => {
            "The board's firmware cannot run this lab's settings. Use Restore WVU Firmware, or ask your instructor to check the lab."
        }
        ConnectionFailureCategory::ConfigurationRejected => {
            "The board did not accept the lab settings. Select Retry connection, then Verify Firmware before starting again."
        }
        ConnectionFailureCategory::StartNotConfirmed => {
            "The board did not confirm the start of recording. Select Retry connection, then Verify Firmware before starting again."
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
    /// Batches still emitted after STOP before the terminal STATUS, mirroring
    /// the firmware's final partial batch and queued batches. The default of
    /// zero (an immediate STATUS) keeps simulator recordings deterministic;
    /// tests raise it to exercise the post-STOP drain.
    post_stop_batches: u32,
    stop_batches_remaining: u32,
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
            post_stop_batches: 0,
            stop_batches_remaining: 0,
        };
        simulator.queue(
            MessageType::Hello,
            [
                REFERENCE_FIRMWARE_BUILD.to_le_bytes().as_slice(),
                REFERENCE_DEVICE_ID.to_le_bytes().as_slice(),
                &[1, 14, 6, 0],
            ]
            .concat(),
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

    /// Queues what a previous session that ended without STOP can leave in
    /// the board's transmit queue: a watchdog ERROR 7, a terminal STATUS
    /// still reporting D4 HIGH with overflows, and a batch of the old run.
    #[cfg(test)]
    fn queue_stale_previous_session_frames(&mut self) -> Result<(), SessionError> {
        self.queue(MessageType::ErrorMessage, vec![7])?;
        self.queue(MessageType::Status, vec![0, 0x01, 4, 0, 0, 0])?;
        let records = 10usize;
        let payload = SampleBatch {
            first_sample_sequence: 9_000,
            sample_period_us: 1_000,
            channel_count: self.channel_count,
            timestamps_us: (0..records as u64)
                .map(|index| 50_000_000 + index * 1_000)
                .collect(),
            samples: vec![1_000; records * usize::from(self.channel_count)],
            status_flags: crate::protocol::SAMPLE_STATUS_ACQUIRING,
            acquisition_queue_overflows: Some(2),
        }
        .to_payload()
        .map_err(|error| SessionError::Protocol(format!("{error:?}")))?;
        self.queue(MessageType::SampleBatch, payload)
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
            sample_period_us: self.sample_period_us,
            channel_count: self.channel_count,
            timestamps_us: (0..count)
                .map(|index| {
                    u64::from(self.sample_sequence + index) * u64::from(self.sample_period_us)
                })
                .collect(),
            samples,
            status_flags: crate::protocol::SAMPLE_STATUS_ACQUIRING,
            acquisition_queue_overflows: Some(0),
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
                .batch_interval
                .is_none_or(|_| Instant::now() >= self.next_batch_at)
        {
            let can_generate = self
                .sample_limit
                .is_none_or(|limit| u64::from(self.sample_sequence) < limit);
            if can_generate {
                self.queue_sample_batch()
                    .map_err(|error| std::io::Error::other(error.to_string()))?;
                if let Some(interval) = self.batch_interval {
                    self.next_batch_at += interval;
                }
            }
            if self.stop_batches_remaining > 0 {
                // STOP was received: emit the configured tail, then the
                // terminal STATUS, as the firmware does after its queue drains.
                self.stop_batches_remaining = if can_generate {
                    self.stop_batches_remaining - 1
                } else {
                    0
                };
                if self.stop_batches_remaining == 0 {
                    self.active = false;
                    self.queue(MessageType::Status, vec![0, self.output_mask, 0, 0, 0, 0])
                        .map_err(|error| std::io::Error::other(error.to_string()))?;
                }
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
                    self.stop_batches_remaining = 0;
                    self.next_batch_at = Instant::now();
                    self.queue(MessageType::Status, vec![1, self.output_mask, 0, 0, 0, 0])
                }
                MessageType::Stop => {
                    self.output_mask = 0;
                    if self.active && self.post_stop_batches > 0 {
                        // The terminal STATUS follows the tail (see `read`).
                        self.stop_batches_remaining = self.post_stop_batches;
                        Ok(())
                    } else {
                        self.active = false;
                        self.queue(MessageType::Status, vec![0, self.output_mask, 0, 0, 0, 0])
                    }
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
            .set_fault(FaultOrigin::Verification, "firmware did not respond".into())
            .unwrap_or_else(|error| panic!("{error}"));

        assert!(!session
            .is_recording()
            .unwrap_or_else(|error| panic!("{error}")));
    }

    #[test]
    fn successful_recovery_normalizes_an_idle_fault_before_the_next_recording() {
        let session = SessionController::default();
        session
            .set_fault(FaultOrigin::Verification, "firmware did not respond".into())
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

    #[test]
    fn a_recording_holds_the_session_lease_until_its_worker_finishes() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let session = SessionController::default();
        let ownership = session.serial_ownership();
        session
            .start_simulator(RecordingDuration::UntilStopped, dir.path().into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(ownership.current(), SerialOwner::Session);
        // Verify, Retry connection, and a restore cannot take the board.
        let busy = ownership
            .try_claim(SerialOwner::Verify)
            .err()
            .unwrap_or_else(|| panic!("a recording must refuse Verify"));
        assert_eq!(busy.holder, SerialOwner::Session);
        assert!(matches!(
            session.claim_serial(SerialOwner::FirmwareJob),
            Err(SessionError::Busy(_))
        ));
        assert_eq!(
            wait_for_state(&session, SessionState::Acquiring).state,
            SessionState::Acquiring
        );
        session
            .request_stop()
            .unwrap_or_else(|error| panic!("{error}"));
        session
            .wait_for_worker()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(ownership.current(), SerialOwner::Idle);
    }

    #[test]
    fn a_start_is_refused_while_another_operation_owns_the_board() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let session = SessionController::default();
        let _verify = session
            .serial_ownership()
            .try_claim(SerialOwner::Verify)
            .unwrap_or_else(|busy| panic!("{busy}"));
        let error = session
            .start_simulator(RecordingDuration::Timed { seconds: 10 }, dir.path().into())
            .err()
            .unwrap_or_else(|| panic!("Start must wait for Verify Firmware"));
        assert!(
            matches!(error, SessionError::Busy(busy) if busy.holder == SerialOwner::Verify),
            "{error}"
        );
        assert!(error.to_string().contains("being verified"), "{error}");
        let status = session.status().unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(status.state, SessionState::Disconnected);
        assert!(fs::read_dir(dir.path())
            .unwrap_or_else(|error| panic!("{error}"))
            .next()
            .is_none());
    }

    #[test]
    fn disconnect_requests_the_stop_and_returns_before_the_recording_is_saved() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let session = SessionController::default();
        session
            .start_simulator(RecordingDuration::UntilStopped, dir.path().into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            wait_for_state(&session, SessionState::Acquiring).state,
            SessionState::Acquiring
        );
        let returned = session
            .request_disconnect()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(returned.state, SessionState::Stopping);
        // The worker finishes saving by itself; polling observes the result.
        let finished = wait_for_state(&session, SessionState::Disconnected);
        assert_eq!(finished.state, SessionState::Disconnected);
        assert_eq!(finished.stop_reason, Some(StopReason::User));
        assert!(finished
            .last_summary
            .as_ref()
            .is_some_and(|summary| summary.completion_status == "complete"));
        session
            .wait_for_worker()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(session.serial_ownership().current(), SerialOwner::Idle);
    }

    #[test]
    fn disconnect_clears_an_idle_fault_but_never_interrupts_a_board_operation() {
        let session = SessionController::default();
        session
            .set_fault(FaultOrigin::Verification, "firmware did not respond".into())
            .unwrap_or_else(|error| panic!("{error}"));
        let job = session
            .serial_ownership()
            .try_claim(SerialOwner::FirmwareJob)
            .unwrap_or_else(|busy| panic!("{busy}"));
        assert!(matches!(
            session.request_disconnect(),
            Err(SessionError::Busy(_))
        ));
        assert_eq!(
            session
                .status()
                .unwrap_or_else(|error| panic!("{error}"))
                .state,
            SessionState::Faulted
        );
        drop(job);
        let cleared = session
            .request_disconnect()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(cleared.state, SessionState::Disconnected);
        assert_eq!(session.serial_ownership().current(), SerialOwner::Idle);
    }

    #[test]
    fn disconnect_during_a_start_that_has_not_begun_is_refused_not_lost() {
        let session = SessionController::default();
        session
            .set_fault(FaultOrigin::Connection, "handshake failed".into())
            .unwrap_or_else(|error| panic!("{error}"));
        // A Start command holds the session lease while it checks the folder,
        // lab, and board, before its worker exists.
        let start = session
            .claim_serial(SerialOwner::Session)
            .unwrap_or_else(|error| panic!("{error}"));
        let refused = session
            .request_disconnect()
            .err()
            .unwrap_or_else(|| panic!("a Disconnect that would be lost must be refused"));
        assert!(
            matches!(refused, SessionError::Busy(busy) if busy.holder == SerialOwner::Session),
            "{refused}"
        );
        assert!(!session.cancel.load(Ordering::Acquire));
        assert!(session
            .stop_reason
            .lock()
            .unwrap_or_else(|error| panic!("{error}"))
            .is_none());
        assert_eq!(
            session
                .status()
                .unwrap_or_else(|error| panic!("{error}"))
                .state,
            SessionState::Faulted
        );

        // A worker that is returning releases the lease within the grace
        // period, and the Disconnect then clears the idle fault.
        let finishing = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            drop(start);
        });
        let cleared = session
            .request_disconnect()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(cleared.state, SessionState::Disconnected);
        finishing
            .join()
            .unwrap_or_else(|_| panic!("finishing thread panicked"));
        assert_eq!(session.serial_ownership().current(), SerialOwner::Idle);
    }

    #[test]
    fn a_port_that_cannot_be_opened_still_reports_a_category_and_origin() {
        let session = SessionController::default();
        let result = session
            .retry_handshake("WVU_TEST_NO_SUCH_PORT", FaultOrigin::Connection)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!result.handshake_succeeded);
        assert!(!result.diagnostics.port_opened);
        // Not listed by the operating system, so it is reported as no longer
        // connected; a listed port that cannot be opened is PortBusy.
        assert_eq!(
            result.diagnostics.failure_category,
            Some(ConnectionFailureCategory::DeviceDisconnected)
        );
        assert!(!result.diagnostics.recommended_action.contains("Reset"));
        let status = session.status().unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(status.state, SessionState::Faulted);
        assert_eq!(status.fault_origin, Some(FaultOrigin::Connection));
        assert!(status.last_error.is_some());
        assert!(!status.recording_active);
        assert!(
            !session
                .runtime
                .lock()
                .unwrap_or_else(|error| panic!("{error}"))
                .handshake_check
        );
    }

    #[test]
    fn a_handshake_check_is_never_reported_as_a_recording() {
        let session = SessionController::default();
        for state in [SessionState::Connecting, SessionState::Connected] {
            {
                let mut runtime = session
                    .runtime
                    .lock()
                    .unwrap_or_else(|error| panic!("{error}"));
                runtime.state = state;
                runtime.handshake_check = true;
            }
            let status = session.status().unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(status.state, state);
            assert!(!status.recording_active, "{state:?} during a board check");
            session
                .runtime
                .lock()
                .unwrap_or_else(|error| panic!("{error}"))
                .handshake_check = false;
            let status = session.status().unwrap_or_else(|error| panic!("{error}"));
            assert!(status.recording_active, "{state:?} during a recording");
        }
        session
            .runtime
            .lock()
            .unwrap_or_else(|error| panic!("{error}"))
            .state = SessionState::Disconnected;
        assert!(
            !session
                .status()
                .unwrap_or_else(|error| panic!("{error}"))
                .recording_active
        );
    }

    #[test]
    fn a_running_simulator_recording_is_reported_as_recording() {
        let directory = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let session = SessionController::default();
        let started = session
            .start_simulator(RecordingDuration::UntilStopped, directory.path().into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(started.recording_active);
        let stopped = session
            .disconnect()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!stopped.recording_active);
    }

    #[test]
    fn only_an_unlisted_port_counts_as_a_removed_board() {
        let listed = PortEnumerationDiagnostic {
            selected_port_present: Some(true),
            same_vid_pid_present: None,
            same_serial_present: None,
            uno_r4_present: Some(true),
            observed_vid: None,
            observed_pid: None,
            observed_serial_number: None,
            enumeration_error: None,
        };
        assert_eq!(
            category_after_io_failure(&listed),
            ConnectionFailureCategory::SerialTransportError
        );
        let unknown = PortEnumerationDiagnostic {
            selected_port_present: None,
            enumeration_error: Some("enumeration failed".into()),
            ..listed.clone()
        };
        assert_eq!(
            category_after_io_failure(&unknown),
            ConnectionFailureCategory::SerialTransportError
        );
        let removed = PortEnumerationDiagnostic {
            selected_port_present: Some(false),
            ..listed
        };
        assert_eq!(
            category_after_io_failure(&removed),
            ConnectionFailureCategory::DeviceDisconnected
        );
    }

    #[test]
    fn recommended_actions_name_only_buttons_that_exist() {
        use ConnectionFailureCategory as Category;
        for category in [
            Category::PortBusy,
            Category::PortOpenNoBytes,
            Category::NonProtocolBytes,
            Category::ProtocolCrcFailure,
            Category::WrongProtocolVersion,
            Category::MissingFirmwareIdentity,
            Category::IncompatibleFirmwareIdentity,
            Category::HandshakeIncomplete,
            Category::SerialTransportError,
            Category::FirmwareProtocolError,
            Category::NoDataTimeout,
            Category::DeviceDisconnected,
            Category::RecordingStorageError,
            Category::LabNotSupportedByFirmware,
            Category::ConfigurationRejected,
            Category::StartNotConfirmed,
        ] {
            let action = recommended_action(&category);
            assert!(!action.contains("Reset board"), "{category:?}: {action}");
            assert!(!action.contains("v0.1"), "{category:?}: {action}");
            assert!(
                !action.contains("refresh devices"),
                "{category:?}: {action}"
            );
        }
        assert!(
            !recommended_action(&Category::RecordingStorageError).contains("Verify Firmware"),
            "storage problems must not send students to firmware recovery"
        );
    }

    #[test]
    fn an_unusable_recording_folder_is_a_storage_failure_not_a_serial_one() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let file = dir.path().join("not-a-folder");
        fs::write(&file, b"x").unwrap_or_else(|error| panic!("{error}"));
        let session = SessionController::default();
        let error = session
            .allocate_paths(
                &file.join("Trial01"),
                &default_general_profile().unwrap_or_else(|error| panic!("{error}")),
            )
            .err()
            .unwrap_or_else(|| panic!("a file cannot hold a recording folder"));
        assert!(matches!(error, SessionError::Storage(_)), "{error}");
        assert_eq!(worker_fault_origin(&error), FaultOrigin::Storage);
    }

    #[test]
    fn a_start_the_board_does_not_confirm_is_a_connection_fault_with_its_own_category() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let session = SessionController::default();
        let profile = default_general_profile().unwrap_or_else(|error| panic!("{error}"));
        // A hardware-style session, which carries connection diagnostics.
        session
            .begin_session(
                false,
                "Arduino UNO R4 WiFi",
                "COM12",
                RecordingDuration::UntilStopped,
                profile.clone(),
                RecordingCalibration::default(),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        let inner = SimulatorIo::new(&profile, RecordingDuration::UntilStopped)
            .unwrap_or_else(|error| panic!("{error}"));
        let mut transport =
            FailingStart::new(inner, session.cancel.clone(), StartFailure::FirmwareError);
        let result = session.capture_transport(
            &mut transport,
            simulator_request(&profile, RecordingDuration::UntilStopped, dir.path()),
        );
        session
            .finish_worker_error(&result)
            .unwrap_or_else(|error| panic!("{error}"));
        let status = session.status().unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(status.state, SessionState::Faulted);
        // No recording ran, so the UI must not say that one ended.
        assert_eq!(status.fault_origin, Some(FaultOrigin::Connection));
        assert_eq!(
            status
                .connection_diagnostics
                .and_then(|diagnostics| diagnostics.failure_category),
            Some(ConnectionFailureCategory::StartNotConfirmed)
        );
    }

    struct HandshakeMock {
        bytes: VecDeque<u8>,
        commands: FrameParser,
        answer_on_ping: Option<u32>,
        pings: u32,
        /// The next this-many writes fail with a Windows-style write timeout
        /// before any byte reaches the board.
        ping_write_timeouts: u32,
        hello_payload: Vec<u8>,
    }

    impl HandshakeMock {
        fn new(answer_on_ping: Option<u32>) -> Self {
            Self {
                bytes: VecDeque::new(),
                commands: FrameParser::default(),
                answer_on_ping,
                pings: 0,
                ping_write_timeouts: 0,
                hello_payload: [
                    REFERENCE_FIRMWARE_BUILD.to_le_bytes().as_slice(),
                    REFERENCE_DEVICE_ID.to_le_bytes().as_slice(),
                    &[1, 14, 6, 0],
                ]
                .concat(),
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
            if self.ping_write_timeouts > 0 {
                self.ping_write_timeouts -= 1;
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "mock write timeout",
                ));
            }
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
        io.hello_payload[0] = io.hello_payload[0].wrapping_sub(1);
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
    fn a_timed_out_ping_write_is_one_failed_attempt_not_the_end_of_the_handshake() {
        let (session, mut acquisition) = handshake_session();
        let mut io = HandshakeMock::new(Some(1));
        io.ping_write_timeouts = 1;
        session
            .wait_for_handshake_with_policy(
                &mut io,
                &mut acquisition,
                Duration::ZERO,
                Duration::from_millis(500),
                3,
                Duration::from_millis(5),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        let diagnostics = session
            .status()
            .unwrap_or_else(|e| panic!("{e}"))
            .connection_diagnostics
            .unwrap_or_else(|| panic!("missing diagnostics"));
        assert_eq!(diagnostics.ping_attempts, 2);
        assert!(diagnostics.pong_received);
        assert_eq!(diagnostics.failure_category, None);
    }

    #[test]
    fn ping_writes_that_always_time_out_are_a_serial_transport_problem() {
        let (session, mut acquisition) = handshake_session();
        let mut io = HandshakeMock::new(Some(1));
        io.ping_write_timeouts = u32::MAX;
        assert!(session
            .wait_for_handshake_with_policy(
                &mut io,
                &mut acquisition,
                Duration::ZERO,
                Duration::from_millis(30),
                3,
                Duration::from_millis(1),
            )
            .is_err());
        let diagnostics = session
            .status()
            .unwrap_or_else(|e| panic!("{e}"))
            .connection_diagnostics
            .unwrap_or_else(|| panic!("missing diagnostics"));
        assert_eq!(diagnostics.ping_attempts, 3);
        assert_eq!(
            diagnostics.failure_category,
            Some(ConnectionFailureCategory::SerialTransportError)
        );
    }

    #[test]
    fn retry_is_prohibited_while_acquiring() {
        let (session, _acquisition) = handshake_session();
        session
            .set_state(SessionState::Acquiring)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            session.retry_handshake("COM12", FaultOrigin::Connection),
            Err(SessionError::State(_))
        ));
        assert_eq!(
            session.status().unwrap_or_else(|e| panic!("{e}")).state,
            SessionState::Acquiring
        );
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
        // A storage refusal must not be described as a board problem.
        assert_eq!(status.fault_origin, Some(FaultOrigin::Storage));
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
                recording_context: &RecordingContext::default(),
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
                recording_context: &RecordingContext::default(),
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
                    recording_context: RecordingContext::default(),
                    expected_port_identity: None,
                    provenance: None,
                },
            )
            .err()
            .unwrap_or_else(|| panic!("disconnecting transport unexpectedly completed"));
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.fault_origin, Some(FaultOrigin::Recording));
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
                    recording_context: RecordingContext::default(),
                    expected_port_identity: None,
                    provenance: None,
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
                recording_context: &RecordingContext::default(),
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
        assert_eq!(fit.fit.paired_samples, 1_201);
        assert!((fit.fit.slope - 120.0).abs() < 0.25);
        assert!((fit.fit.offset + 10.0).abs() < 1.0);
        assert!(fit.fit.r_squared > 0.999);
        assert_eq!(fit.adc_reference_v, 5.0);
        assert_eq!(fit.mpxv_sensor_supply_v, 5.0);
        assert_eq!(fit.recording_adc_reference_v, Some(5.0));
        assert_eq!(fit.recording_mpxv_sensor_supply_v, Some(5.0));

        // A different Vs on the page is used as requested (the math does not
        // silently switch), and the recording's own value is reported beside it.
        let changed =
            crate::calibration::fit_xgzp_from_recording(&crate::calibration::XgzpFitRequest {
                bmeg_path: bmeg.display().to_string(),
                start_seconds: 2.0,
                end_seconds: 8.0,
                adc_reference_v: 5.0,
                mpxv_sensor_supply_v: 4.8,
            })
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(changed.mpxv_sensor_supply_v, 4.8);
        assert_eq!(changed.recording_mpxv_sensor_supply_v, Some(5.0));
        assert!((changed.fit.slope - fit.fit.slope).abs() > 1.0);
        let serialized = serde_json::to_value(&changed).unwrap_or_else(|e| panic!("{e}"));
        assert!(serialized.get("slope").is_some());
        assert!(serialized.get("recording_mpxv_sensor_supply_v").is_some());
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
                recording_context: &RecordingContext::default(),
            })
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            metadata.project_folder.as_deref(),
            Some("C:\\Users\\Student\\Documents\\BMEG 420L")
        );
        assert_eq!(metadata.output_folder.as_deref(), Some("Lab6\\Trial1"));
    }

    #[test]
    fn pulseox_metadata_uses_profile_mapping_instead_of_the_legacy_a0_note() {
        let pulseox = built_in_profiles()
            .unwrap_or_else(|error| panic!("{error}"))
            .into_iter()
            .find(|profile| profile.category == "course_pulseox")
            .unwrap_or_else(|| panic!("missing pulse-ox profile"))
            .snapshot(false);
        let note = default_recording_note(false, &pulseox);
        assert!(note.contains("A0 TX and A1 RX"));
        assert!(note.contains("RED, DARK 1, IR, DARK 2"));
        assert!(!note.contains("floating/uncalibrated"));
        assert!(!note.contains("no human signal"));
    }

    #[test]
    fn recording_safety_preserves_profile_authorization_and_bench_acknowledgement() {
        let profiles = built_in_profiles().unwrap_or_else(|error| panic!("{error}"));
        let bench = profiles
            .iter()
            .find(|profile| profile.category == "development")
            .unwrap_or_else(|| panic!("missing bench profile"))
            .snapshot(true);
        let bench_context = recording_safety_context(&bench);
        assert!(bench_context.bench_only_profile);
        assert!(bench_context.bench_notice_acknowledged);
        assert!(!bench_context.human_connection_authorized);

        let mut approved = profiles
            .iter()
            .find(|profile| profile.category == "course_pulseox")
            .unwrap_or_else(|| panic!("missing pulse-ox profile"))
            .clone();
        approved.safety.human_connection_authorized = true;
        let approved = approved.snapshot(false);
        let safety = recording_safety_context(&approved);
        assert!(safety.human_connection_authorized);
        assert_eq!(safety.medical_validation_status, "not_medical_device");
    }

    #[test]
    fn user_recording_note_and_safety_are_identical_in_bmeg_header_and_sidecar_value() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let session = SessionController::default();
        let bmeg = dir.path().join("note-context.bmeg");
        let context = RecordingContext {
            recording_note: "  Exact operator note — preserve spacing.  ".into(),
        };
        let metadata = session
            .initial_metadata(InitialMetadataRequest {
                simulator: true,
                source: "SIM",
                bmeg: &bmeg,
                duration: &RecordingDuration::Timed { seconds: 10 },
                profile: &default_general_profile().unwrap_or_else(|error| panic!("{error}")),
                calibration: RecordingCalibration::default(),
                recording_path_context: None,
                recording_context: &context,
            })
            .unwrap_or_else(|error| panic!("{error}"));
        let mut writer = BmegWriter::create_synchronized(&bmeg, &metadata, 1)
            .unwrap_or_else(|error| panic!("{error}"));
        writer
            .write_record(&SynchronizedRecord {
                sequence: 0,
                timestamp_us: 1_000,
                status_flags: 1,
                counts: vec![123],
            })
            .unwrap_or_else(|error| panic!("{error}"));
        writer.finish().unwrap_or_else(|error| panic!("{error}"));
        let embedded = BmegReader::open(&bmeg)
            .unwrap_or_else(|error| panic!("{error}"))
            .metadata;
        let sidecar: RecordingMetadata = serde_json::from_slice(
            &serde_json::to_vec(&metadata).unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(embedded.notes, context.recording_note);
        assert_eq!(embedded.user_recording_note, Some(context.recording_note));
        assert_eq!(embedded.recording_safety, sidecar.recording_safety);
        assert_eq!(embedded.notes, sidecar.notes);
    }

    #[test]
    fn recording_filenames_use_the_lab_export_name_without_phase_labels() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let session = SessionController::default();
        for profile in built_in_profiles().unwrap_or_else(|error| panic!("{error}")) {
            let expected = crate::profiles::safe_filename_component(&profile.export.signal_name);
            let snapshot = profile.snapshot(true);
            let paths = session
                .allocate_paths(dir.path(), &snapshot)
                .unwrap_or_else(|error| panic!("{error}"));
            for path in [paths.bmeg, paths.csv, paths.metadata] {
                let filename = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_else(|| panic!("invalid test filename"));
                assert!(filename.contains(&expected));
                assert!(!filename.to_ascii_lowercase().contains("phase"));
            }
        }
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
                pulse_ox_role: None,
            },
            crate::profiles::DigitalOutput {
                pin: "D6".into(),
                label: "IR LED".into(),
                behavior: crate::profiles::DigitalOutputBehavior::AcquisitionSequenced,
                pulse_ox_role: None,
            },
        ];
        pulse.acquisition.state_dwell_us = Some(2_000);
        assert_eq!(
            configure_payload(&pulse.acquisition, Some(&capabilities))
                .unwrap_or_else(|error| panic!("{error}")),
            vec![1, 14, 208, 7, 0, 0, 2, 2, 3, 4, 6]
        );

        // An explicit role wins over the label rule, in configuration and
        // in the recorded output mapping alike.
        pulse.acquisition.digital_outputs = vec![
            crate::profiles::DigitalOutput {
                pin: "D6".into(),
                label: "660 nm emitter".into(),
                behavior: crate::profiles::DigitalOutputBehavior::AcquisitionSequenced,
                pulse_ox_role: Some(crate::profiles::PulseOxRole::Red),
            },
            crate::profiles::DigitalOutput {
                pin: "D5".into(),
                label: "RED LED".into(),
                behavior: crate::profiles::DigitalOutputBehavior::AcquisitionSequenced,
                pulse_ox_role: Some(crate::profiles::PulseOxRole::Ir),
            },
        ];
        assert_eq!(
            configure_payload(&pulse.acquisition, Some(&capabilities))
                .unwrap_or_else(|error| panic!("{error}")),
            vec![1, 14, 208, 7, 0, 0, 2, 2, 3, 6, 5]
        );
    }

    #[test]
    fn unchecking_the_legacy_green_output_really_turns_d4_off() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let store = crate::profiles::ProfileStore::with_root(dir.path().join("profiles"))
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .set_mode(crate::profiles::ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        let capabilities = FirmwareCapabilities {
            supported_adc_resolutions: vec![12, 14],
            max_analog_channels: 6,
            supported_modes: 0x03,
            supported_digital_output_mask: 0x07,
            supported_rates_hz: vec![100, 200, 250, 500, 1_000],
        };
        let bp_id = "wvu.bmeg420l.blood_pressure.ppg.course.capture.v1";
        let factory = store
            .get_locked(bp_id)
            .unwrap_or_else(|error| panic!("{error}"));
        // The factory lab drives D4 HIGH while recording (mask bit 0x01).
        assert_eq!(
            configure_payload(&factory.acquisition, Some(&capabilities))
                .unwrap_or_else(|error| panic!("{error}"))
                .last()
                .copied(),
            Some(0x01)
        );
        let mut draft = store
            .begin_lab_edit(bp_id)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(draft.acquisition.led_outputs.is_none());
        // The instructor unchecks D4.
        draft.acquisition.digital_outputs.clear();
        let saved = store
            .save_lab_draft(draft, Some("1.0.0".into()), "uncheck-d4-0001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(saved.acquisition.led_outputs.is_none());
        assert!(saved.acquisition.resolved_digital_outputs().is_empty());
        assert_eq!(
            configure_payload(&saved.acquisition, Some(&capabilities))
                .unwrap_or_else(|error| panic!("{error}"))
                .last()
                .copied(),
            Some(0x00)
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

    /// Requests a stop through the cancel flag (as the Stop button does) once
    /// the simulator has emitted `stop_after` samples, and records every
    /// command the host writes.
    struct StopAfterSamples {
        inner: SimulatorIo,
        cancel: Arc<AtomicBool>,
        stop_after: u32,
        commands: FrameParser,
        received: Vec<MessageType>,
    }

    impl StopAfterSamples {
        fn new(inner: SimulatorIo, cancel: Arc<AtomicBool>, stop_after: u32) -> Self {
            Self {
                inner,
                cancel,
                stop_after,
                commands: FrameParser::default(),
                received: Vec::new(),
            }
        }

        fn count(&self, message_type: MessageType) -> usize {
            self.received
                .iter()
                .filter(|received| **received == message_type)
                .count()
        }
    }

    impl Read for StopAfterSamples {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.inner.sample_sequence >= self.stop_after {
                self.cancel.store(true, Ordering::Release);
            }
            self.inner.read(buffer)
        }
    }

    impl Write for StopAfterSamples {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            for frame in self.commands.push(buffer) {
                self.received.push(frame.message_type);
            }
            self.inner.write(buffer)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }

    /// How a test transport answers START.
    #[derive(Clone, Copy)]
    enum StartFailure {
        /// The firmware answers START with an ERROR frame instead of STATUS.
        FirmwareError,
        /// START's STATUS never arrives and the user cancels during the wait.
        CancelDuringWait,
    }

    /// Forwards every command to the simulator except START, which it answers
    /// according to `failure`, and records every command the host writes.
    struct FailingStart {
        inner: SimulatorIo,
        cancel: Arc<AtomicBool>,
        failure: StartFailure,
        commands: FrameParser,
        received: Vec<MessageType>,
    }

    impl FailingStart {
        fn new(inner: SimulatorIo, cancel: Arc<AtomicBool>, failure: StartFailure) -> Self {
            Self {
                inner,
                cancel,
                failure,
                commands: FrameParser::default(),
                received: Vec::new(),
            }
        }
    }

    impl Read for FailingStart {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buffer)
        }
    }

    impl Write for FailingStart {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            // Each host command is written as one whole frame.
            let mut start = false;
            for frame in self.commands.push(buffer) {
                start |= frame.message_type == MessageType::Start;
                self.received.push(frame.message_type);
            }
            if !start {
                return self.inner.write(buffer);
            }
            match self.failure {
                StartFailure::FirmwareError => self
                    .inner
                    .queue(MessageType::ErrorMessage, vec![4])
                    .map_err(|error| std::io::Error::other(error.to_string()))?,
                StartFailure::CancelDuringWait => self.cancel.store(true, Ordering::Release),
            }
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }

    const PLENTY_OF_SPACE: u64 = 64 * 1024 * 1024 * 1024;

    /// Which metadata sidecar writes fail.
    #[derive(Clone, Copy, Default)]
    enum MetadataFailure {
        #[default]
        None,
        /// Both the sidecar written before the CSV and its rewrite fail.
        Every,
        /// The first sidecar is written; its rewrite with the CSV result fails.
        AfterFirst,
    }

    #[derive(Default)]
    struct InjectedStorage {
        fail_flush: bool,
        fail_finish: bool,
        fail_csv: bool,
        fail_bmeg_rename: bool,
        fail_metadata: MetadataFailure,
        metadata_writes: AtomicU64,
    }

    impl RecordingStorage for InjectedStorage {
        fn flush(&self, writer: &mut BmegWriter) -> Result<(), RecordingError> {
            if self.fail_flush {
                return Err(RecordingError::Io(std::io::Error::other(
                    "injected flush failure",
                )));
            }
            writer.flush()
        }

        fn finish(&self, writer: BmegWriter) -> Result<(), RecordingError> {
            if self.fail_finish {
                // Close the file (so the test can inspect it) but report the
                // final flush as failed, as a removed USB drive would.
                drop(writer);
                return Err(RecordingError::Io(std::io::Error::other(
                    "injected final flush failure",
                )));
            }
            writer.finish()
        }

        fn write_durably(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
            let call = self.metadata_writes.fetch_add(1, Ordering::Relaxed);
            let fail = match self.fail_metadata {
                MetadataFailure::None => false,
                MetadataFailure::Every => true,
                MetadataFailure::AfterFirst => call > 0,
            };
            if fail {
                return Err(std::io::Error::other("injected metadata write failure"));
            }
            fs::write(path, bytes)
        }

        fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            if self.fail_bmeg_rename && from.to_string_lossy().ends_with(".bmeg.part") {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected sharing violation",
                ));
            }
            fs::rename(from, to)
        }

        fn export_csv(&self, bmeg: &Path, csv: &Path) -> Result<u64, RecordingError> {
            if self.fail_csv {
                // Leave a partial file behind, as a full disk would.
                fs::write(csv, b"record_sequence,t_us\n0,")?;
                return Err(RecordingError::Io(std::io::Error::other(
                    "injected disk full during CSV export",
                )));
            }
            export_bmeg_csv(bmeg, csv)
        }
    }

    fn fast_timing() -> CaptureTiming {
        CaptureTiming {
            disk_check_interval: Duration::from_millis(20),
            flush_interval: Duration::from_millis(20),
            rename_attempts: 3,
            rename_initial_backoff: Duration::from_millis(1),
        }
    }

    fn plenty_of_space() -> Arc<dyn DiskSpaceProvider> {
        Arc::new(FixedDiskSpace(AtomicU64::new(PLENTY_OF_SPACE)))
    }

    fn controller_with(storage: InjectedStorage) -> SessionController {
        SessionController::with_environment(plenty_of_space(), Arc::new(storage), fast_timing())
    }

    fn simulator_request(
        profile: &ProfileSnapshot,
        duration: RecordingDuration,
        output_dir: &Path,
    ) -> CaptureRequest {
        CaptureRequest {
            simulator: true,
            source: "SIM".into(),
            profile: profile.clone(),
            duration,
            output_dir: output_dir.to_path_buf(),
            calibration: RecordingCalibration::default(),
            recording_path_context: None,
            recording_context: RecordingContext::default(),
            expected_port_identity: None,
            provenance: None,
        }
    }

    fn begin_simulated_session(
        session: &SessionController,
        profile: &ProfileSnapshot,
        duration: RecordingDuration,
    ) {
        session
            .begin_session(
                true,
                "Simulator",
                "SIM",
                duration,
                profile.clone(),
                RecordingCalibration::default(),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        session
            .set_state(SessionState::Connected)
            .unwrap_or_else(|e| panic!("{e}"));
    }

    /// Runs one simulated until-stopped capture that stops after `stop_after`
    /// samples (or ends earlier on its own).
    fn run_simulated_capture(
        session: &SessionController,
        output_dir: &Path,
        stop_after: u32,
        configure: impl FnOnce(&mut SimulatorIo),
    ) -> (Result<(), SessionError>, StopAfterSamples) {
        let profile = default_general_profile().unwrap_or_else(|e| panic!("{e}"));
        begin_simulated_session(session, &profile, RecordingDuration::UntilStopped);
        let mut inner = SimulatorIo::new(&profile, RecordingDuration::UntilStopped)
            .unwrap_or_else(|e| panic!("{e}"));
        configure(&mut inner);
        let mut transport = StopAfterSamples::new(inner, session.cancel.clone(), stop_after);
        let result = session.capture_transport(
            &mut transport,
            simulator_request(&profile, RecordingDuration::UntilStopped, output_dir),
        );
        (result, transport)
    }

    fn last_summary(session: &SessionController) -> SessionSummary {
        session
            .status()
            .unwrap_or_else(|e| panic!("{e}"))
            .last_summary
            .unwrap_or_else(|| panic!("missing summary"))
    }

    fn read_metadata(path: &str) -> RecordingMetadata {
        serde_json::from_slice(&fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}")))
            .unwrap_or_else(|e| panic!("{e}"))
    }

    fn bmeg_record_count(path: &str) -> u64 {
        let mut reader = BmegReader::open(Path::new(path)).unwrap_or_else(|e| panic!("{e}"));
        let mut count = 0u64;
        while reader
            .next_record()
            .unwrap_or_else(|e| panic!("{e}"))
            .is_some()
        {
            count += 1;
        }
        count
    }

    fn csv_line_count(path: &str) -> u64 {
        fs::read_to_string(path).unwrap_or_default().lines().count() as u64
    }

    fn wait_for_state(session: &SessionController, state: SessionState) -> SessionStatus {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let status = session.status().unwrap_or_else(|e| panic!("{e}"));
            if status.state == state || Instant::now() >= deadline {
                return status;
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn no_output_lab_keeps_batches_sent_after_stop() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        let (result, transport) = run_simulated_capture(&session, dir.path(), 100, |simulator| {
            simulator.post_stop_batches = 3;
        });
        result.unwrap_or_else(|e| panic!("{e}"));
        let summary = last_summary(&session);
        let emitted = u64::from(transport.inner.sample_sequence);
        assert!(
            emitted >= 130,
            "the simulator must have sent its post-STOP tail ({emitted})"
        );
        // A lab with no outputs reports mask 0 at START; that must not end the
        // post-STOP wait before the tail and the terminal STATUS are read.
        assert_eq!(summary.active_digital_output_mask, Some(0));
        assert_eq!(summary.final_digital_output_mask, Some(0));
        assert_eq!(summary.samples, emitted);
        assert_eq!(bmeg_record_count(&summary.bmeg_path), emitted);
        assert_eq!(csv_line_count(&summary.csv_path), emitted + 1);
        assert_eq!(summary.stop_reason, StopReason::User);
        assert_eq!(summary.completion_status, "complete");
        assert!(!transport.inner.active);
    }

    #[test]
    fn stale_frames_from_a_previous_session_neither_fault_nor_enter_the_recording() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        let (result, transport) = run_simulated_capture(&session, dir.path(), 50, |simulator| {
            simulator
                .queue_stale_previous_session_frames()
                .unwrap_or_else(|e| panic!("{e}"));
        });
        assert!(result.is_ok(), "{result:?}");
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Disconnected);
        assert!(status.last_error.is_none());
        let summary = status
            .last_summary
            .unwrap_or_else(|| panic!("missing summary"));
        assert_eq!(summary.stop_reason, StopReason::User);
        assert_eq!(summary.completion_status, "complete");
        assert!(summary.error.is_none());
        assert_eq!(summary.samples, u64::from(transport.inner.sample_sequence));
        assert_eq!(summary.integrity.missing_sample_sequences, 0);
        assert_eq!(summary.integrity.duplicate_sample_sequences, 0);
        assert_eq!(summary.integrity.out_of_order_sample_sequences, 0);
        assert_eq!(summary.integrity.firmware_overflows, 0);
        let mut reader =
            BmegReader::open(Path::new(&summary.bmeg_path)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            reader
                .next_record()
                .unwrap_or_else(|e| panic!("{e}"))
                .map(|record| record.sequence),
            Some(0)
        );
        // An idle-safe STOP quiesced the board before it was configured.
        let first_stop = transport
            .received
            .iter()
            .position(|message| *message == MessageType::Stop);
        let configure = transport
            .received
            .iter()
            .position(|message| *message == MessageType::Configure);
        assert!(first_stop.is_some() && first_stop < configure);
    }

    #[test]
    fn flush_failure_mid_recording_still_stops_and_finalizes_with_metadata() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = controller_with(InjectedStorage {
            fail_flush: true,
            ..InjectedStorage::default()
        });
        // The sample limit is only a backstop; the flush failure ends capture.
        let (result, transport) = run_simulated_capture(&session, dir.path(), 5_000, |_| {});
        let error = result
            .err()
            .unwrap_or_else(|| panic!("a flush failure must end the recording"));
        assert!(matches!(error, SessionError::Save(_)), "{error}");
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Faulted);
        assert!(status
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("could not be written")));
        let summary = status
            .last_summary
            .unwrap_or_else(|| panic!("missing summary"));
        assert_eq!(summary.stop_reason, StopReason::Fault);
        assert_eq!(summary.completion_status, "incomplete");
        assert!(summary
            .error
            .as_deref()
            .is_some_and(|detail| detail.contains("RECORDING_FLUSH")));
        // The quiescing STOP and the final STOP were both sent, and the
        // simulated board stopped acquiring.
        assert!(transport.count(MessageType::Stop) >= 2);
        assert!(!transport.inner.active);
        assert!(summary.bmeg_path.ends_with(".bmeg"));
        assert_eq!(bmeg_record_count(&summary.bmeg_path), summary.samples);
        let metadata = read_metadata(&summary.metadata_path);
        assert_eq!(metadata.stop_reason, Some(StopReason::Fault));
        assert_eq!(metadata.completion_status, "incomplete");
        assert_eq!(metadata.total_samples, summary.samples);
        assert!(metadata
            .finalize_error
            .as_deref()
            .is_some_and(|detail| detail.contains("RECORDING_FLUSH")));
    }

    #[test]
    fn failed_free_space_queries_are_logged_without_ending_the_recording() {
        struct FlakyDiskSpace(AtomicU64);

        impl DiskSpaceProvider for FlakyDiskSpace {
            fn available_space(&self, _path: &Path) -> std::io::Result<u64> {
                if self.0.fetch_add(1, Ordering::Relaxed) == 0 {
                    Ok(PLENTY_OF_SPACE)
                } else {
                    Err(std::io::Error::other("network share unavailable"))
                }
            }
        }

        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::with_environment(
            Arc::new(FlakyDiskSpace(AtomicU64::new(0))),
            Arc::new(InjectedStorage::default()),
            fast_timing(),
        );
        let (result, _transport) = run_simulated_capture(&session, dir.path(), 150, |_| {});
        result.unwrap_or_else(|e| panic!("{e}"));
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Disconnected);
        assert_eq!(status.available_disk_bytes, Some(PLENTY_OF_SPACE));
        let summary = status
            .last_summary
            .unwrap_or_else(|| panic!("missing summary"));
        assert_eq!(summary.stop_reason, StopReason::User);
        assert_eq!(summary.completion_status, "complete");
        assert_eq!(summary.final_free_disk_bytes, None);
    }

    #[test]
    fn storage_threshold_adds_the_projected_csv_to_the_critical_reserve() {
        let profile = default_general_profile().unwrap_or_else(|e| panic!("{e}"));
        let row = estimated_csv_row_bytes(&profile, &RecordingCalibration::default());
        assert_eq!(
            row,
            CSV_ROW_PREFIX_BYTES + CSV_COUNTS_FIELD_BYTES + CSV_VOLTS_FIELD_BYTES
        );
        assert_eq!(
            storage_stop_threshold(projected_csv_bytes(0, row)),
            DISK_CRITICAL_BYTES + CSV_HEADER_RESERVE_BYTES
        );
        assert_eq!(
            storage_stop_threshold(projected_csv_bytes(1_000, row)),
            DISK_CRITICAL_BYTES + CSV_HEADER_RESERVE_BYTES + 1_000 * row
        );
    }

    #[test]
    fn storage_guard_reserves_room_for_the_csv_export() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        // Enough for the 250 MiB reserve and the CSV header, but only about
        // 8 KiB of projected CSV rows.
        let free = DISK_CRITICAL_BYTES + CSV_HEADER_RESERVE_BYTES + 8 * 1024;
        let session = SessionController::with_environment(
            Arc::new(FixedDiskSpace(AtomicU64::new(free))),
            Arc::new(InjectedStorage::default()),
            fast_timing(),
        );
        let (result, _transport) = run_simulated_capture(&session, dir.path(), 5_000, |_| {});
        result.unwrap_or_else(|e| panic!("{e}"));
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert!(status.storage_warning.is_some());
        let summary = status
            .last_summary
            .unwrap_or_else(|| panic!("missing summary"));
        assert_eq!(summary.stop_reason, StopReason::StorageGuard);
        assert_eq!(summary.recording_status, "storage_guard");
        assert_eq!(summary.completion_status, "incomplete");
        assert!(summary.samples < 5_000);
    }

    #[test]
    fn csv_failure_after_a_user_stop_keeps_metadata_and_faults() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = controller_with(InjectedStorage {
            fail_csv: true,
            ..InjectedStorage::default()
        });
        let (result, _transport) = run_simulated_capture(&session, dir.path(), 50, |_| {});
        // The cancel flag is set (a user Stop); a save failure must still fault.
        session
            .finish_worker_error(&result)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(result, Err(SessionError::Save(_))), "{result:?}");
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Faulted);
        assert!(status
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("CSV export failed")));
        assert_eq!(status.fault_origin, Some(FaultOrigin::Storage));
        let summary = status
            .last_summary
            .unwrap_or_else(|| panic!("missing summary"));
        assert_eq!(summary.stop_reason, StopReason::User);
        assert_eq!(summary.completion_status, "complete");
        assert!(summary.csv_path.is_empty());
        assert!(summary
            .csv_error
            .as_deref()
            .is_some_and(|detail| detail.contains("injected disk full")));
        assert_eq!(bmeg_record_count(&summary.bmeg_path), summary.samples);
        let metadata = read_metadata(&summary.metadata_path);
        assert_eq!(metadata.csv_filename, None);
        assert!(metadata
            .csv_error
            .as_deref()
            .is_some_and(|detail| detail.contains("injected disk full")));
        assert_eq!(metadata.completion_status, "complete");
        let csv = Path::new(&summary.bmeg_path).with_extension("csv");
        assert!(!csv.exists());
        assert!(!sibling_with_suffix(&csv, ".part").exists());
    }

    #[test]
    fn rename_failure_keeps_the_part_file_and_still_writes_metadata_and_csv() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = controller_with(InjectedStorage {
            fail_bmeg_rename: true,
            ..InjectedStorage::default()
        });
        let (result, _transport) = run_simulated_capture(&session, dir.path(), 50, |_| {});
        assert!(matches!(result, Err(SessionError::Save(_))), "{result:?}");
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Faulted);
        let summary = status
            .last_summary
            .unwrap_or_else(|| panic!("missing summary"));
        assert!(
            summary.bmeg_path.ends_with(".bmeg.part"),
            "{}",
            summary.bmeg_path
        );
        assert_eq!(bmeg_record_count(&summary.bmeg_path), summary.samples);
        assert!(summary
            .finalize_error
            .as_deref()
            .is_some_and(|detail| detail.contains("rename")));
        assert_eq!(csv_line_count(&summary.csv_path), summary.samples + 1);
        let metadata = read_metadata(&summary.metadata_path);
        assert!(metadata.bmeg_filename.ends_with(".bmeg.part"));
        assert_eq!(metadata.completion_status, "complete");
        assert!(metadata
            .finalize_error
            .as_deref()
            .is_some_and(|detail| detail.contains("rename")));
    }

    #[test]
    fn only_cancellation_finishes_a_worker_quietly() {
        let profile = default_general_profile().unwrap_or_else(|e| panic!("{e}"));
        let timed = RecordingDuration::Timed { seconds: 10 };

        let cancelled = SessionController::default();
        begin_simulated_session(&cancelled, &profile, timed.clone());
        cancelled.request_stop().unwrap_or_else(|e| panic!("{e}"));
        cancelled
            .finish_worker_error(&Err(SessionError::Cancelled))
            .unwrap_or_else(|e| panic!("{e}"));
        let status = cancelled.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Disconnected);
        assert!(status.last_error.is_none());

        let save_failed = SessionController::default();
        begin_simulated_session(&save_failed, &profile, timed.clone());
        save_failed.request_stop().unwrap_or_else(|e| panic!("{e}"));
        save_failed
            .finish_worker_error(&Err(SessionError::Save("the CSV export failed".into())))
            .unwrap_or_else(|e| panic!("{e}"));
        let status = save_failed.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Faulted);
        assert!(status
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("CSV export failed")));

        let detailed = SessionController::default();
        begin_simulated_session(&detailed, &profile, timed);
        detailed.request_stop().unwrap_or_else(|e| panic!("{e}"));
        detailed
            .set_fault(FaultOrigin::Storage, "stage-specific save detail".into())
            .unwrap_or_else(|e| panic!("{e}"));
        detailed
            .finish_worker_error(&Err(SessionError::Io(std::io::Error::other("generic"))))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            detailed
                .status()
                .unwrap_or_else(|e| panic!("{e}"))
                .last_error
                .as_deref(),
            Some("stage-specific save detail")
        );
    }

    #[test]
    fn a_stop_during_connection_is_shown_as_stopping() {
        let profile = default_general_profile().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        begin_simulated_session(&session, &profile, RecordingDuration::Timed { seconds: 10 });
        let status = session.request_stop().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Stopping);
    }

    #[test]
    fn disconnect_keeps_a_fault_raised_while_the_worker_saved() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = controller_with(InjectedStorage {
            fail_csv: true,
            ..InjectedStorage::default()
        });
        session
            .start_simulator(RecordingDuration::UntilStopped, dir.path().to_path_buf())
            .unwrap_or_else(|e| panic!("{e}"));
        let acquiring = wait_for_state(&session, SessionState::Acquiring);
        assert_eq!(acquiring.state, SessionState::Acquiring);
        thread::sleep(Duration::from_millis(50));
        let status = session.disconnect().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Faulted);
        assert!(status
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("CSV export failed")));
        // An idle historical fault is still cleared by an explicit disconnect.
        let cleared = session.disconnect().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(cleared.state, SessionState::Disconnected);
    }

    #[test]
    fn elapsed_and_remaining_are_measured_from_start() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        session
            .start_simulator(
                RecordingDuration::Timed { seconds: 10 },
                dir.path().to_path_buf(),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        let status = wait_for_state(&session, SessionState::Acquiring);
        assert_eq!(status.state, SessionState::Acquiring);
        // The handshake grace (at least 1.25 s) and configuration are not
        // recording time.
        assert!(
            status.elapsed_seconds < 0.5,
            "elapsed {}",
            status.elapsed_seconds
        );
        assert!(
            status
                .remaining_seconds
                .is_some_and(|remaining| remaining > 9.5),
            "{:?}",
            status.remaining_seconds
        );
        session.request_stop().unwrap_or_else(|e| panic!("{e}"));
        session.wait_for_worker().unwrap_or_else(|e| panic!("{e}"));
        let summary = last_summary(&session);
        let metadata = read_metadata(&summary.metadata_path);
        let opened = metadata
            .session_opened_utc
            .unwrap_or_else(|| panic!("missing session_opened_utc"));
        let acquisition_started = metadata
            .acquisition_started_utc
            .unwrap_or_else(|| panic!("missing acquisition_started_utc"));
        assert!(opened <= metadata.utc_start);
        assert!(metadata.utc_start <= acquisition_started);
        // utc_start is stamped after the handshake, not when the session opened.
        assert!((metadata.utc_start - opened).num_milliseconds() >= 1_000);
    }

    #[test]
    fn start_provenance_never_probes_when_a_version_is_already_known() {
        use crate::arduino_cli::{CliError, ToolVersions};
        let probed = ToolVersions {
            arduino_cli_version: "probed".into(),
            uno_r4_core_version: "probed".into(),
        };
        let runtime = crate::arduino_runtime::RuntimePaths {
            executable: PathBuf::from("arduino-cli.exe"),
            config_file: PathBuf::from("arduino-cli.yaml"),
            root: PathBuf::from("rt"),
            cli_version: "1.5.2-rc.1".into(),
            core_version: "1.6.0".into(),
        };
        let no_probe = || -> Result<ToolVersions, CliError> {
            panic!("Start must not run Arduino CLI when a version is known")
        };
        let cached = provenance_tool_versions(Some(probed.clone()), Some(&runtime), no_probe)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(cached, probed);
        // A failed earlier probe falls back to the bundled runtime's versions.
        let pinned = provenance_tool_versions(None, Some(&runtime), no_probe)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(pinned.arduino_cli_version, "1.5.2-rc.1");
        assert_eq!(pinned.uno_r4_core_version, "1.6.0");
        // Only a development build without the bundled runtime probes.
        let probed_now = provenance_tool_versions(None, None, || Ok(probed.clone()))
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(probed_now, probed);
        assert!(provenance_tool_versions(None, None, || Err(CliError::NotFound)).is_err());
    }

    #[test]
    fn an_oversized_lab_header_is_rejected_before_the_session_begins() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        let mut profile = built_in_profiles()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .find(|profile| profile.category == "development")
            .unwrap_or_else(|| panic!("missing development profile"));
        profile.description = "x".repeat(70_000);
        let error = session
            .start_simulator_with_profile(
                profile.snapshot(true),
                RecordingDuration::Timed { seconds: 10 },
                dir.path().to_path_buf(),
            )
            .err()
            .unwrap_or_else(|| panic!("an oversized header must be rejected"));
        assert!(matches!(error, SessionError::InvalidRequest(_)), "{error}");
        // Had it reached the worker, it would not be described as a recording
        // that ended or as a connection problem.
        assert_eq!(worker_fault_origin(&error), FaultOrigin::Request);
        assert!(error.to_string().contains("too long"), "{error}");
        // The student sees the reason, not an internal state-machine prefix.
        assert!(!error.to_string().contains("illegal transition"), "{error}");
        assert_eq!(
            session.status().unwrap_or_else(|e| panic!("{e}")).state,
            SessionState::Disconnected
        );
        assert_eq!(
            fs::read_dir(dir.path())
                .unwrap_or_else(|e| panic!("{e}"))
                .count(),
            0
        );
    }

    /// Runs one simulated capture whose START fails as `failure` describes,
    /// then unwinds it the way the worker does.
    fn run_failing_start(
        session: &SessionController,
        output_dir: &Path,
        failure: StartFailure,
    ) -> (Result<(), SessionError>, FailingStart) {
        let profile = default_general_profile().unwrap_or_else(|e| panic!("{e}"));
        begin_simulated_session(session, &profile, RecordingDuration::UntilStopped);
        let inner = SimulatorIo::new(&profile, RecordingDuration::UntilStopped)
            .unwrap_or_else(|e| panic!("{e}"));
        let mut transport = FailingStart::new(inner, session.cancel.clone(), failure);
        let result = session.capture_transport(
            &mut transport,
            simulator_request(&profile, RecordingDuration::UntilStopped, output_dir),
        );
        session
            .finish_worker_error(&result)
            .unwrap_or_else(|e| panic!("{e}"));
        (result, transport)
    }

    fn stop_follows_start(received: &[MessageType]) -> bool {
        received
            .iter()
            .position(|message| *message == MessageType::Start)
            .is_some_and(|start| received[start + 1..].contains(&MessageType::Stop))
    }

    fn directory_entries(path: &Path) -> Vec<String> {
        fs::read_dir(path)
            .unwrap_or_else(|e| panic!("{e}"))
            .map(|entry| {
                entry
                    .unwrap_or_else(|e| panic!("{e}"))
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    #[test]
    fn a_rejected_start_sends_stop_and_removes_the_empty_recording() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        let (result, transport) =
            run_failing_start(&session, dir.path(), StartFailure::FirmwareError);
        let error = result
            .err()
            .unwrap_or_else(|| panic!("a rejected START must fail the session"));
        assert!(matches!(error, SessionError::Protocol(_)), "{error}");
        assert!(
            error
                .to_string()
                .contains("firmware ERROR while waiting for START status"),
            "{error}"
        );
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Faulted);
        assert!(status.last_error.is_some());
        // START may have reached the board, so a STOP follows it.
        assert!(
            stop_follows_start(&transport.received),
            "{:?}",
            transport.received
        );
        // The header-only recording is removed; nothing else was written.
        assert!(
            directory_entries(dir.path()).is_empty(),
            "{:?}",
            directory_entries(dir.path())
        );
    }

    #[test]
    fn cancelling_while_waiting_for_start_stops_quietly_and_leaves_no_files() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        let (result, transport) =
            run_failing_start(&session, dir.path(), StartFailure::CancelDuringWait);
        assert!(matches!(result, Err(SessionError::Cancelled)), "{result:?}");
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Disconnected);
        assert!(status.last_error.is_none());
        assert!(
            stop_follows_start(&transport.received),
            "{:?}",
            transport.received
        );
        assert!(
            directory_entries(dir.path()).is_empty(),
            "{:?}",
            directory_entries(dir.path())
        );
    }

    #[test]
    fn a_failed_final_flush_keeps_the_part_file_skips_the_csv_and_faults() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = controller_with(InjectedStorage {
            fail_finish: true,
            ..InjectedStorage::default()
        });
        let (result, transport) = run_simulated_capture(&session, dir.path(), 50, |_| {});
        session
            .finish_worker_error(&result)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(result, Err(SessionError::Save(_))), "{result:?}");
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Faulted);
        assert!(status
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("could not be written to disk")));
        assert!(transport.count(MessageType::Stop) >= 2);
        assert!(!transport.inner.active);
        let summary = status
            .last_summary
            .unwrap_or_else(|| panic!("missing summary"));
        assert_eq!(summary.stop_reason, StopReason::User);
        assert_eq!(summary.completion_status, "incomplete");
        // A file whose final flush failed keeps its `.part` name and is not
        // exported, so it is never mistaken for a finished recording.
        assert!(
            summary.bmeg_path.ends_with(".bmeg.part"),
            "{}",
            summary.bmeg_path
        );
        assert!(Path::new(&summary.bmeg_path).exists());
        assert!(!Path::new(summary.bmeg_path.trim_end_matches(".part")).exists());
        assert!(summary.csv_path.is_empty());
        assert!(summary
            .csv_error
            .as_deref()
            .is_some_and(|detail| detail.contains("skipped")));
        assert!(summary
            .finalize_error
            .as_deref()
            .is_some_and(|detail| detail.contains("flush")));
        let metadata = read_metadata(&summary.metadata_path);
        assert!(metadata.bmeg_filename.ends_with(".bmeg.part"));
        assert_eq!(metadata.completion_status, "incomplete");
        assert_eq!(metadata.csv_filename, None);
        assert!(metadata
            .csv_error
            .as_deref()
            .is_some_and(|detail| detail.contains("skipped")));
        assert!(!directory_entries(dir.path())
            .iter()
            .any(|name| name.ends_with(".csv") || name.ends_with(".csv.part")));
    }

    #[test]
    fn failed_metadata_writes_are_reported_and_leave_no_sidecar() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = controller_with(InjectedStorage {
            fail_metadata: MetadataFailure::Every,
            ..InjectedStorage::default()
        });
        let (result, _transport) = run_simulated_capture(&session, dir.path(), 50, |_| {});
        session
            .finish_worker_error(&result)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(result, Err(SessionError::Save(_))), "{result:?}");
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Faulted);
        assert!(status
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("metadata file")));
        let summary = status
            .last_summary
            .unwrap_or_else(|| panic!("missing summary"));
        assert!(summary.metadata_path.is_empty());
        assert!(summary
            .finalize_error
            .as_deref()
            .is_some_and(|detail| detail.contains("metadata")));
        // The raw recording and the CSV were still saved.
        assert!(
            summary.bmeg_path.ends_with(".bmeg"),
            "{}",
            summary.bmeg_path
        );
        assert_eq!(bmeg_record_count(&summary.bmeg_path), summary.samples);
        assert_eq!(csv_line_count(&summary.csv_path), summary.samples + 1);
        assert!(!directory_entries(dir.path())
            .iter()
            .any(|name| name.contains(".metadata.json")));
    }

    #[test]
    fn a_failed_metadata_rewrite_keeps_the_first_sidecar_and_is_summarized() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = controller_with(InjectedStorage {
            fail_metadata: MetadataFailure::AfterFirst,
            ..InjectedStorage::default()
        });
        let (result, _transport) = run_simulated_capture(&session, dir.path(), 50, |_| {});
        session
            .finish_worker_error(&result)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(result, Err(SessionError::Save(_))), "{result:?}");
        let status = session.status().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.state, SessionState::Faulted);
        let summary = status
            .last_summary
            .unwrap_or_else(|| panic!("missing summary"));
        // The sidecar written before the CSV export survives.
        assert!(!summary.metadata_path.is_empty());
        let metadata = read_metadata(&summary.metadata_path);
        assert_eq!(metadata.total_samples, summary.samples);
        assert_eq!(metadata.csv_filename, None);
        // The summary reports the failure the sidecar itself cannot record.
        assert!(
            summary
                .finalize_error
                .as_deref()
                .is_some_and(|detail| detail.contains("CSV result could not be added")),
            "{:?}",
            summary.finalize_error
        );
        assert!(status
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("metadata file")));
        assert_eq!(csv_line_count(&summary.csv_path), summary.samples + 1);
    }

    #[test]
    fn storage_estimate_uses_the_bmeg_layout_and_the_guard_csv_estimate() {
        let calibration = RecordingCalibration::default();
        let profile = default_general_profile().unwrap_or_else(|e| panic!("{e}"));
        let rate = u64::from(profile.profile.acquisition.sample_rate_hz);
        let fields = profile.profile.acquisition.record_field_names().len();
        let record_bytes = synchronized_record_bytes(fields) as u64;
        let row_bytes = estimated_csv_row_bytes(&profile, &calibration);
        let timed = estimate_recording_storage(
            &profile,
            &calibration,
            &RecordingDuration::Timed { seconds: 60 },
        );
        assert_eq!(timed.records_per_second, rate);
        assert_eq!(timed.bmeg_bytes_per_second, rate * record_bytes);
        assert_eq!(timed.csv_bytes_per_second, rate * row_bytes);
        let records = rate * 60;
        let projected = records * record_bytes
            + BMEG_HEADER_RESERVE_BYTES
            + projected_csv_bytes(records, row_bytes);
        assert_eq!(timed.projected_bytes, Some(projected));
        assert_eq!(
            timed.required_free_bytes,
            Some(projected + DISK_CRITICAL_BYTES)
        );
        assert_eq!(timed.critical_free_bytes, DISK_CRITICAL_BYTES);

        let open =
            estimate_recording_storage(&profile, &calibration, &RecordingDuration::UntilStopped);
        assert_eq!(open.projected_bytes, None);
        assert_eq!(open.required_free_bytes, None);
        assert_eq!(open.csv_bytes_per_second, timed.csv_bytes_per_second);

        // A pulse-ox record is one RED/DARK 1/IR/DARK 2 cycle of four dwells.
        let mut pulseox = built_in_profiles()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .find(|profile| profile.category == "course_pulseox")
            .unwrap_or_else(|| panic!("missing pulse-ox profile"));
        pulseox.acquisition.state_dwell_us = Some(1_000);
        let pulseox = pulseox.snapshot(false);
        let estimate = estimate_recording_storage(
            &pulseox,
            &calibration,
            &RecordingDuration::Timed { seconds: 10 },
        );
        assert_eq!(estimate.records_per_second, 250);
        assert_eq!(
            estimate.bmeg_bytes_per_second,
            250 * synchronized_record_bytes(8) as u64
        );
    }

    #[test]
    fn storage_estimate_is_not_smaller_than_a_real_recording() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        let (result, _transport) = run_simulated_capture(&session, dir.path(), 200, |_| {});
        result.unwrap_or_else(|e| panic!("{e}"));
        let summary = last_summary(&session);
        let profile = default_general_profile().unwrap_or_else(|e| panic!("{e}"));
        let calibration = RecordingCalibration::default();
        let fields = profile.profile.acquisition.record_field_names().len();
        let file_size = |path: &str| {
            fs::metadata(path)
                .unwrap_or_else(|e| panic!("{path}: {e}"))
                .len()
        };
        assert!(
            file_size(&summary.bmeg_path)
                <= summary.samples * synchronized_record_bytes(fields) as u64
                    + BMEG_HEADER_RESERVE_BYTES
        );
        assert!(
            file_size(&summary.csv_path)
                <= projected_csv_bytes(
                    summary.samples,
                    estimated_csv_row_bytes(&profile, &calibration)
                )
        );
    }
}
