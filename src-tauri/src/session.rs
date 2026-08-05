//! Phase 1 production session controller.
//!
//! The controller is shared by Tauri commands, but a worker owns every blocking
//! transport read and disk write. Status and the bounded display history are copied
//! under a short mutex so polling the UI never waits for serial I/O.
use crate::{
    acquisition::{AcquisitionController, AcquisitionSnapshot},
    protocol::{encode_frame, Frame, FrameParser, IntegrityCounters, MessageType, SampleBatch},
    recording::{export_bmeg_csv, BmegWriter, RawSample, RecordingMetadata},
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
const MAX_CAPTURE_SECONDS: u32 = 600;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);

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
}

#[derive(Clone)]
pub struct SessionController {
    runtime: Arc<Mutex<SessionRuntime>>,
    cancel: Arc<AtomicBool>,
    worker: Arc<Mutex<Option<JoinHandle<()>>>>,
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
}

impl Default for SessionController {
    fn default() -> Self {
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
            })),
            cancel: Arc::new(AtomicBool::new(false)),
            worker: Arc::new(Mutex::new(None)),
        }
    }
}

impl SessionController {
    pub fn status(&self) -> Result<SessionStatus, SessionError> {
        let runtime = self.lock_runtime()?;
        Ok(SessionStatus {
            state: runtime.state,
            board: runtime.board.clone(),
            port: runtime.port.clone(),
            protocol_version: runtime.protocol_version.clone(),
            simulator: runtime.simulator,
            samples: runtime.samples,
            packets: runtime.packets,
            measured_rate_hz: runtime.measured_rate_hz,
            integrity: runtime.integrity.clone(),
            last_error: runtime.last_error.clone(),
            last_summary: runtime.last_summary.clone(),
        })
    }

    pub fn recent_samples(&self) -> Result<Vec<RawSample>, SessionError> {
        Ok(self.lock_runtime()?.recent.iter().copied().collect())
    }

    /// Starts the common production path on a worker. This returns quickly; callers poll status.
    pub fn start_simulator(
        &self,
        seconds: u32,
        output_dir: PathBuf,
    ) -> Result<SessionStatus, SessionError> {
        self.validate_duration(seconds)?;
        self.begin_session(true, "Simulator", "SIM")?;
        let controller = self.clone();
        self.spawn_worker(move || controller.capture_simulator_worker(seconds, output_dir))?;
        self.status()
    }

    /// Starts the common production path with a serial transport on a worker.
    pub fn start_serial(
        &self,
        port_name: String,
        seconds: u32,
        output_dir: PathBuf,
    ) -> Result<SessionStatus, SessionError> {
        self.validate_duration(seconds)?;
        self.begin_session(false, "Arduino UNO R4 WiFi", &port_name)?;
        let controller = self.clone();
        self.spawn_worker(move || {
            controller.capture_serial_worker(port_name, seconds, output_dir)
        })?;
        self.status()
    }

    /// Synchronous entry point used by the controlled acceptance harness and tests.
    pub fn capture_simulator(
        &self,
        seconds: u32,
        output_dir: &Path,
    ) -> Result<SessionSummary, SessionError> {
        self.validate_duration(seconds)?;
        self.begin_session(true, "Simulator", "SIM")?;
        self.capture_simulator_worker(seconds, output_dir.to_path_buf())?;
        self.status()?.last_summary.ok_or(SessionError::State(
            "simulator session did not produce a summary",
        ))
    }

    /// Synchronous serial entry point used by the controlled acceptance harness.
    pub fn capture_serial(
        &self,
        port_name: &str,
        seconds: u32,
        output_dir: &Path,
    ) -> Result<SessionSummary, SessionError> {
        self.validate_duration(seconds)?;
        self.begin_session(false, "Arduino UNO R4 WiFi", port_name)?;
        self.capture_serial_worker(port_name.to_owned(), seconds, output_dir.to_path_buf())?;
        self.status()?.last_summary.ok_or(SessionError::State(
            "serial session did not produce a summary",
        ))
    }

    /// Idempotent: it asks a running worker to finish the current recording.
    pub fn request_stop(&self) -> Result<SessionStatus, SessionError> {
        {
            let mut runtime = self.lock_runtime()?;
            if matches!(
                runtime.state,
                SessionState::Acquiring | SessionState::Configured | SessionState::Connecting
            ) {
                runtime.state = SessionState::Stopping;
            }
        }
        self.cancel.store(true, Ordering::Release);
        self.status()
    }

    /// Idempotent disconnect. The worker is joined without holding the runtime mutex.
    pub fn disconnect(&self) -> Result<SessionStatus, SessionError> {
        self.cancel.store(true, Ordering::Release);
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
        Ok(SessionStatus {
            state: runtime.state,
            board: runtime.board.clone(),
            port: runtime.port.clone(),
            protocol_version: runtime.protocol_version.clone(),
            simulator: runtime.simulator,
            samples: runtime.samples,
            packets: runtime.packets,
            measured_rate_hz: runtime.measured_rate_hz,
            integrity: runtime.integrity.clone(),
            last_error: runtime.last_error.clone(),
            last_summary: runtime.last_summary.clone(),
        })
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

    fn validate_duration(&self, seconds: u32) -> Result<(), SessionError> {
        if seconds == 0 || seconds > MAX_CAPTURE_SECONDS {
            return Err(SessionError::State("duration must be 1–600 seconds"));
        }
        Ok(())
    }

    fn begin_session(&self, simulator: bool, board: &str, port: &str) -> Result<(), SessionError> {
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
        self.cancel.store(false, Ordering::Release);
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
        seconds: u32,
        output_dir: PathBuf,
    ) -> Result<(), SessionError> {
        let result = (|| {
            let mut simulator = SimulatorIo::new(seconds)?;
            self.set_state(SessionState::Connected)?;
            self.capture_transport(&mut simulator, true, "SIM", seconds, &output_dir)
        })();
        self.finish_worker_error(&result)?;
        result
    }

    fn capture_serial_worker(
        &self,
        port_name: String,
        seconds: u32,
        output_dir: PathBuf,
    ) -> Result<(), SessionError> {
        let result = (|| {
            let mut port = serialport::new(&port_name, 115_200)
                .timeout(Duration::from_millis(25))
                .open()?;
            // Clear stale bytes before asserting host control lines. The firmware then emits HELLO.
            port.clear(serialport::ClearBuffer::Input)?;
            port.write_data_terminal_ready(true)?;
            port.write_request_to_send(true)?;
            // The reference sketch deliberately allows up to one second for USB CDC to enumerate.
            // Sending CONFIGURE/PING sooner can race its startup serial initialization on Windows.
            thread::sleep(Duration::from_millis(1_250));
            self.set_state(SessionState::Connected)?;
            self.capture_transport(&mut port, false, &port_name, seconds, &output_dir)
        })();
        self.finish_worker_error(&result)?;
        result
    }

    fn capture_transport<T: Read + Write>(
        &self,
        io: &mut T,
        simulator: bool,
        source: &str,
        seconds: u32,
        output_dir: &Path,
    ) -> Result<(), SessionError> {
        // Collect tooling provenance before START so no external command can delay raw intake.
        let (temporary_bmeg, bmeg, csv, metadata) = self.allocate_paths(output_dir)?;
        let initial_meta = self.initial_metadata(simulator, source, &bmeg)?;
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
        let mut buffer = [0u8; 512];
        let mut terminal_error = None;
        let mut stopped_by_user = false;

        while started.elapsed() < Duration::from_secs(u64::from(seconds)) {
            if self.cancel.load(Ordering::Acquire) {
                stopped_by_user = true;
                break;
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
        let host_elapsed = started.elapsed().as_secs_f64();
        let board_elapsed = board_elapsed_seconds(&snapshot);
        let recording_status = if terminal_error.is_some() {
            "disconnected"
        } else if stopped_by_user {
            "stopped_by_user"
        } else {
            "complete"
        };
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
            integrity: snapshot.integrity,
            error: terminal_error
                .as_ref()
                .map(std::string::ToString::to_string),
        };
        self.set_summary(summary.clone())?;
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
        self.send_command(io, MessageType::Ping, 0, vec![])?;
        self.wait_until(
            io,
            acquisition,
            |s| s.hello_seen && s.capabilities_seen && s.pong_seen,
            "HELLO, CAPABILITIES, and PONG",
        )
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
            firmware_build: 0x0001_0000,
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
                self.set_fault(error.to_string())?;
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

fn board_elapsed_seconds(snapshot: &AcquisitionSnapshot) -> f64 {
    if snapshot.sample_count > 1 && snapshot.measured_rate_hz > 0.0 {
        (snapshot.sample_count - 1) as f64 / snapshot.measured_rate_hz
    } else {
        0.0
    }
}

/// Deterministic device transport. Commands are decoded as normal frames and cause the
/// matching firmware responses, while sample batches are generated lazily.
struct SimulatorIo {
    bytes: VecDeque<u8>,
    commands: FrameParser,
    packet_sequence: u32,
    sample_sequence: u32,
    sample_limit: u32,
    active: bool,
}

impl SimulatorIo {
    fn new(seconds: u32) -> Result<Self, SessionError> {
        let mut simulator = Self {
            bytes: VecDeque::new(),
            commands: FrameParser::default(),
            packet_sequence: 0,
            sample_sequence: 0,
            sample_limit: seconds.saturating_mul(1_000),
            active: false,
        };
        simulator.queue(
            MessageType::Hello,
            vec![0, 0, 1, 0, 0x34, 0x4f, 0x4e, 0x55, 1, 12, 1, 0],
        )?;
        simulator.queue(MessageType::Capabilities, vec![12, 1, 6, 0])?;
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
        if self.sample_sequence >= self.sample_limit {
            return Ok(());
        }
        let count = (self.sample_limit - self.sample_sequence).min(10);
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
        self.sample_sequence += count;
        self.queue(MessageType::SampleBatch, payload)
    }
}

impl Read for SimulatorIo {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.bytes.is_empty() && self.active && self.sample_sequence < self.sample_limit {
            self.queue_sample_batch()
                .map_err(|error| std::io::Error::other(error.to_string()))?;
        }
        if self.bytes.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "simulator idle",
            ));
        }
        let take = self.bytes.len().min(buffer.len()).min(7);
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
    use crate::recording::BmegReader;
    use tempfile::tempdir;

    #[test]
    fn simulator_complete_session_is_bounded_and_recorded() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        let summary = session
            .capture_simulator(2, dir.path())
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

    #[test]
    fn duplicate_start_is_rejected_while_worker_runs() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        session
            .start_simulator(2, dir.path().to_path_buf())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(session
            .start_simulator(1, dir.path().to_path_buf())
            .is_err());
        session.request_stop().unwrap_or_else(|e| panic!("{e}"));
        session.wait_for_worker().unwrap_or_else(|e| panic!("{e}"));
    }

    #[test]
    fn repeated_simulator_sessions_allocate_distinct_recordings() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let session = SessionController::default();
        let first = session
            .capture_simulator(1, dir.path())
            .unwrap_or_else(|e| panic!("{e}"));
        let second = session
            .capture_simulator(1, dir.path())
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
            .begin_session(true, "Simulator", "SIM")
            .unwrap_or_else(|e| panic!("{e}"));
        session
            .set_state(SessionState::Connected)
            .unwrap_or_else(|e| panic!("{e}"));
        let mut transport = DisconnectingSimulator {
            inner: SimulatorIo::new(2).unwrap_or_else(|e| panic!("{e}")),
            reads: 0,
            fail_after: 30,
        };
        assert!(session
            .capture_transport(&mut transport, true, "SIM", 2, dir.path())
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
