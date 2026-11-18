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
    serial_lease::{SerialLease, SerialOwner},
    session::{ConnectionFailureCategory, FaultOrigin, SessionController, SessionError},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, PoisonError,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const APPLICATION_PORT_TIMEOUT: Duration = Duration::from_secs(15);
const APPLICATION_PORT_POLL: Duration = Duration::from_millis(300);
const ARDUINO_USB_VID: u16 = 0x2341;
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

impl FirmwareJobStage {
    /// From the upload onward the board is being written (or has just been
    /// written and is being checked), so a restore can no longer be canceled.
    pub fn is_past_cancel_point(self) -> bool {
        matches!(
            self,
            Self::Uploading | Self::WaitingForApplicationPort | Self::VerifyingFirmware
        )
    }
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
    /// Cancel was refused because the board is being written.
    UploadCannotBeCanceled,
    /// An Arduino CLI command missed its deadline and its tools were stopped.
    ToolTimedOut,
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

    fn with_recommended_action(mut self, recommended_action: &str) -> Self {
        self.recommended_action = recommended_action.into();
        self
    }

    /// A command's worker task could not run or did not return normally.
    pub fn task_failure(details: impl Into<String>) -> Self {
        internal_failure(details)
    }
}

impl std::fmt::Display for FirmwareFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}: {} ({})",
            self.title, self.explanation, self.technical_details
        )
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
    /// Set only while a native-USB restore waits for the student's double
    /// press of RESET: when that wait gives up. The page shows the "press
    /// RESET twice" card and its countdown from it.
    pub manual_reset_deadline_utc: Option<DateTime<Utc>>,
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
    /// The port on which compatible WVU firmware was last proven (by Verify
    /// Firmware or a completed restore). Hardware Start is allowed only there.
    pub verified_port: Option<String>,
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
    /// Where compatibility was last proven; cleared by a failed verify, a job
    /// start, or a job failure.
    verified_port: Option<String>,
}

impl WorkflowRuntime {
    fn new() -> Self {
        Self {
            compatibility: FirmwareCompatibility::Unknown,
            active_job: None,
            last_restore: None,
            last_failure: None,
            verified_port: None,
        }
    }

    /// Readiness is bound to the port where compatible firmware was proven.
    /// A failed restore on another port does not matter once Verify succeeds.
    fn acquisition_allowed(&self, port: &str) -> bool {
        self.compatibility == FirmwareCompatibility::WvuProtocolCompatible
            && self
                .verified_port
                .as_deref()
                .is_some_and(|verified| verified.eq_ignore_ascii_case(port))
    }

    fn begin_job(&mut self, job: FirmwareJobStatus) {
        self.active_job = Some(job);
        self.last_failure = None;
        self.compatibility = FirmwareCompatibility::UploadInProgress;
        self.verified_port = None;
    }

    fn finish_job_success(&mut self, job: FirmwareJobStatus, compatibility: FirmwareCompatibility) {
        self.active_job = None;
        self.compatibility = compatibility;
        // The restore verified the board where it returned after the upload.
        self.verified_port = (compatibility == FirmwareCompatibility::WvuProtocolCompatible)
            .then(|| job.final_port.clone())
            .flatten();
        self.last_restore = Some(job);
    }

    fn finish_job_failure(&mut self, job: FirmwareJobStatus, failure: FirmwareFailure) {
        self.active_job = None;
        self.last_failure = Some(failure);
        self.compatibility = FirmwareCompatibility::VerificationFailed;
        self.verified_port = None;
        self.last_restore = Some(job);
    }

    /// Applies a completed Verify Firmware handshake on `port`. A firmware
    /// job owns the board's firmware status, so nothing changes while one is
    /// active (the serial lease already prevents that overlap).
    fn record_verification(
        &mut self,
        port: &str,
        compatible: bool,
        failure: Option<FirmwareFailure>,
    ) {
        if self.active_job.is_some() {
            return;
        }
        if compatible {
            self.compatibility = FirmwareCompatibility::WvuProtocolCompatible;
            self.verified_port = Some(port.to_owned());
            self.last_failure = None;
        } else {
            self.compatibility = FirmwareCompatibility::VerificationFailed;
            self.verified_port = None;
            self.last_failure = failure;
        }
    }
}

impl FirmwareWorkflow {
    pub fn new(session: SessionController) -> Self {
        Self {
            session,
            state: Arc::new(Mutex::new(WorkflowRuntime::new())),
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
            verified_port: runtime.verified_port.clone(),
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
        // Reuse the per-run version cache; probe individually only to explain
        // which part is missing when the cached probe fails.
        let (cli_version, uno_r4_core_version) = match crate::arduino_cli::cached_tool_versions() {
            Ok(versions) => (
                Some(versions.arduino_cli_version),
                Some(versions.uno_r4_core_version),
            ),
            Err(_) => (
                cli.version().ok().map(|log| log.stdout.trim().to_owned()),
                cli.uno_r4_core_version().ok(),
            ),
        };
        // Students only ever get the offline tools included with the app, so
        // the remedy is Repair (re-extract) or a reinstall, never a download.
        let problem = if cli_version.is_none() {
            Some("The Arduino tools did not start correctly. Select Repair Arduino tools. If that does not help, reinstall WVU Bioinstrumentation Studio or contact your instructor.".into())
        } else if uno_r4_core_version.is_none() {
            Some("The Arduino UNO R4 tools are incomplete. Select Repair Arduino tools. If that does not help, reinstall WVU Bioinstrumentation Studio or contact your instructor.".into())
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
        // The job owns the port and session from here until its worker ends.
        let lease = self.claim_serial(SerialOwner::FirmwareJob, FirmwareJobStage::Preparing)?;
        let boards = detected_boards(&request.port, FirmwareJobStage::Preparing)?;
        let board = board_on_port(&boards, &request.port, FirmwareJobStage::Preparing)?;
        // The native-USB path finds the board again only as "the one upload
        // port" and "the one native port", so a second UNO could be written
        // or verified instead of the confirmed one.
        ensure_single_uno_for_restore(&boards, &uno_usb_serial_ports())?;
        self.ensure_upload_is_safe(FirmwareJobStage::Preparing)?;
        let job = self.new_job(
            FirmwareJobKind::RestoreReference,
            None,
            Some(board.port.clone()),
            board.serial_number.clone(),
        );
        self.start_worker(lease, job.clone(), move |workflow| {
            workflow.restore_reference(job.id, board)
        })?;
        Ok(job)
    }

    /// Cancels a restore before the upload starts. From the upload onward the
    /// request is refused: stopping a flash part-way would only leave the
    /// board without working firmware.
    ///
    /// The stage check and the cancel flag are set under the state lock, and
    /// the worker enters the upload under the same lock
    /// (`enter_upload_stage`), so a cancel is either applied before the upload
    /// begins or refused.
    pub fn cancel_active_job(&self) -> Result<FirmwareWorkflowStatus, FirmwareFailure> {
        {
            let mut runtime = self.lock_state()?;
            let Some(job) = runtime.active_job.as_mut() else {
                drop(runtime);
                return self.status();
            };
            if job.stage.is_past_cancel_point() {
                return Err(FirmwareFailure::new(
                    FirmwareErrorCategory::UploadCannotBeCanceled,
                    job.stage,
                    "The WVU firmware is being written to the board, so the restore cannot be canceled now.",
                    format!("cancel refused at stage {:?}", job.stage),
                ));
            }
            self.cancel.store(true, Ordering::Release);
            job.message = "Cancel requested; stopping the Arduino tools.".into();
        }
        self.status()
    }

    /// True while a firmware job runs. The window must not close then.
    ///
    /// Both signals must agree: the job is published as active and its
    /// worker still holds the FirmwareJob lease. The lease is released when
    /// the worker ends, even by a panic, so a job that somehow could not clear
    /// its own status never keeps the window open forever.
    pub fn has_active_job(&self) -> bool {
        let published = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .active_job
            .is_some();
        published && self.session.serial_ownership().current() == SerialOwner::FirmwareJob
    }

    /// Read-only Verify Firmware: a handshake that never uploads.
    ///
    /// It refuses while another operation owns the board (and so never
    /// changes compatibility during a restore). A port that cannot be opened
    /// is reported as busy (or as no longer connected) and leaves the previous
    /// compatibility unchanged, because the board was not examined.
    pub fn verify_existing_reference(
        &self,
        port: String,
    ) -> Result<FirmwareVerification, FirmwareFailure> {
        let _lease = self.claim_serial(SerialOwner::Verify, FirmwareJobStage::VerifyingFirmware)?;
        let boards = detected_boards(&port, FirmwareJobStage::VerifyingFirmware)?;
        let board = board_on_port(&boards, &port, FirmwareJobStage::VerifyingFirmware)?;
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
            .retry_handshake(&board.port, FaultOrigin::Verification)
            .map_err(session_failure)?;
        if !result.diagnostics.port_opened {
            let detail = self
                .session
                .status()
                .ok()
                .and_then(|status| status.last_error)
                .unwrap_or_else(|| "serial port open failed".into());
            let failure = unopened_port_failure(&board.port, &result.diagnostics, detail);
            self.lock_state()?.last_failure = Some(failure.clone());
            return Err(failure);
        }
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
        let failure = (!verification.compatible).then(|| {
            FirmwareFailure::new(
                FirmwareErrorCategory::ProtocolVerificationFailed,
                FirmwareJobStage::VerifyingFirmware,
                verification.explanation.clone(),
                format!("diagnostics: {:?}", result.diagnostics.failure_category),
            )
        });
        self.lock_state()?
            .record_verification(&board.port, verification.compatible, failure);
        Ok(verification)
    }

    pub fn is_acquisition_allowed(&self, port: &str) -> Result<bool, FirmwareFailure> {
        Ok(self.lock_state()?.acquisition_allowed(port))
    }

    /// Claims the shared serial lease, or explains which operation holds it.
    fn claim_serial(
        &self,
        owner: SerialOwner,
        stage: FirmwareJobStage,
    ) -> Result<SerialLease, FirmwareFailure> {
        self.session
            .claim_serial(owner)
            .map_err(|error| busy_failure(stage, error))
    }

    /// Runs `work` on the firmware worker, which owns `lease` until it has
    /// published the job result (or unwound from a panic).
    fn start_worker<F>(
        &self,
        lease: SerialLease,
        job: FirmwareJobStatus,
        work: F,
    ) -> Result<(), FirmwareFailure>
    where
        F: FnOnce(FirmwareWorkflow) -> Result<(), FirmwareFailure> + Send + 'static,
    {
        {
            let mut worker = self
                .worker
                .lock()
                .map_err(|_| internal_failure("firmware worker lock poisoned"))?;
            // Every job worker holds the lease until it returns, and the
            // caller holds it now, so a previous worker is already exiting.
            if let Some(previous) = worker.take() {
                let _ = previous.join();
            }
        }
        self.cancel.store(false, Ordering::Release);
        self.lock_state()?.begin_job(job.clone());
        let controller = self.clone();
        let handle = thread::spawn(move || {
            let _lease = lease;
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(controller.clone())))
                    .unwrap_or_else(|_| {
                        Err(internal_failure("firmware worker stopped unexpectedly"))
                    });
            if let Err(failure) = outcome {
                let _ = controller.finish_failure(job.id, failure);
            }
            // The job must never stay published as active once its worker
            // ends, or Verify would be ignored and Cancel would keep showing.
            controller.abandon_job_if_active(job.id);
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
            let deadline = chrono::Duration::from_std(NATIVE_USB_MANUAL_RESET_TIMEOUT)
                .ok()
                .map(|timeout| Utc::now() + timeout);
            self.update_job(job_id, |job| {
                job.stage = FirmwareJobStage::WaitingForBootloader;
                job.message = "The WVU firmware is using native USB. Press the board Reset button twice to switch safely to the upload interface.".into();
                job.manual_reset_deadline_utc = deadline;
            })?;
            let waited = wait_for_esp32_bridge_port(&cli, &self.cancel);
            // The double-reset wait is over whether or not the bridge appeared.
            let cleared = self.update_job(job_id, |job| {
                job.manual_reset_deadline_utc = None;
                if let Ok(bridge) = &waited {
                    job.bootloader_port = Some(bridge.port.clone());
                }
            });
            let bridge = waited?;
            cleared?;
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
        // Past this point the restore cannot be canceled; the upload is
        // bounded by its own timeout instead.
        self.enter_upload_stage(job_id)?;
        let log = cli
            .upload_input(&binary_path, &upload_board.port)
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
        // The job's own lease covers this handshake.
        let result = self
            .session
            .retry_handshake(&board.port, FaultOrigin::Verification)
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
            manual_reset_deadline_utc: None,
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

    /// Moves the job to Uploading unless a cancel arrived first. See
    /// `cancel_active_job`: both run under the state lock.
    fn enter_upload_stage(&self, job_id: u64) -> Result<(), FirmwareFailure> {
        let mut runtime = self.lock_state()?;
        if self.cancel.load(Ordering::Acquire) {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::Canceled,
                FirmwareJobStage::Canceled,
                "Firmware restore was canceled before the upload started.",
                "cancel requested before upload",
            ));
        }
        let job = runtime
            .active_job
            .as_mut()
            .ok_or_else(|| internal_failure("firmware job was not active"))?;
        if job.id != job_id {
            return Err(internal_failure("firmware job identity changed"));
        }
        job.stage = FirmwareJobStage::Uploading;
        job.message = "Writing the WVU firmware to the board. Do not unplug the board.".into();
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
        runtime.finish_job_success(job, compatibility);
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
        runtime.finish_job_failure(job, failure);
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

    /// Last resort after a job worker ends: if the job is still published as
    /// active (its own finish could not run), record it as failed.
    fn abandon_job_if_active(&self, job_id: u64) {
        let mut runtime = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(mut job) = runtime.active_job.clone().filter(|job| job.id == job_id) else {
            return;
        };
        let failure = internal_failure("firmware worker ended without publishing a result");
        job.stage = FirmwareJobStage::Failed;
        job.active = false;
        job.completed_utc = Some(Utc::now());
        job.message = failure.explanation.clone();
        job.failure = Some(failure.clone());
        runtime.finish_job_failure(job, failure);
    }

    /// The runtime holds plain status values that every update writes whole,
    /// so a panic elsewhere while the lock was held leaves it usable. Status,
    /// Verify, and job completion keep working instead of failing forever.
    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, WorkflowRuntime>, FirmwareFailure> {
        Ok(self.state.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// The UNO R4 WiFi boards Arduino CLI reports now, after checking `port`.
fn detected_boards(port: &str, stage: FirmwareJobStage) -> Result<Vec<BoardInfo>, FirmwareFailure> {
    if port.trim().is_empty() || port.contains('\0') {
        return Err(FirmwareFailure::new(
            FirmwareErrorCategory::BoardNotFound,
            stage,
            "Select a detected Arduino UNO R4 WiFi port.",
            "empty or NUL-containing port",
        ));
    }
    required_cli(stage)?
        .boards()
        .map_err(|error| cli_failure(stage, FirmwareErrorCategory::BoardNotFound, error))
}

fn board_on_port(
    boards: &[BoardInfo],
    port: &str,
    stage: FirmwareJobStage,
) -> Result<BoardInfo, FirmwareFailure> {
    let matches: Vec<_> = boards
        .iter()
        .filter(|board| board.port.eq_ignore_ascii_case(port))
        .collect();
    match matches.as_slice() {
        [board] => Ok((*board).clone()),
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

/// A USB serial port the operating system lists: name, USB VID, and USB PID.
type UsbSerialPort = (String, u16, u16);

/// Operating-system USB serial ports. Read-only enumeration; nothing is opened.
fn uno_usb_serial_ports() -> Vec<UsbSerialPort> {
    serialport::available_ports()
        .map(|ports| {
            ports
                .into_iter()
                .filter_map(|port| match port.port_type {
                    serialport::SerialPortType::UsbPort(usb) => {
                        Some((port.port_name, usb.vid, usb.pid))
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Every port that belongs to an Arduino UNO R4 WiFi in either USB mode:
/// each board Arduino CLI identifies as one (whatever its PID), plus each
/// Arduino USB serial port with the native WVU PID (0x006D) or the ESP32
/// upload-bridge PID (0x1002). One physical board shows exactly one of them.
fn connected_uno_r4_ports(
    cli_boards: &[BoardInfo],
    usb_ports: &[UsbSerialPort],
) -> std::collections::BTreeSet<String> {
    cli_boards
        .iter()
        .filter(|board| board.fqbn == UNO_R4_WIFI_FQBN)
        .map(|board| board.port.to_ascii_uppercase())
        .chain(
            usb_ports
                .iter()
                .filter(|(_, vid, pid)| {
                    *vid == ARDUINO_USB_VID && matches!(*pid, NATIVE_USB_PID | ESP32_BRIDGE_PID)
                })
                .map(|(port, _, _)| port.to_ascii_uppercase()),
        )
        .collect()
}

/// Restore WVU Firmware works on exactly one connected UNO R4 WiFi. During a
/// native-USB restore the board changes PID, COM port, and USB serial number,
/// so it is found again only as "the one upload port" and then "the one
/// native port"; with two boards attached the wrong one could be written or
/// verified. Refuse before anything is touched.
fn ensure_single_uno_for_restore(
    cli_boards: &[BoardInfo],
    usb_ports: &[UsbSerialPort],
) -> Result<(), FirmwareFailure> {
    let ports = connected_uno_r4_ports(cli_boards, usb_ports);
    if ports.len() <= 1 {
        return Ok(());
    }
    let listed = ports.iter().cloned().collect::<Vec<_>>().join(", ");
    Err(FirmwareFailure::new(
        FirmwareErrorCategory::AmbiguousBoard,
        FirmwareJobStage::Preparing,
        format!(
            "{} Arduino UNO R4 WiFi boards are connected ({listed}). Restore WVU Firmware updates one board at a time.",
            ports.len()
        ),
        format!("UNO R4 WiFi ports present before restore: {listed}"),
    )
    .with_recommended_action(
        "Disconnect the other UNO boards so only the board to restore is connected. Then select Refresh Board and Restore WVU Firmware again.",
    ))
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
            "The included Arduino UNO R4 tools are incomplete.",
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
    // Versions come from the per-run cache or the runtime manifest; writing a
    // log (often right after a failure) never starts more tool processes.
    let known = crate::arduino_cli::known_tool_versions();
    let runtime = crate::arduino_runtime::active_runtime();
    let arduino_cli_version = known
        .as_ref()
        .map(|versions| versions.arduino_cli_version.clone())
        .or_else(|| runtime.map(|paths| paths.cli_version.clone()));
    let uno_r4_core_version = known
        .map(|versions| versions.uno_r4_core_version)
        .or_else(|| runtime.map(|paths| paths.core_version.clone()));
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
        FirmwareErrorCategory::ArduinoCliMissing => ("Arduino tools unavailable", "Select Repair Arduino tools. If that does not help, restart the application, reinstall WVU Bioinstrumentation Studio, or contact your instructor."),
        FirmwareErrorCategory::CoreMissing => ("Arduino tools incomplete", "Select Repair Arduino tools. If that does not help, reinstall WVU Bioinstrumentation Studio or contact your instructor."),
        FirmwareErrorCategory::ConfirmationRequired => ("Confirmation required", "Confirm that you want to restore the WVU firmware."),
        FirmwareErrorCategory::CompileFailed => ("Firmware preparation failed", "Review the advanced details and contact your instructor if the problem continues."),
        FirmwareErrorCategory::PortBusy => ("Serial port is busy", "Stop any recording, close Arduino IDE, Serial Monitor, or Arduino Cloud Agent, then try again."),
        FirmwareErrorCategory::BoardNotFound => ("Board not found", "Select Refresh Board and choose the connected UNO R4 WiFi."),
        FirmwareErrorCategory::AmbiguousBoard => ("More than one board found", "Unplug the other Arduino boards, then select Refresh Board."),
        FirmwareErrorCategory::UploadFailed => ("Upload failed", "Close Arduino IDE, Serial Monitor, or Arduino Cloud Agent, press the board Reset button twice, then try Restore WVU Firmware again."),
        FirmwareErrorCategory::ApplicationPortNotFound => ("Board did not return after the upload", "Select Refresh Board; the COM number may have changed. Then select Verify Firmware."),
        FirmwareErrorCategory::ManualResetRequired => ("Board reset needed", "While Restore WVU Firmware is open, press the board Reset button twice to switch safely to the upload interface."),
        FirmwareErrorCategory::ProtocolVerificationFailed => ("Firmware verification failed", "Use Restore WVU Firmware before using Acquisition."),
        FirmwareErrorCategory::WrongFirmwareIdentity => ("Firmware update required", "Use Restore WVU Firmware before using Acquisition."),
        FirmwareErrorCategory::Canceled => ("Operation canceled", "Reconnect the board if needed, then restore the WVU firmware again."),
        FirmwareErrorCategory::UploadCannotBeCanceled => ("The board is being written", "Wait for the restore to finish. Do not unplug the board."),
        FirmwareErrorCategory::ToolTimedOut => ("Arduino tools stopped responding", "Unplug the board and plug it back in, select Refresh Board, then try again. If it keeps happening, restart the application."),
        FirmwareErrorCategory::InternalError => ("Firmware workflow error", "Copy diagnostics and contact the instructor/developer."),
    }
}

fn session_failure(error: SessionError) -> FirmwareFailure {
    if matches!(error, SessionError::Busy(_)) {
        return busy_failure(FirmwareJobStage::ClosingSerialSession, error);
    }
    FirmwareFailure::new(
        FirmwareErrorCategory::PortBusy,
        FirmwareJobStage::ClosingSerialSession,
        error.to_string(),
        error.to_string(),
    )
}

/// Another operation in this application owns the board. Waiting, not
/// closing other programs, is the remedy.
fn busy_failure(stage: FirmwareJobStage, error: SessionError) -> FirmwareFailure {
    let message = error.to_string();
    FirmwareFailure::new(
        FirmwareErrorCategory::PortBusy,
        stage,
        message.clone(),
        format!("serial lease refused: {message}"),
    )
    .with_recommended_action("Wait for the current operation to finish, then try again.")
}

/// Verify could not open the port, so the board was not examined. Windows
/// reports a busy port and a removed board alike; the handshake's read-only
/// enumeration (its failure category) tells them apart.
fn unopened_port_failure(
    port: &str,
    diagnostics: &crate::session::ConnectionDiagnostics,
    technical_details: String,
) -> FirmwareFailure {
    if diagnostics.failure_category == Some(ConnectionFailureCategory::DeviceDisconnected) {
        FirmwareFailure::new(
            FirmwareErrorCategory::BoardNotFound,
            FirmwareJobStage::VerifyingFirmware,
            format!("{port} is no longer connected. The board may have been unplugged or moved to another USB port."),
            technical_details,
        )
    } else {
        FirmwareFailure::new(
            FirmwareErrorCategory::PortBusy,
            FirmwareJobStage::VerifyingFirmware,
            format!("{port} could not be opened. Another program is probably using it, so the firmware was not checked."),
            technical_details,
        )
        .with_recommended_action(
            "Close Arduino IDE, Serial Monitor, or Arduino Cloud Agent, then select Verify Firmware again.",
        )
    }
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
    if matches!(error, crate::arduino_cli::CliError::TimedOut { .. }) {
        let failure = FirmwareFailure::new(
            FirmwareErrorCategory::ToolTimedOut,
            stage,
            error.to_string(),
            details,
        );
        // A stopped upload may have left the board part-way written; the
        // double reset puts it back into upload mode for a new restore.
        return if stage == FirmwareJobStage::Uploading {
            failure.with_recommended_action(
                "Unplug the board and plug it back in. Then select Refresh Board, press the board Reset button twice, and select Restore WVU Firmware again.",
            )
        } else {
            failure
        };
    }
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
        let diagnostics = verified_diagnostics();
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

    fn verified_diagnostics() -> crate::session::ConnectionDiagnostics {
        crate::session::ConnectionDiagnostics {
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
        }
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

    fn restore_job(workflow: &FirmwareWorkflow, port: &str) -> FirmwareJobStatus {
        workflow.new_job(
            FirmwareJobKind::RestoreReference,
            None,
            Some(port.into()),
            None,
        )
    }

    fn failure(category: FirmwareErrorCategory) -> FirmwareFailure {
        FirmwareFailure::new(category, FirmwareJobStage::Failed, "test", "test")
    }

    #[test]
    fn a_new_job_reports_no_double_reset_wait() {
        let workflow = FirmwareWorkflow::new(SessionController::default());
        let job = restore_job(&workflow, "COM9");
        assert!(job.manual_reset_deadline_utc.is_none());
        // The page reads the field by this name; it is always present.
        let json = serde_json::to_value(&job).unwrap_or_else(|error| panic!("{error}"));
        assert!(json
            .get("manual_reset_deadline_utc")
            .is_some_and(serde_json::Value::is_null));
    }

    #[test]
    fn a_failed_restore_on_one_port_does_not_block_a_board_verified_on_another() {
        let workflow = FirmwareWorkflow::new(SessionController::default());
        let mut runtime = workflow
            .lock_state()
            .unwrap_or_else(|error| panic!("{error:?}"));
        let job = restore_job(&workflow, "COM9");
        runtime.begin_job(job.clone());
        assert!(!runtime.acquisition_allowed("COM9"));
        runtime.finish_job_failure(job, failure(FirmwareErrorCategory::UploadFailed));
        assert!(!runtime.acquisition_allowed("COM9"));

        runtime.record_verification("COM12", true, None);
        assert!(runtime.acquisition_allowed("COM12"));
        assert!(runtime.acquisition_allowed("com12"));
        // The failed restore stays visible as history without binding Start.
        assert!(runtime.last_restore.is_some());
        assert!(!runtime.acquisition_allowed("COM9"));
    }

    #[test]
    fn start_is_refused_on_a_port_other_than_the_verified_one() {
        let workflow = FirmwareWorkflow::new(SessionController::default());
        let mut runtime = workflow
            .lock_state()
            .unwrap_or_else(|error| panic!("{error:?}"));
        assert!(!runtime.acquisition_allowed("COM12"));
        runtime.record_verification("COM12", true, None);
        assert_eq!(runtime.verified_port.as_deref(), Some("COM12"));
        assert!(!runtime.acquisition_allowed("COM13"));
        assert!(runtime.acquisition_allowed("COM12"));
    }

    #[test]
    fn failed_verification_and_job_start_clear_the_verified_port() {
        let workflow = FirmwareWorkflow::new(SessionController::default());
        let mut runtime = workflow
            .lock_state()
            .unwrap_or_else(|error| panic!("{error:?}"));
        runtime.record_verification("COM12", true, None);
        runtime.record_verification(
            "COM12",
            false,
            Some(failure(FirmwareErrorCategory::ProtocolVerificationFailed)),
        );
        assert_eq!(runtime.verified_port, None);
        assert_eq!(
            runtime.compatibility,
            FirmwareCompatibility::VerificationFailed
        );
        assert!(runtime.last_failure.is_some());
        assert!(!runtime.acquisition_allowed("COM12"));

        runtime.record_verification("COM12", true, None);
        assert!(runtime.last_failure.is_none());
        let job = restore_job(&workflow, "COM12");
        runtime.begin_job(job);
        assert_eq!(runtime.verified_port, None);
        assert!(!runtime.acquisition_allowed("COM12"));
    }

    #[test]
    fn a_successful_restore_proves_the_port_where_the_board_returned() {
        let workflow = FirmwareWorkflow::new(SessionController::default());
        let mut runtime = workflow
            .lock_state()
            .unwrap_or_else(|error| panic!("{error:?}"));
        let mut job = restore_job(&workflow, "COM34");
        runtime.begin_job(job.clone());
        job.final_port = Some("COM35".into());
        runtime.finish_job_success(job, FirmwareCompatibility::WvuProtocolCompatible);
        assert_eq!(runtime.verified_port.as_deref(), Some("COM35"));
        assert!(runtime.acquisition_allowed("COM35"));
        assert!(!runtime.acquisition_allowed("COM34"));
    }

    #[test]
    fn verification_never_changes_compatibility_while_a_job_is_active() {
        let workflow = FirmwareWorkflow::new(SessionController::default());
        let mut runtime = workflow
            .lock_state()
            .unwrap_or_else(|error| panic!("{error:?}"));
        runtime.begin_job(restore_job(&workflow, "COM12"));
        runtime.record_verification("COM12", true, None);
        assert_eq!(
            runtime.compatibility,
            FirmwareCompatibility::UploadInProgress
        );
        assert_eq!(runtime.verified_port, None);
        assert!(!runtime.acquisition_allowed("COM12"));
    }

    #[test]
    fn verify_and_restore_are_refused_while_another_operation_owns_the_board() {
        let session = SessionController::default();
        let workflow = FirmwareWorkflow::new(session.clone());
        {
            let mut runtime = workflow
                .lock_state()
                .unwrap_or_else(|error| panic!("{error:?}"));
            runtime.record_verification("COM12", true, None);
        }
        let _job = session
            .serial_ownership()
            .try_claim(SerialOwner::FirmwareJob)
            .unwrap_or_else(|busy| panic!("{busy}"));
        let refused = workflow
            .verify_existing_reference("COM12".into())
            .err()
            .unwrap_or_else(|| panic!("Verify must wait for the restore"));
        assert_eq!(refused.category, FirmwareErrorCategory::PortBusy);
        assert!(refused.explanation.contains("being restored"));
        let status = workflow
            .status()
            .unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(
            status.compatibility,
            FirmwareCompatibility::WvuProtocolCompatible
        );
        assert_eq!(status.verified_port.as_deref(), Some("COM12"));

        let restore = workflow
            .start_restore_reference(RestoreReferenceRequest {
                port: "COM12".into(),
                confirmation: true,
            })
            .err()
            .unwrap_or_else(|| panic!("a second job must be refused"));
        assert_eq!(restore.category, FirmwareErrorCategory::PortBusy);
        assert!(workflow
            .status()
            .unwrap_or_else(|error| panic!("{error:?}"))
            .job
            .is_none());
    }

    #[test]
    fn an_unopened_port_is_busy_unless_the_board_is_gone() {
        let mut diagnostics = crate::session::ConnectionDiagnostics {
            port_opened: false,
            failure_category: Some(ConnectionFailureCategory::PortBusy),
            ..verified_diagnostics()
        };
        let busy = unopened_port_failure("COM12", &diagnostics, "Access is denied.".into());
        assert_eq!(busy.category, FirmwareErrorCategory::PortBusy);
        assert!(busy.recommended_action.contains("Arduino IDE"));
        assert!(busy.recommended_action.contains("Verify Firmware"));
        assert!(!busy.recommended_action.contains("Restore"));

        diagnostics.failure_category = Some(ConnectionFailureCategory::DeviceDisconnected);
        let gone = unopened_port_failure("COM12", &diagnostics, "not found".into());
        assert_eq!(gone.category, FirmwareErrorCategory::BoardNotFound);
        assert!(gone.recommended_action.contains("Refresh Board"));
    }

    #[test]
    fn firmware_failures_serialize_with_student_facing_fields() {
        let value = serde_json::to_value(failure(FirmwareErrorCategory::PortBusy))
            .unwrap_or_else(|error| panic!("{error}"));
        for field in ["category", "title", "explanation", "recommended_action"] {
            assert!(value.get(field).is_some(), "{field}");
        }
        assert_eq!(value["category"], "port_busy");
    }

    #[test]
    fn cancel_is_refused_once_the_board_is_being_written() {
        let workflow = FirmwareWorkflow::new(SessionController::default());
        // With no job, cancel is a harmless status read.
        assert!(workflow
            .cancel_active_job()
            .unwrap_or_else(|error| panic!("{error:?}"))
            .job
            .is_none());
        let job = restore_job(&workflow, "COM12");
        let _lease = workflow
            .session
            .serial_ownership()
            .try_claim(SerialOwner::FirmwareJob)
            .unwrap_or_else(|busy| panic!("{busy}"));
        workflow
            .lock_state()
            .unwrap_or_else(|error| panic!("{error:?}"))
            .begin_job(job.clone());
        assert!(workflow.has_active_job());

        // Before the upload a cancel is accepted, and the worker then
        // refuses to start writing the board.
        workflow
            .set_stage(job.id, FirmwareJobStage::Compiling, "compiling")
            .unwrap_or_else(|error| panic!("{error:?}"));
        workflow
            .cancel_active_job()
            .unwrap_or_else(|error| panic!("{error:?}"));
        assert!(workflow.cancel.load(Ordering::Acquire));
        let canceled = workflow
            .enter_upload_stage(job.id)
            .err()
            .unwrap_or_else(|| panic!("a canceled restore must not upload"));
        assert_eq!(canceled.category, FirmwareErrorCategory::Canceled);
        assert_eq!(
            workflow
                .status()
                .unwrap_or_else(|error| panic!("{error:?}"))
                .job
                .map(|job| job.stage),
            Some(FirmwareJobStage::Compiling)
        );

        // From the upload onward a cancel is refused and changes nothing.
        workflow.cancel.store(false, Ordering::Release);
        workflow
            .enter_upload_stage(job.id)
            .unwrap_or_else(|error| panic!("{error:?}"));
        for stage in [
            FirmwareJobStage::Uploading,
            FirmwareJobStage::WaitingForApplicationPort,
            FirmwareJobStage::VerifyingFirmware,
        ] {
            workflow
                .set_stage(job.id, stage, "writing")
                .unwrap_or_else(|error| panic!("{error:?}"));
            let refused = workflow
                .cancel_active_job()
                .err()
                .unwrap_or_else(|| panic!("cancel must be refused at {stage:?}"));
            assert_eq!(
                refused.category,
                FirmwareErrorCategory::UploadCannotBeCanceled
            );
            assert_eq!(refused.stage, stage);
            assert!(refused.recommended_action.contains("Do not unplug"));
            assert!(!workflow.cancel.load(Ordering::Acquire));
        }
        assert!(!FirmwareJobStage::WaitingForBootloader.is_past_cancel_point());
        assert!(!FirmwareJobStage::Compiling.is_past_cancel_point());
    }

    fn native_board(port: &str) -> BoardInfo {
        BoardInfo {
            usb_pid: Some(NATIVE_USB_PID),
            ..board(port, None)
        }
    }

    #[test]
    fn restore_is_refused_while_more_than_one_uno_is_connected() {
        let native = native_board("COM34");
        let bridge = board("COM12", Some("ESP32"));
        // One board, seen by both Arduino CLI and Windows, is one board.
        assert!(ensure_single_uno_for_restore(
            std::slice::from_ref(&native),
            &[("com34".into(), ARDUINO_USB_VID, NATIVE_USB_PID)]
        )
        .is_ok());
        assert!(ensure_single_uno_for_restore(std::slice::from_ref(&bridge), &[]).is_ok());

        // A native-USB board and a bridge-mode board: the F-2 case.
        let refused = ensure_single_uno_for_restore(&[native.clone(), bridge.clone()], &[])
            .err()
            .unwrap_or_else(|| panic!("two UNO boards must be refused"));
        assert_eq!(refused.category, FirmwareErrorCategory::AmbiguousBoard);
        assert_eq!(refused.stage, FirmwareJobStage::Preparing);
        assert!(refused.recommended_action.starts_with(
            "Disconnect the other UNO boards so only the board to restore is connected."
        ));
        assert!(refused.explanation.contains("COM12, COM34"));

        // Two native or two bridge boards are refused as well.
        assert!(
            ensure_single_uno_for_restore(&[native.clone(), native_board("COM35")], &[]).is_err()
        );
        assert!(ensure_single_uno_for_restore(
            &[bridge.clone(), board("COM13", Some("ESP32-B"))],
            &[]
        )
        .is_err());
        // A second UNO that only Windows lists (any of the two PIDs) counts.
        for pid in [NATIVE_USB_PID, ESP32_BRIDGE_PID] {
            assert!(ensure_single_uno_for_restore(
                std::slice::from_ref(&native),
                &[("COM40".into(), ARDUINO_USB_VID, pid)]
            )
            .is_err());
        }
        // Other USB serial devices do not.
        assert!(ensure_single_uno_for_restore(
            &[native],
            &[
                ("COM5".into(), 0x0403, 0x6001),
                ("COM6".into(), ARDUINO_USB_VID, 0x0069),
            ]
        )
        .is_ok());
    }

    #[test]
    fn a_tool_timeout_is_reported_with_its_own_category() {
        let timed_out = || crate::arduino_cli::CliError::TimedOut {
            log: Box::new(CommandLog {
                command: vec!["arduino-cli".into(), "upload".into()],
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                duration_ms: 180_000,
                canceled: false,
                timed_out: true,
            }),
            timeout_seconds: 180,
        };
        let upload = cli_failure(
            FirmwareJobStage::Uploading,
            FirmwareErrorCategory::UploadFailed,
            timed_out(),
        );
        assert_eq!(upload.category, FirmwareErrorCategory::ToolTimedOut);
        assert!(upload.explanation.contains("180 seconds"));
        assert!(upload.recommended_action.contains("Reset button twice"));
        let scan = cli_failure(
            FirmwareJobStage::Preparing,
            FirmwareErrorCategory::BoardNotFound,
            timed_out(),
        );
        assert_eq!(scan.category, FirmwareErrorCategory::ToolTimedOut);
        assert!(scan.recommended_action.contains("Refresh Board"));
    }

    #[test]
    fn tool_problems_never_suggest_a_network_command() {
        for category in [
            FirmwareErrorCategory::ArduinoCliMissing,
            FirmwareErrorCategory::CoreMissing,
        ] {
            let (_, action) = failure_copy(category);
            assert!(action.contains("Repair Arduino tools"), "{action}");
            assert!(!action.contains("arduino-cli"), "{action}");
        }
    }

    #[test]
    fn a_job_whose_worker_ended_never_blocks_close_or_verify() {
        let workflow = FirmwareWorkflow::new(SessionController::default());
        let job = restore_job(&workflow, "COM12");
        let lease = workflow
            .session
            .serial_ownership()
            .try_claim(SerialOwner::FirmwareJob)
            .unwrap_or_else(|busy| panic!("{busy}"));
        workflow
            .lock_state()
            .unwrap_or_else(|error| panic!("{error:?}"))
            .begin_job(job.clone());
        assert!(workflow.has_active_job());

        // A panic while the state lock is held poisons it.
        let poisoner = workflow.clone();
        let panicked = thread::spawn(move || {
            let _guard = poisoner.state.lock();
            panic!("simulated failure while the firmware state was locked");
        })
        .join();
        assert!(panicked.is_err());
        assert!(workflow.state.is_poisoned());

        // The worker's lease is gone once it unwinds: closing is allowed even
        // though the job is still published.
        drop(lease);
        assert!(!workflow.has_active_job());

        // The worker's last-resort cleanup still clears the job, and status
        // and Verify keep working on the poisoned lock.
        workflow.abandon_job_if_active(job.id);
        let status = workflow
            .status()
            .unwrap_or_else(|error| panic!("{error:?}"));
        assert!(status.job.is_none());
        assert_eq!(
            status.last_restore.map(|restore| restore.stage),
            Some(FirmwareJobStage::Failed)
        );
        workflow
            .lock_state()
            .unwrap_or_else(|error| panic!("{error:?}"))
            .record_verification("COM12", true, None);
        assert!(workflow
            .is_acquisition_allowed("COM12")
            .unwrap_or_else(|error| panic!("{error:?}")));
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
