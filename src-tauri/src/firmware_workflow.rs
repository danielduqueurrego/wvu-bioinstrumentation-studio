//! WVU reference-firmware restore and verification orchestration.
//!
//! This controller coordinates the acquisition `SessionController`, the safe
//! Arduino CLI adapter, and one active restore job. It intentionally does not
//! expose a general sketch editor, compiler, or arbitrary upload path.
use crate::{
    arduino_cli::{
        parse_compile_usage, parse_compiler_diagnostics, ArduinoCli, BoardInfo, CommandLog,
        CompileUsage, CompilerDiagnostic, UNO_R4_WIFI_FQBN,
    },
    protocol::{PROTOCOL_MAJOR, PROTOCOL_MINOR, REFERENCE_DEVICE_ID, REFERENCE_FIRMWARE_BUILD},
    reference_firmware::{
        controlled_reference_source, source_hash, FirmwareIdentity, FirmwareVerificationKind,
    },
    session::{ResetTarget, SessionController},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const APPLICATION_PORT_TIMEOUT: Duration = Duration::from_secs(15);
const APPLICATION_PORT_POLL: Duration = Duration::from_millis(300);
const NATIVE_USB_PID: u16 = 0x006d;
const ESP32_BRIDGE_PID: u16 = 0x1002;
const NATIVE_USB_MANUAL_RESET_TIMEOUT: Duration = Duration::from_secs(90);
const RETAINED_FIRMWARE_JOB_LOGS: usize = 30;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirmwareCompatibility {
    Unknown,
    WvuProtocolCompatible,
    WvuProtocolIncompatible,
    UploadInProgress,
    VerificationFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirmwareJobKind {
    RestoreReference,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirmwareJobStage {
    Idle,
    Preparing,
    ClosingSerialSession,
    Compiling,
    TouchReset,
    WaitingForBootloader,
    Uploading,
    WaitingForApplicationPort,
    VerifyingFirmware,
    Complete,
    Failed,
    Canceled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirmwareErrorCategory {
    ArduinoCliMissing,
    CoreMissing,
    ConfirmationRequired,
    CompileFailed,
    PortBusy,
    BoardNotFound,
    AmbiguousBoard,
    UploadFailed,
    ApplicationPortNotFound,
    ManualResetRequired,
    ProtocolVerificationFailed,
    WrongFirmwareIdentity,
    Canceled,
    InternalError,
}

#[derive(Clone, Debug, Serialize)]
pub struct FirmwareFailure {
    pub category: FirmwareErrorCategory,
    pub stage: FirmwareJobStage,
    pub title: String,
    pub explanation: String,
    pub recommended_action: String,
    pub technical_details: String,
}

impl FirmwareFailure {
    fn new(
        category: FirmwareErrorCategory,
        stage: FirmwareJobStage,
        explanation: impl Into<String>,
        technical_details: impl Into<String>,
    ) -> Self {
        let (title, recommended_action) = failure_copy(category);
        Self {
            category,
            stage,
            title: title.into(),
            explanation: explanation.into(),
            recommended_action: recommended_action.into(),
            technical_details: technical_details.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FirmwareJobStatus {
    pub id: u64,
    pub kind: FirmwareJobKind,
    pub stage: FirmwareJobStage,
    pub active: bool,
    /// SHA-256 of the immutable reference source compiled for this restore.
    pub source_hash: Option<String>,
    pub original_port: Option<String>,
    pub bootloader_port: Option<String>,
    pub final_port: Option<String>,
    pub board_serial: Option<String>,
    pub started_utc: DateTime<Utc>,
    pub completed_utc: Option<DateTime<Utc>>,
    pub message: String,
    pub compile_usage: Option<CompileUsage>,
    pub diagnostics: Vec<CompilerDiagnostic>,
    pub compile_log: Option<CommandLog>,
    pub upload_log: Option<CommandLog>,
    pub verification: Option<FirmwareVerification>,
    pub failure: Option<FirmwareFailure>,
    pub log_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FirmwareVerification {
    pub declared_kind: FirmwareVerificationKind,
    pub compatible: bool,
    pub protocol_version: Option<String>,
    pub identity: Option<FirmwareIdentity>,
    pub bytes_received: Option<u64>,
    pub valid_frames: Option<u64>,
    pub crc_failures: Option<u64>,
    pub explanation: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct FirmwareEnvironmentStatus {
    pub cli_path: Option<String>,
    pub cli_version: Option<String>,
    pub uno_r4_core_version: Option<String>,
    pub expected_fqbn: String,
    pub boards: Vec<BoardInfo>,
    pub ready: bool,
    pub problem: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct RestoreReferenceRequest {
    pub port: String,
    pub confirmation: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct FirmwareWorkflowStatus {
    pub compatibility: FirmwareCompatibility,
    pub job: Option<FirmwareJobStatus>,
    pub last_restore: Option<FirmwareJobStatus>,
    pub last_failure: Option<FirmwareFailure>,
}

#[derive(Clone)]
pub struct FirmwareWorkflow {
    session: SessionController,
    state: Arc<Mutex<WorkflowRuntime>>,
    cancel: Arc<AtomicBool>,
    worker: Arc<Mutex<Option<JoinHandle<()>>>>,
    next_job_id: Arc<AtomicU64>,
}

struct WorkflowRuntime {
    compatibility: FirmwareCompatibility,
    active_job: Option<FirmwareJobStatus>,
    last_restore: Option<FirmwareJobStatus>,
    last_failure: Option<FirmwareFailure>,
}

impl FirmwareWorkflow {
    pub fn new(session: SessionController) -> Self {
        Self {
            session,
            state: Arc::new(Mutex::new(WorkflowRuntime {
                compatibility: FirmwareCompatibility::Unknown,
                active_job: None,
                last_restore: None,
                last_failure: None,
            })),
            cancel: Arc::new(AtomicBool::new(false)),
            worker: Arc::new(Mutex::new(None)),
            next_job_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn status(&self) -> Result<FirmwareWorkflowStatus, FirmwareFailure> {
        let runtime = self.lock_state()?;
        Ok(FirmwareWorkflowStatus {
            compatibility: runtime.compatibility,
            job: runtime.active_job.clone(),
            last_restore: runtime.last_restore.clone(),
            last_failure: runtime.last_failure.clone(),
        })
    }

    pub fn environment(&self) -> FirmwareEnvironmentStatus {
        let cli = match ArduinoCli::discover(None) {
            Ok(cli) => cli,
            Err(error) => {
                return FirmwareEnvironmentStatus {
                    cli_path: None,
                    cli_version: None,
                    uno_r4_core_version: None,
                    expected_fqbn: UNO_R4_WIFI_FQBN.into(),
                    boards: Vec::new(),
                    ready: false,
                    problem: Some(error.to_string()),
                }
            }
        };
        let cli_path = Some(cli.executable().display().to_string());
        let cli_version = cli.version().ok().map(|log| log.stdout.trim().to_owned());
        let uno_r4_core_version = cli.uno_r4_core_version().ok();
        let problem = if cli_version.is_none() {
            Some("Arduino CLI is present but did not return a version.".into())
        } else if uno_r4_core_version.is_none() {
            Some("Arduino UNO R4 core is not installed. Run `arduino-cli core install arduino:renesas_uno`.".into())
        } else {
            None
        };
        FirmwareEnvironmentStatus {
            cli_path,
            cli_version,
            uno_r4_core_version,
            expected_fqbn: UNO_R4_WIFI_FQBN.into(),
            // Board enumeration intentionally lives in the application-level discovery
            // workflow so opening the application does not spuriously rescan devices.
            boards: Vec::new(),
            ready: problem.is_none(),
            problem,
        }
    }

    pub fn start_restore_reference(
        &self,
        request: RestoreReferenceRequest,
    ) -> Result<FirmwareJobStatus, FirmwareFailure> {
        if !request.confirmation {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::ConfirmationRequired,
                FirmwareJobStage::Preparing,
                "Restoring the WVU reference firmware replaces the sketch currently on the board.",
                "confirmation=false",
            ));
        }
        let board = self.selected_board(&request.port, FirmwareJobStage::Preparing)?;
        self.ensure_upload_is_safe(FirmwareJobStage::Preparing)?;
        let job = self.new_job(
            FirmwareJobKind::RestoreReference,
            None,
            Some(board.port.clone()),
            board.serial_number.clone(),
        );
        self.start_worker(job.clone(), move |workflow| {
            workflow.restore_reference(job.id, board)
        })?;
        Ok(job)
    }

    pub fn cancel_active_job(&self) -> Result<FirmwareWorkflowStatus, FirmwareFailure> {
        if self.lock_state()?.active_job.is_none() {
            return self.status();
        }
        self.cancel.store(true, Ordering::Release);
        self.set_message("Cancel requested; terminating the current Arduino CLI process.")?;
        self.status()
    }

    pub fn verify_existing_reference(
        &self,
        port: String,
    ) -> Result<FirmwareVerification, FirmwareFailure> {
        let board = self.selected_board(&port, FirmwareJobStage::VerifyingFirmware)?;
        if self.session.is_recording().map_err(session_failure)? {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::PortBusy,
                FirmwareJobStage::VerifyingFirmware,
                "Cannot verify firmware while acquisition owns the serial port.",
                "active acquisition session",
            ));
        }
        let result = self
            .session
            .retry_handshake(ResetTarget {
                port: board.port.clone(),
                serial_number: board.serial_number.clone(),
            })
            .map_err(session_failure)?;
        let verification = verification_from_handshake(
            FirmwareVerificationKind::WvuProtocolReference,
            &result.diagnostics,
            result.handshake_succeeded,
        );
        if verification.compatible {
            self.session
                .prepare_for_new_recording()
                .map_err(session_failure)?;
        }
        let mut runtime = self.lock_state()?;
        runtime.compatibility = if verification.compatible {
            FirmwareCompatibility::WvuProtocolCompatible
        } else {
            FirmwareCompatibility::VerificationFailed
        };
        if !verification.compatible {
            runtime.last_failure = Some(FirmwareFailure::new(
                FirmwareErrorCategory::ProtocolVerificationFailed,
                FirmwareJobStage::VerifyingFirmware,
                verification.explanation.clone(),
                format!("diagnostics: {:?}", result.diagnostics.failure_category),
            ));
        }
        Ok(verification)
    }

    pub fn is_acquisition_allowed(&self, port: &str) -> Result<bool, FirmwareFailure> {
        let runtime = self.lock_state()?;
        Ok(
            runtime.compatibility == FirmwareCompatibility::WvuProtocolCompatible
                && runtime
                    .last_restore
                    .as_ref()
                    .and_then(|job| job.final_port.as_ref().or(job.original_port.as_ref()))
                    .is_none_or(|known| known.eq_ignore_ascii_case(port)),
        )
    }

    fn start_worker<F>(&self, job: FirmwareJobStatus, work: F) -> Result<(), FirmwareFailure>
    where
        F: FnOnce(FirmwareWorkflow) -> Result<(), FirmwareFailure> + Send + 'static,
    {
        {
            let mut worker = self
                .worker
                .lock()
                .map_err(|_| internal_failure("firmware worker lock poisoned"))?;
            if worker.as_ref().is_some_and(|handle| !handle.is_finished()) {
                return Err(FirmwareFailure::new(
                    FirmwareErrorCategory::InternalError,
                    FirmwareJobStage::Preparing,
                    "Another firmware compile or upload is already active.",
                    "one active firmware job is allowed",
                ));
            }
            if let Some(previous) = worker.take() {
                let _ = previous.join();
            }
        }
        self.cancel.store(false, Ordering::Release);
        {
            let mut runtime = self.lock_state()?;
            runtime.active_job = Some(job.clone());
            runtime.last_failure = None;
            runtime.compatibility = FirmwareCompatibility::UploadInProgress;
        }
        let controller = self.clone();
        let handle = thread::spawn(move || {
            if let Err(failure) = work(controller.clone()) {
                let _ = controller.finish_failure(job.id, failure);
            }
        });
        *self
            .worker
            .lock()
            .map_err(|_| internal_failure("firmware worker lock poisoned"))? = Some(handle);
        Ok(())
    }

    fn restore_reference(&self, job_id: u64, board: BoardInfo) -> Result<(), FirmwareFailure> {
        self.set_stage(
            job_id,
            FirmwareJobStage::ClosingSerialSession,
            "Closing any idle acquisition session before restoring the controlled reference.",
        )?;
        self.session.disconnect().map_err(session_failure)?;
        self.set_stage(
            job_id,
            FirmwareJobStage::Compiling,
            "Compiling the controlled WVU reference firmware.",
        )?;
        let cli = required_cli(FirmwareJobStage::Compiling)?;
        require_core(&cli, FirmwareJobStage::Compiling)?;
        let reference_dir = job_directory(job_id).join("reference_unor4wifi");
        fs::create_dir_all(&reference_dir)
            .map_err(|error| io_failure(FirmwareJobStage::Compiling, error))?;
        let source_path = reference_dir.join("reference_unor4wifi.ino");
        fs::write(&source_path, controlled_reference_source())
            .map_err(|error| io_failure(FirmwareJobStage::Compiling, error))?;
        let build_dir = job_directory(job_id).join("compile");
        fs::create_dir_all(&build_dir)
            .map_err(|error| io_failure(FirmwareJobStage::Compiling, error))?;
        let log = cli
            .compile_to(&reference_dir, &build_dir, &self.cancel)
            .map_err(|error| {
                cli_failure(
                    FirmwareJobStage::Compiling,
                    FirmwareErrorCategory::CompileFailed,
                    error,
                )
            })?;
        let usage = parse_compile_usage(&format!("{}\n{}", log.stdout, log.stderr));
        let diagnostics = parse_compiler_diagnostics(&format!("{}\n{}", log.stdout, log.stderr));
        self.update_job(job_id, |job| {
            job.compile_usage = Some(usage);
            job.diagnostics = diagnostics;
            job.compile_log = Some(log.clone());
        })?;
        if log.canceled {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::Canceled,
                FirmwareJobStage::Canceled,
                "Reference compile canceled.",
                "Arduino CLI child was terminated",
            ));
        }
        if !log.succeeded() {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::CompileFailed,
                FirmwareJobStage::Compiling,
                "Controlled reference firmware did not compile.",
                combined_output(&log),
            ));
        }
        let source_hash = source_hash(
            &fs::read(&source_path)
                .map_err(|error| io_failure(FirmwareJobStage::Compiling, error))?,
        );
        let binary_path = build_dir.join("reference_unor4wifi.ino.bin");
        self.update_job(job_id, |job| job.source_hash = Some(source_hash))?;
        if !binary_path.is_file() {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::CompileFailed,
                FirmwareJobStage::Compiling,
                "Reference compile did not produce a binary.",
                binary_path.display().to_string(),
            ));
        }
        self.upload_reference_artifact(job_id, board, binary_path)
    }

    fn upload_reference_artifact(
        &self,
        job_id: u64,
        board: BoardInfo,
        binary_path: PathBuf,
    ) -> Result<(), FirmwareFailure> {
        let cli = required_cli(FirmwareJobStage::Uploading)?;
        require_core(&cli, FirmwareJobStage::Uploading)?;
        let native_restore = uses_native_usb(&board);
        let upload_board = if native_restore {
            self.set_stage(
                job_id,
                FirmwareJobStage::WaitingForBootloader,
                "The WVU firmware is using native USB. Press the board Reset button twice to switch safely to the upload interface.",
            )?;
            let bridge = wait_for_esp32_bridge_port(&cli, &self.cancel)?;
            self.update_job(job_id, |job| {
                job.bootloader_port = Some(bridge.port.clone())
            })?;
            bridge
        } else {
            board.clone()
        };
        self.set_stage(
            job_id,
            FirmwareJobStage::TouchReset,
            if native_restore {
                "The board upload interface is ready. Arduino CLI will complete the controlled upload."
            } else {
                "Arduino CLI will reset the selected UNO R4 WiFi for upload."
            },
        )?;
        self.set_stage(
            job_id,
            FirmwareJobStage::WaitingForBootloader,
            "Waiting for Arduino CLI bootloader/upload transition.",
        )?;
        self.set_stage(
            job_id,
            FirmwareJobStage::Uploading,
            "Uploading the WVU reference firmware to the selected UNO R4 WiFi.",
        )?;
        let log = cli
            .upload_input(&binary_path, &upload_board.port, &self.cancel)
            .map_err(|error| {
                cli_failure(
                    FirmwareJobStage::Uploading,
                    FirmwareErrorCategory::UploadFailed,
                    error,
                )
            })?;
        self.update_job(job_id, |job| job.upload_log = Some(log.clone()))?;
        if log.canceled {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::Canceled,
                FirmwareJobStage::Canceled,
                "Firmware restore was canceled.",
                "Arduino CLI child was terminated",
            ));
        }
        if !log.succeeded() {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::UploadFailed,
                FirmwareJobStage::Uploading,
                "The WVU firmware could not be uploaded; the board may still contain its prior firmware.",
                combined_output(&log),
            ));
        }
        self.set_stage(
            job_id,
            FirmwareJobStage::WaitingForApplicationPort,
            "Waiting for the controlled WVU native-USB application port to return.",
        )?;
        let final_board = wait_for_native_reference_port(&cli, &self.cancel, |candidate| {
            self.update_job(job_id, |job| {
                if !candidate.port.eq_ignore_ascii_case(&board.port) {
                    job.final_port = Some(candidate.port.clone());
                }
            })
        })?;
        self.update_job(job_id, |job| {
            job.final_port = Some(final_board.port.clone())
        })?;
        self.verify_uploaded_reference(job_id, final_board)
    }

    fn verify_uploaded_reference(
        &self,
        job_id: u64,
        board: BoardInfo,
    ) -> Result<(), FirmwareFailure> {
        self.set_stage(
            job_id,
            FirmwareJobStage::VerifyingFirmware,
            "Verifying HELLO, CAPABILITIES, PONG, CRC, and controlled firmware identity.",
        )?;
        let result = self
            .session
            .retry_handshake(ResetTarget {
                port: board.port.clone(),
                serial_number: board.serial_number.clone(),
            })
            .map_err(session_failure)?;
        let verification = verification_from_handshake(
            FirmwareVerificationKind::WvuProtocolReference,
            &result.diagnostics,
            result.handshake_succeeded,
        );
        self.update_job(job_id, |job| job.verification = Some(verification.clone()))?;
        if !verification.compatible {
            return Err(FirmwareFailure::new(
                if result.diagnostics.firmware_build.is_some()
                    || result.diagnostics.firmware_board_id.is_some()
                {
                    FirmwareErrorCategory::WrongFirmwareIdentity
                } else {
                    FirmwareErrorCategory::ProtocolVerificationFailed
                },
                FirmwareJobStage::VerifyingFirmware,
                "Upload completed, but the required WVU protocol identity was not verified.",
                verification.explanation,
            ));
        }
        self.session
            .prepare_for_new_recording()
            .map_err(session_failure)?;
        self.finish_success(job_id, FirmwareCompatibility::WvuProtocolCompatible)
    }

    fn selected_board(
        &self,
        port: &str,
        stage: FirmwareJobStage,
    ) -> Result<BoardInfo, FirmwareFailure> {
        if port.trim().is_empty() || port.contains('\0') {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::BoardNotFound,
                stage,
                "Select a detected Arduino UNO R4 WiFi port.",
                "empty or NUL-containing port",
            ));
        }
        let cli = required_cli(stage)?;
        let matches: Vec<_> = cli
            .boards()
            .map_err(|error| cli_failure(stage, FirmwareErrorCategory::BoardNotFound, error))?
            .into_iter()
            .filter(|board| board.port.eq_ignore_ascii_case(port))
            .collect();
        match matches.as_slice() {
            [board] => Ok(board.clone()),
            [] => Err(FirmwareFailure::new(
                FirmwareErrorCategory::BoardNotFound,
                stage,
                "The selected COM port is not a currently detected UNO R4 WiFi.",
                port,
            )),
            _ => Err(FirmwareFailure::new(
                FirmwareErrorCategory::AmbiguousBoard,
                stage,
                "Multiple supported board identities matched the selected port.",
                port,
            )),
        }
    }

    fn ensure_upload_is_safe(&self, stage: FirmwareJobStage) -> Result<(), FirmwareFailure> {
        if self.session.is_recording().map_err(session_failure)? {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::PortBusy,
                stage,
                "Stop and finalize acquisition before uploading firmware.",
                "production serial session is active",
            ));
        }
        Ok(())
    }

    fn new_job(
        &self,
        kind: FirmwareJobKind,
        source_hash: Option<String>,
        original_port: Option<String>,
        board_serial: Option<String>,
    ) -> FirmwareJobStatus {
        FirmwareJobStatus {
            id: self.next_job_id.fetch_add(1, Ordering::Relaxed),
            kind,
            stage: FirmwareJobStage::Preparing,
            active: true,
            source_hash,
            original_port,
            bootloader_port: None,
            final_port: None,
            board_serial,
            started_utc: Utc::now(),
            completed_utc: None,
            message: "Preparing firmware operation.".into(),
            compile_usage: None,
            diagnostics: Vec::new(),
            compile_log: None,
            upload_log: None,
            verification: None,
            failure: None,
            log_path: None,
        }
    }

    fn set_stage(
        &self,
        job_id: u64,
        stage: FirmwareJobStage,
        message: &str,
    ) -> Result<(), FirmwareFailure> {
        self.update_job(job_id, |job| {
            job.stage = stage;
            job.message = message.into();
        })
    }

    fn set_message(&self, message: &str) -> Result<(), FirmwareFailure> {
        let mut runtime = self.lock_state()?;
        if let Some(job) = runtime.active_job.as_mut() {
            job.message = message.into();
        }
        Ok(())
    }

    fn update_job<F>(&self, job_id: u64, update: F) -> Result<(), FirmwareFailure>
    where
        F: FnOnce(&mut FirmwareJobStatus),
    {
        let mut runtime = self.lock_state()?;
        let job = runtime
            .active_job
            .as_mut()
            .ok_or_else(|| internal_failure("firmware job was not active"))?;
        if job.id != job_id {
            return Err(internal_failure("firmware job identity changed"));
        }
        update(job);
        Ok(())
    }

    fn finish_success(
        &self,
        job_id: u64,
        compatibility: FirmwareCompatibility,
    ) -> Result<(), FirmwareFailure> {
        // Keep the active snapshot published while log I/O runs so polling
        // never observes a gap between the active and completed restore state.
        let mut job = self.active_job_snapshot(job_id)?;
        job.stage = FirmwareJobStage::Complete;
        job.active = false;
        job.completed_utc = Some(Utc::now());
        job.message = "Firmware operation completed.".into();
        job.log_path = write_workflow_log(&job).ok();
        let mut runtime = self.lock_state()?;
        verify_active_job(&runtime, job_id)?;
        runtime.active_job = None;
        runtime.compatibility = compatibility;
        runtime.last_restore = Some(job);
        Ok(())
    }

    fn finish_failure(&self, job_id: u64, failure: FirmwareFailure) -> Result<(), FirmwareFailure> {
        // See `finish_success`: no terminal status gap is allowed while the
        // workflow writes its diagnostic log.
        let mut job = self.active_job_snapshot(job_id)?;
        job.stage = if failure.category == FirmwareErrorCategory::Canceled {
            FirmwareJobStage::Canceled
        } else {
            FirmwareJobStage::Failed
        };
        job.active = false;
        job.completed_utc = Some(Utc::now());
        job.message = failure.explanation.clone();
        job.failure = Some(failure.clone());
        job.log_path = write_workflow_log(&job).ok();
        let mut runtime = self.lock_state()?;
        verify_active_job(&runtime, job_id)?;
        runtime.active_job = None;
        runtime.last_failure = Some(failure);
        runtime.compatibility = FirmwareCompatibility::VerificationFailed;
        runtime.last_restore = Some(job);
        Ok(())
    }

    fn active_job_snapshot(&self, job_id: u64) -> Result<FirmwareJobStatus, FirmwareFailure> {
        let runtime = self.lock_state()?;
        let job = runtime
            .active_job
            .as_ref()
            .ok_or_else(|| internal_failure("firmware job was not active"))?;
        if job.id != job_id {
            return Err(internal_failure("firmware job identity changed"));
        }
        Ok(job.clone())
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, WorkflowRuntime>, FirmwareFailure> {
        self.state
            .lock()
            .map_err(|_| internal_failure("firmware state lock poisoned"))
    }
}

fn verify_active_job(runtime: &WorkflowRuntime, job_id: u64) -> Result<(), FirmwareFailure> {
    match runtime.active_job.as_ref() {
        Some(job) if job.id == job_id => Ok(()),
        Some(_) => Err(internal_failure("firmware job identity changed")),
        None => Err(internal_failure("firmware job was not active")),
    }
}

fn required_cli(stage: FirmwareJobStage) -> Result<ArduinoCli, FirmwareFailure> {
    ArduinoCli::discover(None).map_err(|error| match error {
        crate::arduino_cli::CliError::NotFound
        | crate::arduino_cli::CliError::RuntimeUnavailable => FirmwareFailure::new(
            FirmwareErrorCategory::ArduinoCliMissing,
            stage,
            "Arduino tools are unavailable. Editing and saving remain available.",
            error.to_string(),
        ),
        _ => cli_failure(stage, FirmwareErrorCategory::InternalError, error),
    })
}

fn require_core(cli: &ArduinoCli, stage: FirmwareJobStage) -> Result<(), FirmwareFailure> {
    cli.uno_r4_core_version().map(|_| ()).map_err(|error| {
        FirmwareFailure::new(
            FirmwareErrorCategory::CoreMissing,
            stage,
            "The included Arduino UNO R4 tools are incomplete. Reinstall the application or contact your instructor.",
            error.to_string(),
        )
    })
}

fn wait_for_native_reference_port<F>(
    cli: &ArduinoCli,
    cancel: &AtomicBool,
    mut observe: F,
) -> Result<BoardInfo, FirmwareFailure>
where
    F: FnMut(&BoardInfo) -> Result<(), FirmwareFailure>,
{
    let started = Instant::now();
    while started.elapsed() < APPLICATION_PORT_TIMEOUT {
        if cancel.load(Ordering::Acquire) {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::Canceled,
                FirmwareJobStage::Canceled,
                "Upload canceled while waiting for the application port.",
                "cancel requested",
            ));
        }
        if let Some(board) =
            select_returned_native_reference_port(cli.boards().unwrap_or_default())?
        {
            observe(&board)?;
            return Ok(board);
        }
        thread::sleep(APPLICATION_PORT_POLL);
    }
    Err(FirmwareFailure::new(FirmwareErrorCategory::ApplicationPortNotFound, FirmwareJobStage::WaitingForApplicationPort, "Arduino CLI reported upload success, but the controlled native-USB application port did not return in time.", "expected one Arduino UNO R4 WiFi port with PID 0x006D"))
}

/// The controlled reference firmware deliberately uses RA4M1 native USB CDC.
/// Its USB serial number differs from the ESP32 bridge's serial number, so an
/// upload transition cannot use the legacy bridge serial as an identity key.
/// Requiring exactly one native-USB UNO port after an already-completed upload
/// is both deterministic and safer than silently selecting an arbitrary board.
fn select_returned_native_reference_port(
    boards: Vec<BoardInfo>,
) -> Result<Option<BoardInfo>, FirmwareFailure> {
    let candidates: Vec<_> = boards
        .into_iter()
        .filter(|candidate| candidate.fqbn == UNO_R4_WIFI_FQBN && uses_native_usb(candidate))
        .collect();
    match candidates.as_slice() {
        [] => Ok(None),
        [board] => Ok(Some(board.clone())),
        _ => Err(FirmwareFailure::new(
            FirmwareErrorCategory::AmbiguousBoard,
            FirmwareJobStage::WaitingForApplicationPort,
            "Multiple native-USB UNO R4 WiFi ports are present after the upload.",
            "ambiguous native USB application candidates",
        )),
    }
}

fn uses_native_usb(board: &BoardInfo) -> bool {
    board.usb_pid == Some(NATIVE_USB_PID)
}

/// A native-USB sketch has switched the physical USB mux away from the ESP32
/// bridge, and this pinned core cannot perform a 1200-bps touch through that
/// native CDC interface. The user-approved double reset returns the mux to the
/// ordinary bridge. We only enumerate while waiting; no port is opened, reset,
/// or otherwise touched until the normal Arduino CLI upload begins.
fn wait_for_esp32_bridge_port(
    cli: &ArduinoCli,
    cancel: &AtomicBool,
) -> Result<BoardInfo, FirmwareFailure> {
    let started = Instant::now();
    while started.elapsed() < NATIVE_USB_MANUAL_RESET_TIMEOUT {
        if cancel.load(Ordering::Acquire) {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::Canceled,
                FirmwareJobStage::Canceled,
                "Firmware restore canceled while waiting for the board reset.",
                "cancel requested",
            ));
        }
        let candidates: Vec<_> = cli
            .boards()
            .unwrap_or_default()
            .into_iter()
            .filter(|candidate| {
                candidate.fqbn == UNO_R4_WIFI_FQBN && candidate.usb_pid == Some(ESP32_BRIDGE_PID)
            })
            .collect();
        match candidates.as_slice() {
            [board] => return Ok(board.clone()),
            [] => thread::sleep(APPLICATION_PORT_POLL),
            _ => {
                return Err(FirmwareFailure::new(
                    FirmwareErrorCategory::AmbiguousBoard,
                    FirmwareJobStage::WaitingForBootloader,
                    "More than one ESP32 bridge upload port is available.",
                    "disconnect extra UNO R4 WiFi boards, then try Restore WVU Firmware again",
                ));
            }
        }
    }
    Err(FirmwareFailure::new(
        FirmwareErrorCategory::ManualResetRequired,
        FirmwareJobStage::WaitingForBootloader,
        "The board did not switch to its upload interface in time. While Restore WVU Firmware is open, press the board Reset button twice, then try again.",
        "native USB reset window expired while waiting for one PID 0x1002 UNO R4 WiFi port",
    ))
}

fn verification_from_handshake(
    declared_kind: FirmwareVerificationKind,
    diagnostics: &crate::session::ConnectionDiagnostics,
    handshake_succeeded: bool,
) -> FirmwareVerification {
    let expected = diagnostics.firmware_build == Some(REFERENCE_FIRMWARE_BUILD)
        && diagnostics.firmware_board_id == Some(REFERENCE_DEVICE_ID)
        && diagnostics.hello_received
        && diagnostics.capabilities_received
        && diagnostics.pong_received
        && diagnostics.crc_failures == 0;
    let compatible = handshake_succeeded && expected;
    FirmwareVerification {
        declared_kind,
        compatible,
        protocol_version: diagnostics.protocol_version.clone(),
        identity: compatible.then_some(FirmwareIdentity {
            protocol_version: format!("{PROTOCOL_MAJOR}.{PROTOCOL_MINOR}"),
            firmware_build: REFERENCE_FIRMWARE_BUILD,
            device_id: REFERENCE_DEVICE_ID,
        }),
        bytes_received: Some(diagnostics.bytes_received),
        valid_frames: Some(diagnostics.valid_frames),
        crc_failures: Some(diagnostics.crc_failures),
        explanation: if compatible {
            "WVU protocol HELLO, CAPABILITIES, PONG, CRC, and controlled firmware identity verified.".into()
        } else {
            "The returning sketch did not prove the required controlled WVU protocol identity."
                .into()
        },
    }
}

fn job_directory(job_id: u64) -> PathBuf {
    default_log_dir().join("jobs").join(format!("job_{job_id}"))
}

fn default_log_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("firmware_workspace_data"))
        .join("WVU Bioinstrumentation Studio")
        .join("firmware")
        .join("logs")
}

#[derive(Serialize)]
struct FirmwareWorkflowLog<'a> {
    app_name: &'static str,
    app_version: &'static str,
    git_revision: Option<&'static str>,
    fqbn: &'static str,
    arduino_cli_version: Option<String>,
    uno_r4_core_version: Option<String>,
    job: &'a FirmwareJobStatus,
}

fn write_workflow_log(job: &FirmwareJobStatus) -> Result<String, std::io::Error> {
    let directory = default_log_dir();
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!(
        "firmware_job_{}_{}.json",
        job.id,
        job.started_utc.format("%Y%m%d_%H%M%S")
    ));
    let cli = ArduinoCli::discover(None).ok();
    let arduino_cli_version = cli
        .as_ref()
        .and_then(|item| item.version().ok().map(|log| log.stdout.trim().to_owned()));
    let uno_r4_core_version = cli
        .as_ref()
        .and_then(|item| item.uno_r4_core_version().ok());
    let log = FirmwareWorkflowLog {
        app_name: "WVU Bioinstrumentation Studio",
        app_version: env!("CARGO_PKG_VERSION"),
        git_revision: option_env!("GIT_COMMIT"),
        fqbn: UNO_R4_WIFI_FQBN,
        arduino_cli_version,
        uno_r4_core_version,
        job,
    };
    fs::write(
        &path,
        serde_json::to_vec_pretty(&log).map_err(std::io::Error::other)?,
    )?;
    // Firmware compile/upload output can be large. Retain recent diagnostics
    // for support without indefinitely consuming a student's AppData folder.
    let _ = prune_firmware_job_logs(&directory);
    Ok(path.display().to_string())
}

fn prune_firmware_job_logs(directory: &Path) -> Result<(), std::io::Error> {
    let mut logs = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
                && entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("firmware_job_")
        })
        .collect::<Vec<_>>();
    logs.sort_by_key(|entry| {
        entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    if logs.len() > RETAINED_FIRMWARE_JOB_LOGS {
        let excess = logs.len() - RETAINED_FIRMWARE_JOB_LOGS;
        for entry in logs.into_iter().take(excess) {
            fs::remove_file(entry.path())?;
        }
    }

    let jobs = directory.join("jobs");
    if !jobs.is_dir() {
        return Ok(());
    }
    let mut workspaces = fs::read_dir(&jobs)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .collect::<Vec<_>>();
    workspaces.sort_by_key(|entry| {
        entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    if workspaces.len() > RETAINED_FIRMWARE_JOB_LOGS {
        let excess = workspaces.len() - RETAINED_FIRMWARE_JOB_LOGS;
        for entry in workspaces.into_iter().take(excess) {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

fn combined_output(log: &CommandLog) -> String {
    format!(
        "command: {}\nstdout:\n{}\nstderr:\n{}",
        log.command.join(" "),
        log.stdout,
        log.stderr
    )
}

fn failure_copy(category: FirmwareErrorCategory) -> (&'static str, &'static str) {
    match category {
        FirmwareErrorCategory::ArduinoCliMissing => ("Arduino tools unavailable", "Restart the application. If the problem continues, reinstall WVU Bioinstrumentation Studio or contact your instructor."),
        FirmwareErrorCategory::CoreMissing => ("Arduino tools incomplete", "Reinstall WVU Bioinstrumentation Studio or contact your instructor."),
        FirmwareErrorCategory::ConfirmationRequired => ("Confirmation required", "Confirm that you want to restore the WVU firmware."),
        FirmwareErrorCategory::CompileFailed => ("Firmware preparation failed", "Review the advanced details and contact your instructor if the problem continues."),
        FirmwareErrorCategory::PortBusy => ("Serial port is busy", "Stop/finalize acquisition and close other serial programs, then retry."),
        FirmwareErrorCategory::BoardNotFound => ("Board not found", "Refresh boards and select the connected UNO R4 WiFi."),
        FirmwareErrorCategory::AmbiguousBoard => ("Board identity is ambiguous", "Disconnect other matching boards and retry with the board serial shown."),
        FirmwareErrorCategory::UploadFailed => ("Upload failed", "Review Arduino CLI output and ensure no other program owns the selected COM port."),
        FirmwareErrorCategory::ApplicationPortNotFound => ("Application port did not return", "Refresh boards after upload. Do not assume the old COM number."),
        FirmwareErrorCategory::ManualResetRequired => ("Board reset needed", "While Restore WVU Firmware is open, press the board Reset button twice to switch safely to the upload interface."),
        FirmwareErrorCategory::ProtocolVerificationFailed => ("Firmware verification failed", "Use Restore WVU Firmware before using Acquisition."),
        FirmwareErrorCategory::WrongFirmwareIdentity => ("Firmware update required", "Use Restore WVU Firmware before using Acquisition."),
        FirmwareErrorCategory::Canceled => ("Operation canceled", "Reconnect the board if needed, then restore the WVU firmware again."),
        FirmwareErrorCategory::InternalError => ("Firmware workflow error", "Copy diagnostics and contact the instructor/developer."),
    }
}

fn session_failure(error: crate::session::SessionError) -> FirmwareFailure {
    FirmwareFailure::new(
        FirmwareErrorCategory::PortBusy,
        FirmwareJobStage::ClosingSerialSession,
        error.to_string(),
        error.to_string(),
    )
}

fn io_failure(stage: FirmwareJobStage, error: std::io::Error) -> FirmwareFailure {
    FirmwareFailure::new(
        FirmwareErrorCategory::InternalError,
        stage,
        "Could not create or read firmware workflow files.",
        error.to_string(),
    )
}

fn cli_failure(
    stage: FirmwareJobStage,
    category: FirmwareErrorCategory,
    error: crate::arduino_cli::CliError,
) -> FirmwareFailure {
    let details = error
        .command_log()
        .map(combined_output)
        .unwrap_or_else(|| error.to_string());
    FirmwareFailure::new(category, stage, error.to_string(), details)
}

fn internal_failure(details: impl Into<String>) -> FirmwareFailure {
    FirmwareFailure::new(
        FirmwareErrorCategory::InternalError,
        FirmwareJobStage::Failed,
        "Firmware workflow encountered an internal error.",
        details,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn board(port: &str, serial: Option<&str>) -> BoardInfo {
        BoardInfo {
            port: port.into(),
            name: "Arduino UNO R4 WiFi".into(),
            fqbn: UNO_R4_WIFI_FQBN.into(),
            serial_number: serial.map(str::to_owned),
            usb_vid: Some(0x2341),
            usb_pid: Some(0x1002),
        }
    }

    #[test]
    fn firmware_job_retention_keeps_the_log_directory_bounded() {
        let directory = tempdir().unwrap_or_else(|error| panic!("{error}"));
        for index in 0..(RETAINED_FIRMWARE_JOB_LOGS + 3) {
            fs::write(
                directory
                    .path()
                    .join(format!("firmware_job_{index}_test.json")),
                b"{}",
            )
            .unwrap_or_else(|error| panic!("{error}"));
        }
        let jobs = directory.path().join("jobs");
        fs::create_dir_all(&jobs).unwrap_or_else(|error| panic!("{error}"));
        for index in 0..(RETAINED_FIRMWARE_JOB_LOGS + 2) {
            fs::create_dir_all(jobs.join(format!("job_{index}")))
                .unwrap_or_else(|error| panic!("{error}"));
        }
        prune_firmware_job_logs(directory.path()).unwrap_or_else(|error| panic!("{error}"));
        assert!(
            fs::read_dir(directory.path())
                .unwrap_or_else(|error| panic!("{error}"))
                .filter_map(Result::ok)
                .filter(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("firmware_job_"))
                .count()
                <= RETAINED_FIRMWARE_JOB_LOGS
        );
        assert!(
            fs::read_dir(jobs)
                .unwrap_or_else(|error| panic!("{error}"))
                .filter_map(Result::ok)
                .count()
                <= RETAINED_FIRMWARE_JOB_LOGS
        );
    }

    #[test]
    fn controlled_identity_requires_all_protocol_evidence() {
        let diagnostics = crate::session::ConnectionDiagnostics {
            selected_port: "COM12".into(),
            board: "UNO".into(),
            fqbn: UNO_R4_WIFI_FQBN.into(),
            port_opened: true,
            bytes_received: 64,
            valid_frames: 3,
            crc_failures: 0,
            skipped_noise_bytes: 0,
            hello_received: true,
            capabilities_received: true,
            pong_received: true,
            protocol_version: Some(format!("{PROTOCOL_MAJOR}.{PROTOCOL_MINOR}")),
            firmware_capabilities: None,
            firmware_build: Some(REFERENCE_FIRMWARE_BUILD),
            firmware_board_id: Some(REFERENCE_DEVICE_ID),
            raw_byte_classification: "validated WVU binary frames".into(),
            ping_attempts: 1,
            handshake_elapsed_ms: 1,
            reset_attempted: false,
            original_port: Some("COM12".into()),
            final_port: None,
            disappearance_observed: false,
            reappearance_observed: false,
            bootloader_observed: false,
            failure_category: None,
            recommended_action: "ok".into(),
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
        };
        assert!(
            verification_from_handshake(
                FirmwareVerificationKind::WvuProtocolReference,
                &diagnostics,
                true
            )
            .compatible
        );
        let mut wrong = diagnostics;
        wrong.firmware_build = Some(9);
        assert!(
            !verification_from_handshake(
                FirmwareVerificationKind::WvuProtocolReference,
                &wrong,
                true
            )
            .compatible
        );
    }

    #[test]
    fn native_reference_port_requires_the_ra4m1_native_usb_pid() {
        let mut native = board("COM19", Some("RA4M1-SERIAL"));
        native.usb_pid = Some(NATIVE_USB_PID);
        let bridge = board("COM3", Some("ESP32-BRIDGE"));
        let selected = select_returned_native_reference_port(vec![bridge, native])
            .unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(selected.map(|item| item.port), Some("COM19".into()));
    }

    #[test]
    fn native_reference_port_does_not_select_the_esp32_bridge() {
        let bridge = board("COM12", Some("ESP32-BRIDGE"));
        assert!(select_returned_native_reference_port(vec![bridge])
            .unwrap_or_else(|error| panic!("{error:?}"))
            .is_none());
        let mut native = board("COM12", None);
        native.usb_pid = Some(NATIVE_USB_PID);
        assert_eq!(
            select_returned_native_reference_port(vec![native])
                .unwrap_or_else(|error| panic!("{error:?}"))
                .map(|item| item.port),
            Some("COM12".into())
        );
    }

    #[test]
    fn multiple_native_reference_ports_are_an_actionable_error() {
        let mut first = board("COM13", Some("RA4M1-A"));
        first.usb_pid = Some(NATIVE_USB_PID);
        let mut second = board("COM14", Some("RA4M1-B"));
        second.usb_pid = Some(NATIVE_USB_PID);
        let error = select_returned_native_reference_port(vec![first, second])
            .err()
            .unwrap_or_else(|| panic!("expected an ambiguous-board failure"));
        assert_eq!(error.category, FirmwareErrorCategory::AmbiguousBoard);
    }

    #[test]
    fn terminal_status_snapshot_does_not_create_a_polling_gap() {
        let workflow = FirmwareWorkflow::new(SessionController::default());
        let job = workflow.new_job(FirmwareJobKind::RestoreReference, None, None, None);
        {
            let mut runtime = workflow
                .lock_state()
                .unwrap_or_else(|error| panic!("{error:?}"));
            runtime.active_job = Some(job.clone());
        }
        let snapshot = workflow
            .active_job_snapshot(job.id)
            .unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(snapshot.id, job.id);
        assert!(workflow
            .status()
            .unwrap_or_else(|error| panic!("{error:?}"))
            .job
            .is_some());
    }
}
