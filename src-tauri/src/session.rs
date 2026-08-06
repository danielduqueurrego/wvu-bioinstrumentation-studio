//! Phase 1 production session controller.
//!
//! The controller is shared by Tauri commands, but a worker owns every blocking
//! transport read and disk write. Status and the bounded display history are copied
//! under a short mutex so polling the UI never waits for serial I/O.
use crate::{
    acquisition::{AcquisitionController, AcquisitionSnapshot},
    protocol::{
        encode_frame, Frame, FrameParser, IntegrityCounters, MessageType, SampleBatch,
        REFERENCE_DEVICE_ID, REFERENCE_FIRMWARE_BUILD,
    },
    recording::{
        export_bmeg_csv, BmegWriter, RawSample, RecordingDuration, RecordingMetadata, StopReason,
    },
};
use chrono::{Local, Utc};
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

const DISPLAY_CAPACITY: usize = 1_500;
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
    recent: VecDeque<RawSample>,
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
                protocol_version: "0.1".into(),
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

    pub fn recent_samples(&self) -> Result<Vec<RawSample>, SessionError> {
        Ok(self.lock_runtime()?.recent.iter().copied().collect())
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

    /// Starts the common production path on a worker. This returns quickly; callers poll status.
    pub fn start_simulator(
        &self,
        duration: RecordingDuration,
        output_dir: PathBuf,
    ) -> Result<SessionStatus, SessionError> {
        self.validate_duration(&duration)?;
        self.begin_session(true, "Simulator", "SIM", duration.clone())?;
        let controller = self.clone();
        self.spawn_worker(move || controller.capture_simulator_worker(duration, output_dir))?;
        self.status()
    }

    /// Starts the common production path with a serial transport on a worker.
    pub fn start_serial(
        &self,
        port_name: String,
        duration: RecordingDuration,
        output_dir: PathBuf,
    ) -> Result<SessionStatus, SessionError> {
        self.validate_duration(&duration)?;
        self.begin_session(false, "Arduino UNO R4 WiFi", &port_name, duration.clone())?;
        let controller = self.clone();
        self.spawn_worker(move || {
            controller.capture_serial_worker(port_name, duration, output_dir)
        })?;
        self.status()
    }

    /// Synchronous entry point used by the controlled acceptance harness and tests.
    pub fn capture_simulator(
        &self,
        duration: RecordingDuration,
        output_dir: &Path,
    ) -> Result<SessionSummary, SessionError> {
        self.validate_duration(&duration)?;
        self.begin_session(true, "Simulator", "SIM", duration.clone())?;
        self.capture_simulator_worker(duration, output_dir.to_path_buf())?;
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
        self.begin_session(true, "Simulator", "SIM", duration.clone())?;
        self.capture_simulator_worker(duration, output_dir.to_path_buf())?;
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
        self.validate_duration(&duration)?;
        self.begin_session(false, "Arduino UNO R4 WiFi", port_name, duration.clone())?;
        self.capture_serial_worker(port_name.to_owned(), duration, output_dir.to_path_buf())?;
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
            let mut port = serialport::new(&target.port, 115_200)
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
            let mut port = serialport::new(&final_port, 115_200)
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
    ) -> Result<(), SessionError> {
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
        duration: RecordingDuration,
        output_dir: PathBuf,
    ) -> Result<(), SessionError> {
        let result = (|| {
            let mut simulator = SimulatorIo::new(duration.clone())?;
            self.set_state(SessionState::Connected)?;
            self.capture_transport(&mut simulator, true, "SIM", duration, &output_dir)
        })();
        self.finish_worker_error(&result)?;
        result
    }

    fn capture_serial_worker(
        &self,
        port_name: String,
        duration: RecordingDuration,
        output_dir: PathBuf,
    ) -> Result<(), SessionError> {
        let result = (|| {
            let mut port = serialport::new(&port_name, 115_200)
                .timeout(Duration::from_millis(25))
                .open()?;
            self.mark_port_opened(&port_name)?;
            // Clear stale bytes before asserting host control lines. The firmware then emits HELLO.
            port.clear(serialport::ClearBuffer::Input)?;
            port.write_data_terminal_ready(true)?;
            port.write_request_to_send(true)?;
            // wait_for_handshake passively listens through the bounded startup grace
            // before its first PING, so a healthy board can announce itself first.
            self.set_state(SessionState::Connected)?;
            self.capture_transport(&mut port, false, &port_name, duration, &output_dir)
        })();
        self.finish_worker_error(&result)?;
        result
    }

    fn capture_transport<T: Read + Write>(
        &self,
        io: &mut T,
        simulator: bool,
        source: &str,
        duration: RecordingDuration,
        output_dir: &Path,
    ) -> Result<(), SessionError> {
        // Collect tooling provenance before START so no external command can delay raw intake.
        let (temporary_bmeg, bmeg, csv, metadata) = self.allocate_paths(output_dir)?;
        let initial_free_disk_bytes = self.free_disk_space(output_dir)?;
        self.update_disk_space(initial_free_disk_bytes)?;
        if initial_free_disk_bytes < DISK_CRITICAL_BYTES {
            return Err(SessionError::Storage(format!(
                "only {} MiB free in {}; recording requires at least {} MiB",
                initial_free_disk_bytes / (1024 * 1024),
                output_dir.display(),
                DISK_CRITICAL_BYTES / (1024 * 1024)
            )));
        }
        let mut initial_meta = self.initial_metadata(simulator, source, &bmeg, &duration)?;
        initial_meta.initial_free_disk_bytes = Some(initial_free_disk_bytes);
        let (tx, rx) = sync_channel(4_096);
        let mut acquisition = AcquisitionController::new(tx);
        self.wait_for_handshake(io, &mut acquisition)?;
        self.send_command(
            io,
            MessageType::Configure,
            1,
            vec![0xe8, 3, 0, 0, 12, 1, 0, 0],
        )?;
        self.wait_until(io, &mut acquisition, |s| s.config_ack_seen, "CONFIG_ACK")?;
        acquisition.configure().map_err(SessionError::State)?;
        self.set_state(SessionState::Configured)?;
        // The recording is open before START, so every validated post-start sample has a sink.
        let mut raw = BmegWriter::create(&temporary_bmeg, &initial_meta)?;
        acquisition.start().map_err(SessionError::State)?;
        self.send_command(io, MessageType::Start, 2, vec![])?;
        self.wait_until(io, &mut acquisition, |s| s.status_seen, "START status")?;
        self.set_state(SessionState::Acquiring)?;

        let started = Instant::now();
        let mut last_ping = Instant::now();
        let mut last_disk_check = Instant::now();
        let mut last_flush = Instant::now();
        let mut buffer = [0u8; 512];
        let mut terminal_error = None;
        let mut stop_reason = None;

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
                Ok(n) if n > 0 => acquisition.ingest_bytes(&buffer[..n]),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => {
                    terminal_error = Some(error);
                    break;
                }
            }
            self.drain_samples(&rx, &mut raw)?;
            self.update_from_acquisition(&acquisition)?;
            if last_ping.elapsed() >= Duration::from_secs(1) {
                if let Err(error) = self.send_command(io, MessageType::Ping, 3, vec![]) {
                    terminal_error = Some(std::io::Error::other(error.to_string()));
                    break;
                }
                last_ping = Instant::now();
            }
            if last_flush.elapsed() >= RECORDING_FLUSH_INTERVAL {
                raw.flush()?;
                last_flush = Instant::now();
            }
            if last_disk_check.elapsed() >= DISK_CHECK_INTERVAL {
                let free = self.free_disk_space(output_dir)?;
                self.update_disk_space(free)?;
                if free < DISK_CRITICAL_BYTES {
                    stop_reason = Some(self.set_stop_reason_once(StopReason::StorageGuard)?);
                    break;
                }
                last_disk_check = Instant::now();
            }
        }

        let _ = self.send_command(io, MessageType::Stop, 4, vec![]);
        self.drain_samples(&rx, &mut raw)?;
        raw.finish()?;
        fs::rename(&temporary_bmeg, &bmeg)?;
        let csv_rows = export_bmeg_csv(&bmeg, &csv)?;
        let mut snapshot = acquisition.snapshot();
        if terminal_error.is_some() {
            snapshot.integrity.disconnect_events += 1;
        }
        self.update_snapshot(&snapshot)?;
        let reason = if terminal_error.is_some() {
            StopReason::Disconnect
        } else {
            stop_reason.unwrap_or(StopReason::Fault)
        };
        let host_elapsed = started.elapsed().as_secs_f64();
        let board_elapsed = board_elapsed_seconds(&snapshot);
        let recording_status = reason.recording_status();
        let final_free_disk_bytes = self.free_disk_space(output_dir).ok();
        if let Some(free) = final_free_disk_bytes {
            self.update_disk_space(free)?;
        }
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
        fs::write(
            &metadata,
            serde_json::to_vec_pretty(&final_meta)
                .map_err(crate::recording::RecordingError::Json)?,
        )?;
        let summary = SessionSummary {
            state: if terminal_error.is_some() {
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
            error: terminal_error
                .as_ref()
                .map(std::string::ToString::to_string),
        };
        self.set_summary(summary.clone())?;
        self.set_runtime_stop_reason(reason)?;
        if let Some(error) = terminal_error {
            self.set_fault(error.to_string())?;
            return Err(SessionError::Io(error));
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
                        "handshake reached protocol v0.1 on {} but firmware identity was not accepted: build={:?}, device={:?}. {}",
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
        let bytes = encode_frame(&Frame {
            message_type,
            flags: 0,
            sequence,
            payload,
        })
        .map_err(|error| SessionError::Protocol(format!("{error:?}")))?;
        io.write_all(&bytes)?;
        io.flush()?;
        Ok(())
    }

    fn drain_samples(
        &self,
        receiver: &Receiver<RawSample>,
        raw: &mut BmegWriter,
    ) -> Result<(), SessionError> {
        for sample in receiver.try_iter() {
            raw.write(sample)?;
            let mut runtime = self.lock_runtime()?;
            if runtime.recent.len() == DISPLAY_CAPACITY {
                runtime.recent.pop_front();
            }
            runtime.recent.push_back(sample);
        }
        Ok(())
    }

    fn initial_metadata(
        &self,
        simulator: bool,
        source: &str,
        bmeg: &Path,
        duration: &RecordingDuration,
    ) -> Result<RecordingMetadata, SessionError> {
        let (arduino_cli_version, uno_r4_core_version) = if simulator {
            ("not applicable".into(), "not applicable".into())
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
            (version, core)
        };
        Ok(RecordingMetadata {
            utc_start: Utc::now(),
            local_start: Local::now(),
            board: if simulator {
                "Simulator".into()
            } else {
                "Arduino UNO R4 WiFi".into()
            },
            com_port: source.into(),
            fqbn: if simulator {
                "simulator".into()
            } else {
                "arduino:renesas_uno:unor4wifi".into()
            },
            arduino_cli_version,
            uno_r4_core_version,
            firmware_build: REFERENCE_FIRMWARE_BUILD,
            protocol_version: "0.1".into(),
            analog_pin: "A0".into(),
            adc_bits: 12,
            requested_sample_rate_hz: 1_000,
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
                "Deterministic simulator waveform; no human signal.".into()
            } else {
                "A0 raw floating/uncalibrated engineering communication test; no human signal."
                    .into()
            },
            duration_mode: Some(duration.label().into()),
            requested_duration_seconds: duration.requested_seconds(),
            stop_reason: None,
            initial_free_disk_bytes: None,
            final_free_disk_bytes: None,
            completion_status: "active".into(),
        })
    }

    fn allocate_paths(
        &self,
        output_dir: &Path,
    ) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), SessionError> {
        fs::create_dir_all(output_dir)?;
        let stamp = Local::now().format("%Y%m%d_%H%M%S");
        for run in 1..=99 {
            let base = output_dir.join(format!("{stamp}_Phase1_A0_Run{run:02}"));
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
                self.set_fault(error.to_string())?;
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
                    ConnectionFailureCategory::DeviceDisconnected
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
            Some("0.1".into())
        } else {
            None
        };
        diagnostics.firmware_build = snapshot.firmware_build;
        diagnostics.firmware_board_id = snapshot.firmware_board_id;
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
        let elapsed_seconds = runtime
            .started_at
            .map(|started| started.elapsed().as_secs_f64())
            .unwrap_or(0.0);
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
}

impl SimulatorIo {
    fn new(duration: RecordingDuration) -> Result<Self, SessionError> {
        let mut simulator = Self {
            bytes: VecDeque::new(),
            commands: FrameParser::default(),
            packet_sequence: 0,
            sample_sequence: 0,
            sample_limit: duration
                .requested_seconds()
                .map(|seconds| seconds.saturating_mul(1_000)),
            active: false,
            next_batch_at: Instant::now(),
            batch_interval: Some(Duration::from_millis(10)),
            max_fragment: 7,
        };
        simulator.queue(
            MessageType::Hello,
            vec![1, 0, 1, 0, 0x34, 0x4f, 0x4e, 0x55, 1, 12, 1, 0],
        )?;
        simulator.queue(MessageType::Capabilities, vec![12, 1, 6, 0])?;
        Ok(simulator)
    }

    #[cfg(test)]
    fn new_accelerated(duration: RecordingDuration) -> Result<Self, SessionError> {
        let mut simulator = Self::new(duration)?;
        simulator.batch_interval = None;
        simulator.max_fragment = 128;
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
        let samples = (0..count)
            .map(|index| {
                let phase =
                    (self.sample_sequence + index) as f64 * std::f64::consts::TAU * 2.0 / 1_000.0;
                (2048.0 + phase.sin() * 1200.0).clamp(0.0, 4095.0) as u16
            })
            .collect();
        let payload = SampleBatch {
            first_sample_sequence: self.sample_sequence,
            first_timestamp_us: u64::from(self.sample_sequence) * 1_000,
            sample_period_us: 1_000,
            channel_count: 1,
            samples,
            status_flags: 1,
        }
        .to_payload()
        .map_err(|error| SessionError::Protocol(format!("{error:?}")))?;
        self.sample_sequence = self.sample_sequence.wrapping_add(count);
        self.queue(MessageType::SampleBatch, payload)
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
                MessageType::Configure => self.queue(MessageType::ConfigAck, vec![]),
                MessageType::Start => {
                    self.active = true;
                    self.next_batch_at = Instant::now();
                    self.queue(MessageType::Status, vec![])
                }
                MessageType::Stop => {
                    self.active = false;
                    self.queue(MessageType::Status, vec![])
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
        assert_eq!(
            session.recent_samples().unwrap_or_default().len(),
            DISPLAY_CAPACITY
        );
        assert!(BmegReader::open(Path::new(&summary.bmeg_path)).is_ok());
        assert_eq!(
            std::fs::read_to_string(&summary.csv_path)
                .unwrap_or_default()
                .lines()
                .count(),
            2_001
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
                hello_payload: vec![1, 0, 1, 0, 0x34, 0x4f, 0x4e, 0x55, 1, 12, 1, 0],
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
                Duration::from_millis(1),
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
        io.hello_payload[0] = 2;
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
            .initial_metadata(true, "SIM", &bmeg, &duration)
            .unwrap_or_else(|e| panic!("{e}"));
        let mut writer = BmegWriter::create(&bmeg, &metadata).unwrap_or_else(|e| panic!("{e}"));
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
                writer.write(sample).unwrap_or_else(|e| panic!("{e}"));
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

    #[test]
    fn terminal_transport_error_finalizes_a_readable_disconnected_recording() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        session
            .begin_session(
                true,
                "Simulator",
                "SIM",
                RecordingDuration::Timed { seconds: 2 },
            )
            .unwrap_or_else(|e| panic!("{e}"));
        session
            .set_state(SessionState::Connected)
            .unwrap_or_else(|e| panic!("{e}"));
        let mut transport = DisconnectingSimulator {
            inner: SimulatorIo::new(RecordingDuration::Timed { seconds: 2 })
                .unwrap_or_else(|e| panic!("{e}")),
            reads: 0,
            fail_after: 300,
        };
        assert!(session
            .capture_transport(
                &mut transport,
                true,
                "SIM",
                RecordingDuration::Timed { seconds: 2 },
                dir.path(),
            )
            .is_err());
        let summary = session
            .status()
            .unwrap_or_else(|e| panic!("{e}"))
            .last_summary
            .unwrap_or_else(|| panic!("missing interrupted summary"));
        assert_eq!(summary.recording_status, "disconnected");
        assert_eq!(summary.integrity.disconnect_events, 1);
        assert!(BmegReader::open(Path::new(&summary.bmeg_path)).is_ok());
    }
}
