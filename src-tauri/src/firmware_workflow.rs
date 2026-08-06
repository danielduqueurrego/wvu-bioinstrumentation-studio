//! Phase 2 compile/upload orchestration.
//!
//! This controller owns no frontend serial handle. It coordinates the existing
//! acquisition `SessionController`, the safe Arduino CLI adapter, and one active
//! firmware job. Arduino CLI performs the UNO upload/reset; this module observes
//! and records only the selected board's application-port return.
use crate::{
    arduino_cli::{
        parse_compile_usage, parse_compiler_diagnostics, ArduinoCli, BoardInfo, CommandLog,
        CompileUsage, CompilerDiagnostic, UNO_R4_WIFI_FQBN,
    },
    firmware_workspace::{
        stable_hash, FirmwareIdentity, FirmwareProject, FirmwareVerificationKind,
        FirmwareWorkspace, WorkspaceError,
    },
    protocol::{PROTOCOL_MAJOR, PROTOCOL_MINOR, REFERENCE_DEVICE_ID, REFERENCE_FIRMWARE_BUILD},
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirmwareCompatibility {
    Unknown,
    WvuProtocolCompatible,
    WvuProtocolIncompatible,
    NonWvuSketch,
    UploadInProgress,
    VerificationFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirmwareJobKind {
    Compile,
    Upload,
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
    ProjectInvalid,
    UnsavedChanges,
    CompileFailed,
    PortBusy,
    UnsupportedBoard,
    BoardNotFound,
    AmbiguousBoard,
    ResetFailed,
    BootloaderNotFound,
    UploadFailed,
    ApplicationPortNotFound,
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
    pub project_folder: Option<String>,
    /// Stable hash of the saved source compiled for this job, when applicable.
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
pub struct CompileProjectRequest {
    pub project_folder: String,
    pub unsaved_changes: bool,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct UploadProjectRequest {
    pub project_folder: String,
    pub port: String,
    pub unsaved_changes: bool,
    pub confirmation: bool,
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
    pub last_compile: Option<FirmwareJobStatus>,
    pub last_upload: Option<FirmwareJobStatus>,
    pub last_failure: Option<FirmwareFailure>,
}

#[derive(Clone)]
pub struct FirmwareWorkflow {
    workspace: FirmwareWorkspace,
    session: SessionController,
    state: Arc<Mutex<WorkflowRuntime>>,
    cancel: Arc<AtomicBool>,
    worker: Arc<Mutex<Option<JoinHandle<()>>>>,
    next_job_id: Arc<AtomicU64>,
}

struct WorkflowRuntime {
    compatibility: FirmwareCompatibility,
    active_job: Option<FirmwareJobStatus>,
    last_compile: Option<FirmwareJobStatus>,
    last_upload: Option<FirmwareJobStatus>,
    last_failure: Option<FirmwareFailure>,
    build_artifact: Option<BuildArtifact>,
}

#[derive(Clone, Debug)]
struct BuildArtifact {
    project_folder: String,
    source_hash: String,
    binary_path: PathBuf,
}

impl FirmwareWorkflow {
    pub fn new(session: SessionController) -> Self {
        Self::with_workspace(FirmwareWorkspace::default(), session)
    }

    pub fn with_workspace(workspace: FirmwareWorkspace, session: SessionController) -> Self {
        Self {
            workspace,
            session,
            state: Arc::new(Mutex::new(WorkflowRuntime {
                compatibility: FirmwareCompatibility::Unknown,
                active_job: None,
                last_compile: None,
                last_upload: None,
                last_failure: None,
                build_artifact: None,
            })),
            cancel: Arc::new(AtomicBool::new(false)),
            worker: Arc::new(Mutex::new(None)),
            next_job_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn workspace(&self) -> &FirmwareWorkspace {
        &self.workspace
    }

    pub fn status(&self) -> Result<FirmwareWorkflowStatus, FirmwareFailure> {
        let runtime = self.lock_state()?;
        Ok(FirmwareWorkflowStatus {
            compatibility: runtime.compatibility,
            job: runtime.active_job.clone(),
            last_compile: runtime.last_compile.clone(),
            last_upload: runtime.last_upload.clone(),
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
        let boards = cli.boards().unwrap_or_default();
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
            boards,
            ready: problem.is_none(),
            problem,
        }
    }

    pub fn start_compile(
        &self,
        request: CompileProjectRequest,
    ) -> Result<FirmwareJobStatus, FirmwareFailure> {
        if request.unsaved_changes {
            return Err(failure_unsaved(FirmwareJobStage::Preparing));
        }
        let project = self.open_project(&request.project_folder, FirmwareJobStage::Preparing)?;
        let job = self.new_job(
            FirmwareJobKind::Compile,
            Some(project.project_folder.clone()),
            Some(project.source_hash.clone()),
            None,
            None,
        );
        self.start_worker(job.clone(), move |workflow| {
            workflow.compile_project(job.id, project)
        })?;
        Ok(job)
    }

    pub fn start_upload(
        &self,
        request: UploadProjectRequest,
    ) -> Result<FirmwareJobStatus, FirmwareFailure> {
        if !request.confirmation {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::ProjectInvalid,
                FirmwareJobStage::Preparing,
                "Upload requires explicit confirmation showing the project and selected board.",
                "confirmation=false",
            ));
        }
        if request.unsaved_changes {
            return Err(failure_unsaved(FirmwareJobStage::Preparing));
        }
        let project = self.open_project(&request.project_folder, FirmwareJobStage::Preparing)?;
        let board = self.selected_board(&request.port, FirmwareJobStage::Preparing)?;
        let artifact = self.current_build_for(&project, FirmwareJobStage::Preparing)?;
        self.ensure_upload_is_safe(FirmwareJobStage::Preparing)?;
        let job = self.new_job(
            FirmwareJobKind::Upload,
            Some(project.project_folder.clone()),
            Some(project.source_hash.clone()),
            Some(board.port.clone()),
            board.serial_number.clone(),
        );
        self.start_worker(job.clone(), move |workflow| {
            workflow.upload_artifact(job.id, project, board, artifact, false)
        })?;
        Ok(job)
    }

    pub fn start_restore_reference(
        &self,
        request: RestoreReferenceRequest,
    ) -> Result<FirmwareJobStatus, FirmwareFailure> {
        if !request.confirmation {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::ProjectInvalid,
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
                    .last_upload
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
            if matches!(
                job.kind,
                FirmwareJobKind::Upload | FirmwareJobKind::RestoreReference
            ) {
                runtime.compatibility = FirmwareCompatibility::UploadInProgress;
            }
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

    fn compile_project(
        &self,
        job_id: u64,
        project: FirmwareProject,
    ) -> Result<(), FirmwareFailure> {
        self.set_stage(
            job_id,
            FirmwareJobStage::Compiling,
            "Compiling the saved project with Arduino CLI.",
        )?;
        let cli = required_cli(FirmwareJobStage::Compiling)?;
        require_core(&cli, FirmwareJobStage::Compiling)?;
        let build_dir = job_directory(job_id).join("compile");
        fs::create_dir_all(&build_dir)
            .map_err(|error| io_failure(FirmwareJobStage::Compiling, error))?;
        let log = cli
            .compile_to(Path::new(&project.project_folder), &build_dir, &self.cancel)
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
                "Compile canceled by the user.",
                "Arduino CLI child was terminated",
            ));
        }
        if !log.succeeded() {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::CompileFailed,
                FirmwareJobStage::Compiling,
                "Arduino CLI could not compile the saved project. The board was not changed.",
                combined_output(&log),
            ));
        }
        let binary = build_dir.join(format!("{}.bin", project.metadata.source_filename));
        if !binary.is_file() {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::CompileFailed,
                FirmwareJobStage::Compiling,
                "Arduino CLI reported success but did not produce the expected binary.",
                binary.display().to_string(),
            ));
        }
        self.workspace
            .update_compile_success(&project.project_folder, project.source_hash.clone())
            .map_err(workspace_failure)?;
        {
            let mut runtime = self.lock_state()?;
            runtime.build_artifact = Some(BuildArtifact {
                project_folder: project.project_folder.clone(),
                source_hash: project.source_hash,
                binary_path: binary,
            });
        }
        self.finish_success(job_id, FirmwareCompatibility::Unknown)
    }

    fn upload_artifact(
        &self,
        job_id: u64,
        project: FirmwareProject,
        board: BoardInfo,
        artifact: BuildArtifact,
        restore: bool,
    ) -> Result<(), FirmwareFailure> {
        self.set_stage(
            job_id,
            FirmwareJobStage::ClosingSerialSession,
            "Closing any idle acquisition session before upload.",
        )?;
        self.session.disconnect().map_err(session_failure)?;
        let cli = required_cli(FirmwareJobStage::Uploading)?;
        require_core(&cli, FirmwareJobStage::Uploading)?;
        self.set_stage(
            job_id,
            FirmwareJobStage::TouchReset,
            "Arduino CLI will reset the selected UNO R4 WiFi for upload.",
        )?;
        self.set_stage(
            job_id,
            FirmwareJobStage::WaitingForBootloader,
            "Waiting for Arduino CLI bootloader/upload transition.",
        )?;
        self.set_stage(
            job_id,
            FirmwareJobStage::Uploading,
            "Uploading the compiled binary to the selected UNO R4 WiFi.",
        )?;
        let log = cli
            .upload_input(&artifact.binary_path, &board.port, &self.cancel)
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
                "Upload canceled by the user.",
                "Arduino CLI child was terminated",
            ));
        }
        if !log.succeeded() {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::UploadFailed,
                FirmwareJobStage::Uploading,
                "Arduino CLI upload failed; the board may still contain its prior sketch.",
                combined_output(&log),
            ));
        }
        self.set_stage(
            job_id,
            FirmwareJobStage::WaitingForApplicationPort,
            "Waiting for the selected UNO R4 WiFi application port to return.",
        )?;
        let final_board = wait_for_application_port(&cli, &board, &self.cancel, |candidate| {
            self.update_job(job_id, |job| {
                if !candidate.port.eq_ignore_ascii_case(&board.port) {
                    job.final_port = Some(candidate.port.clone());
                }
            })
        })?;
        self.update_job(job_id, |job| {
            job.final_port = Some(final_board.port.clone())
        })?;
        match project.metadata.verification_kind {
            FirmwareVerificationKind::NonWvu => {
                let verification = FirmwareVerification {
                    declared_kind: FirmwareVerificationKind::NonWvu,
                    compatible: false,
                    protocol_version: None,
                    identity: None,
                    bytes_received: None,
                    valid_frames: None,
                    crc_failures: None,
                    explanation: "Upload completed. This project is declared as non-WVU firmware, so binary acquisition remains unavailable until compatible firmware is restored.".into(),
                };
                self.update_job(job_id, |job| job.verification = Some(verification))?;
                if !restore {
                    self.workspace
                        .update_upload_success(&project.project_folder, final_board.port, None)
                        .map_err(workspace_failure)?;
                }
                self.finish_success(job_id, FirmwareCompatibility::NonWvuSketch)
            }
            FirmwareVerificationKind::WvuProtocolReference => {
                self.verify_uploaded_reference(job_id, project.project_folder, final_board, restore)
            }
        }
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
        fs::write(
            &source_path,
            FirmwareWorkspace::controlled_reference_source().map_err(workspace_failure)?,
        )
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
        let artifact = BuildArtifact {
            project_folder: "controlled-reference".into(),
            source_hash: stable_hash(
                &fs::read(&source_path)
                    .map_err(|error| io_failure(FirmwareJobStage::Compiling, error))?,
            ),
            binary_path: build_dir.join("reference_unor4wifi.ino.bin"),
        };
        self.update_job(job_id, |job| {
            job.source_hash = Some(artifact.source_hash.clone())
        })?;
        if !artifact.binary_path.is_file() {
            return Err(FirmwareFailure::new(
                FirmwareErrorCategory::CompileFailed,
                FirmwareJobStage::Compiling,
                "Reference compile did not produce a binary.",
                artifact.binary_path.display().to_string(),
            ));
        }
        let project = FirmwareProject {
            project_folder: artifact.project_folder.clone(),
            source_path: source_path.display().to_string(),
            metadata_path: String::new(),
            metadata: controlled_reference_metadata(),
            source: String::new(),
            source_hash: artifact.source_hash.clone(),
        };
        self.upload_artifact(job_id, project, board, artifact, true)
    }

    fn verify_uploaded_reference(
        &self,
        job_id: u64,
        project_folder: String,
        board: BoardInfo,
        restore: bool,
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
        if !restore {
            self.workspace
                .update_upload_success(&project_folder, board.port, verification.identity.clone())
                .map_err(workspace_failure)?;
        }
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

    fn current_build_for(
        &self,
        project: &FirmwareProject,
        stage: FirmwareJobStage,
    ) -> Result<BuildArtifact, FirmwareFailure> {
        let artifact = self.lock_state()?.build_artifact.clone().ok_or_else(|| {
            FirmwareFailure::new(
                FirmwareErrorCategory::CompileFailed,
                stage,
                "Compile this saved project before uploading.",
                "no current build artifact",
            )
        })?;
        if artifact.project_folder != project.project_folder
            || artifact.source_hash != project.source_hash
            || !artifact.binary_path.is_file()
        {
            return Err(FirmwareFailure::new(FirmwareErrorCategory::CompileFailed, stage, "The selected project has not been successfully compiled in its current saved state.", "compile artifact does not match project/source hash"));
        }
        Ok(artifact)
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
        project_folder: Option<String>,
        source_hash: Option<String>,
        original_port: Option<String>,
        board_serial: Option<String>,
    ) -> FirmwareJobStatus {
        FirmwareJobStatus {
            id: self.next_job_id.fetch_add(1, Ordering::Relaxed),
            kind,
            stage: FirmwareJobStage::Preparing,
            active: true,
            project_folder,
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
        // Keep the active snapshot published while log I/O runs. Otherwise a
        // status poll can observe neither `job` nor `last_compile/upload`.
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
        if job.kind == FirmwareJobKind::Compile {
            runtime.last_compile = Some(job);
        } else {
            runtime.last_upload = Some(job);
        }
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
        if job.kind == FirmwareJobKind::Compile {
            runtime.last_compile = Some(job);
        } else {
            runtime.compatibility = FirmwareCompatibility::VerificationFailed;
            runtime.last_upload = Some(job);
        }
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

    fn open_project(
        &self,
        path: &str,
        stage: FirmwareJobStage,
    ) -> Result<FirmwareProject, FirmwareFailure> {
        self.workspace
            .open_project(path)
            .map_err(|error| workspace_failure_at(stage, error))
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
        crate::arduino_cli::CliError::NotFound => FirmwareFailure::new(
            FirmwareErrorCategory::ArduinoCliMissing,
            stage,
            "Arduino CLI is not available. Editing and saving remain available.",
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
            "Arduino UNO R4 core is not installed. Install `arduino:renesas_uno` before compiling.",
            error.to_string(),
        )
    })
}

fn wait_for_application_port<F>(
    cli: &ArduinoCli,
    original: &BoardInfo,
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
            select_returned_application_port(original, cli.boards().unwrap_or_default())?
        {
            observe(&board)?;
            return Ok(board);
        }
        thread::sleep(APPLICATION_PORT_POLL);
    }
    Err(FirmwareFailure::new(FirmwareErrorCategory::ApplicationPortNotFound, FirmwareJobStage::WaitingForApplicationPort, "Arduino CLI reported upload success, but the selected UNO R4 WiFi application port did not return in time.", original.port.clone()))
}

/// Matches a returning application port without assuming the COM number is
/// stable. A serial number is authoritative; without one, the safer Phase 2
/// fallback is the original port rather than picking a different unknown device.
fn select_returned_application_port(
    original: &BoardInfo,
    boards: Vec<BoardInfo>,
) -> Result<Option<BoardInfo>, FirmwareFailure> {
    let candidates: Vec<_> = boards
        .into_iter()
        .filter(|candidate| {
            candidate.fqbn == UNO_R4_WIFI_FQBN
                && match (&original.serial_number, &candidate.serial_number) {
                    (Some(expected), Some(actual)) => expected == actual,
                    (Some(_), None) => false,
                    (None, _) => candidate.port.eq_ignore_ascii_case(&original.port),
                }
        })
        .collect();
    match candidates.as_slice() {
        [] => Ok(None),
        [board] => Ok(Some(board.clone())),
        _ => Err(FirmwareFailure::new(
            FirmwareErrorCategory::AmbiguousBoard,
            FirmwareJobStage::WaitingForApplicationPort,
            "Multiple UNO R4 WiFi application ports match the returning board.",
            "ambiguous board serial/port candidates",
        )),
    }
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

fn controlled_reference_metadata() -> crate::firmware_workspace::ProjectMetadata {
    crate::firmware_workspace::ProjectMetadata {
        schema_version: crate::firmware_workspace::PROJECT_SCHEMA_VERSION,
        project_name: "reference_unor4wifi".into(),
        created_utc: Utc::now(),
        modified_utc: Utc::now(),
        target_board: "Arduino UNO R4 WiFi".into(),
        fqbn: UNO_R4_WIFI_FQBN.into(),
        source_filename: "reference_unor4wifi.ino".into(),
        selected_com_port: None,
        template_origin: crate::firmware_workspace::TemplateKind::WvuProtocolReference,
        verification_kind: FirmwareVerificationKind::WvuProtocolReference,
        lab_profile: None,
        notes: None,
        last_successful_compile_utc: None,
        last_successful_upload_utc: None,
        last_verified_firmware_identity: None,
        last_compile_source_hash: None,
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
        .join("firmware_workspace")
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
    Ok(path.display().to_string())
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
        FirmwareErrorCategory::ArduinoCliMissing => ("Arduino CLI unavailable", "Install Arduino CLI or configure an instructor-approved path, then refresh the environment."),
        FirmwareErrorCategory::CoreMissing => ("UNO R4 core unavailable", "Install the Arduino UNO R4 core, then refresh the environment."),
        FirmwareErrorCategory::ProjectInvalid => ("Project needs attention", "Correct the project path/name or confirm the requested destructive action."),
        FirmwareErrorCategory::UnsavedChanges => ("Save required", "Save the editor contents, then compile or upload again."),
        FirmwareErrorCategory::CompileFailed => ("Compile failed", "Review the compiler output and navigate to the reported source line."),
        FirmwareErrorCategory::PortBusy => ("Serial port is busy", "Stop/finalize acquisition and close other serial programs, then retry."),
        FirmwareErrorCategory::UnsupportedBoard => ("Unsupported board", "Select a detected Arduino UNO R4 WiFi."),
        FirmwareErrorCategory::BoardNotFound => ("Board not found", "Refresh boards and select the connected UNO R4 WiFi."),
        FirmwareErrorCategory::AmbiguousBoard => ("Board identity is ambiguous", "Disconnect other matching boards and retry with the board serial shown."),
        FirmwareErrorCategory::ResetFailed => ("Reset transition failed", "Reconnect the selected UNO R4 WiFi and retry the explicit upload."),
        FirmwareErrorCategory::BootloaderNotFound => ("Bootloader not found", "Confirm the selected UNO R4 WiFi is connected, then retry upload."),
        FirmwareErrorCategory::UploadFailed => ("Upload failed", "Review Arduino CLI output and ensure no other program owns the selected COM port."),
        FirmwareErrorCategory::ApplicationPortNotFound => ("Application port did not return", "Refresh boards after upload. Do not assume the old COM number."),
        FirmwareErrorCategory::ProtocolVerificationFailed => ("Protocol verification failed", "Restore the controlled WVU reference firmware before using Acquisition."),
        FirmwareErrorCategory::WrongFirmwareIdentity => ("Firmware identity mismatch", "Restore the controlled WVU reference firmware before using Acquisition."),
        FirmwareErrorCategory::Canceled => ("Operation canceled", "Review the operation log and compile again before uploading."),
        FirmwareErrorCategory::InternalError => ("Firmware workflow error", "Copy diagnostics and contact the instructor/developer."),
    }
}

fn workspace_failure(error: WorkspaceError) -> FirmwareFailure {
    workspace_failure_at(FirmwareJobStage::Preparing, error)
}

fn workspace_failure_at(stage: FirmwareJobStage, error: WorkspaceError) -> FirmwareFailure {
    FirmwareFailure::new(
        FirmwareErrorCategory::ProjectInvalid,
        stage,
        error.to_string(),
        error.to_string(),
    )
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

fn failure_unsaved(stage: FirmwareJobStage) -> FirmwareFailure {
    FirmwareFailure::new(
        FirmwareErrorCategory::UnsavedChanges,
        stage,
        "Save editor changes before compiling or uploading.",
        "frontend reported unsaved changes",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firmware_workspace::{CreateProjectRequest, TemplateKind};
    use tempfile::tempdir;

    fn board(port: &str, serial: Option<&str>) -> BoardInfo {
        BoardInfo {
            port: port.into(),
            name: "Arduino UNO R4 WiFi".into(),
            fqbn: UNO_R4_WIFI_FQBN.into(),
            serial_number: serial.map(str::to_owned),
        }
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
            protocol_version: Some("0.1".into()),
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
    fn returning_port_follows_board_serial_and_ignores_unrelated_ports() {
        let original = board("COM12", Some("UNO-SERIAL"));
        let returning = board("COM19", Some("UNO-SERIAL"));
        let unrelated = board("COM3", Some("OTHER-UNO"));
        let selected = select_returned_application_port(&original, vec![unrelated, returning])
            .unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(selected.map(|item| item.port), Some("COM19".into()));
    }

    #[test]
    fn returning_port_never_guesses_a_changed_port_without_identity() {
        let original = board("COM12", None);
        let changed_unknown = board("COM19", Some("UNKNOWN"));
        assert!(
            select_returned_application_port(&original, vec![changed_unknown])
                .unwrap_or_else(|error| panic!("{error:?}"))
                .is_none()
        );
        let same_port = board("COM12", None);
        assert_eq!(
            select_returned_application_port(&original, vec![same_port])
                .unwrap_or_else(|error| panic!("{error:?}"))
                .map(|item| item.port),
            Some("COM12".into())
        );
    }

    #[test]
    fn ambiguous_returning_identity_is_an_actionable_error() {
        let original = board("COM12", Some("UNO-SERIAL"));
        let error = select_returned_application_port(
            &original,
            vec![
                board("COM13", Some("UNO-SERIAL")),
                board("COM14", Some("UNO-SERIAL")),
            ],
        )
        .err()
        .unwrap_or_else(|| panic!("expected an ambiguous-board failure"));
        assert_eq!(error.category, FirmwareErrorCategory::AmbiguousBoard);
    }

    #[test]
    fn terminal_status_snapshot_does_not_create_a_polling_gap() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let workflow = FirmwareWorkflow::with_workspace(
            FirmwareWorkspace::new(dir.path().join("workspace")),
            SessionController::default(),
        );
        let job = workflow.new_job(FirmwareJobKind::Compile, None, None, None, None);
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

    #[test]
    fn upload_preflight_requires_current_matching_compile_and_no_active_session() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let workspace = FirmwareWorkspace::new(dir.path().join("appdata"));
        let project = workspace
            .create_project(CreateProjectRequest {
                parent_folder: dir.path().display().to_string(),
                project_name: "NonWvu".into(),
                template: TemplateKind::SerialDiagnostic,
                notes: None,
                overwrite_confirmed: false,
            })
            .unwrap_or_else(|error| panic!("{error}"));
        let workflow = FirmwareWorkflow::with_workspace(workspace, SessionController::default());
        assert!(workflow
            .current_build_for(&project, FirmwareJobStage::Preparing)
            .is_err());
        let artifact = BuildArtifact {
            project_folder: project.project_folder.clone(),
            source_hash: project.source_hash.clone(),
            binary_path: PathBuf::from("missing.bin"),
        };
        workflow
            .lock_state()
            .unwrap_or_else(|error| panic!("{:?}", error.category))
            .build_artifact = Some(artifact);
        assert!(workflow
            .current_build_for(&project, FirmwareJobStage::Preparing)
            .is_err());
        assert_eq!(board("COM12", Some("ABC")).fqbn, UNO_R4_WIFI_FQBN);
    }
}
