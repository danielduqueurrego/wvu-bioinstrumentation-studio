pub mod acquisition;
pub mod arduino_cli;
pub mod firmware_workflow;
pub mod firmware_workspace;
pub mod profiles;
pub mod protocol;
pub mod recording;
pub mod session;
pub mod validation;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tauri::{Manager, WindowEvent};

struct AppState {
    /// Exactly one controller owns the Phase 1 session lifecycle. It has no frontend serial handle.
    session: session::SessionController,
    /// One firmware workflow coordinates CLI jobs with the same serial-session boundary.
    firmware: firmware_workflow::FirmwareWorkflow,
    /// Profiles are independent of firmware editing but bind every Phase 3A recording.
    profiles: profiles::ProfileStore,
    /// Separate, instructor-authored evidence records bind bench validation to a
    /// frozen profile and controlled firmware identity.
    validations: validation::ValidationStore,
}

#[derive(Serialize)]
struct RecentPoint {
    sequence: u32,
    timestamp_us: u64,
    counts: u16,
}

#[derive(Serialize)]
struct SerialPortInfo {
    port: String,
    kind: String,
}

#[derive(Deserialize)]
struct CreateValidationDraftRequest {
    profile_id: String,
    validation_id: String,
    hardware: validation::ValidationHardware,
    equipment: Vec<validation::EquipmentItem>,
    test_conditions: BTreeMap<String, String>,
    notes: String,
}

#[derive(Deserialize)]
struct ValidationRunStartRequest {
    port: Option<String>,
    output_directory: String,
    duration: recording::RecordingDuration,
    profile_id: String,
    validation_id: String,
    test_type: validation::ValidationTestType,
    run_number: u32,
    bench_validation_acknowledged: bool,
    source_description: String,
    source_setpoint_v: Option<f64>,
    source_offset_v: Option<f64>,
    source_frequency_hz: Option<f64>,
    source_peak_to_peak_v: Option<f64>,
    equipment_metadata: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct CompleteValidationRunRequest {
    validation_id: String,
    test_type: validation::ValidationTestType,
    run_number: u32,
    source_description: String,
    source_setpoint_v: Option<f64>,
    source_frequency_hz: Option<f64>,
    source_peak_to_peak_v: Option<f64>,
    criteria: Vec<validation::AcceptanceCriterion>,
    notes: String,
}

#[tauri::command]
fn list_boards(cli_path: Option<String>) -> Result<Vec<arduino_cli::BoardInfo>, String> {
    let override_path = cli_path.as_deref().map(PathBuf::from);
    let cli =
        arduino_cli::ArduinoCli::discover(override_path.as_deref()).map_err(|e| e.to_string())?;
    cli.boards().map_err(|e| e.to_string())
}

#[tauri::command]
fn list_serial_ports() -> Result<Vec<SerialPortInfo>, String> {
    serialport::available_ports()
        .map_err(|error| format!("could not enumerate serial ports: {error}"))
        .map(|ports| {
            ports
                .into_iter()
                .map(|port| SerialPortInfo {
                    port: port.port_name,
                    kind: format!("{:?}", port.port_type),
                })
                .collect()
        })
}

#[tauri::command]
fn arduino_cli_version(cli_path: Option<String>) -> Result<arduino_cli::CommandLog, String> {
    let override_path = cli_path.as_deref().map(PathBuf::from);
    let cli =
        arduino_cli::ArduinoCli::discover(override_path.as_deref()).map_err(|e| e.to_string())?;
    cli.version().map_err(|e| e.to_string())
}

fn firmware_error<T>(result: Result<T, firmware_workflow::FirmwareFailure>) -> Result<T, String> {
    result.map_err(|failure| {
        serde_json::to_string(&failure)
            .unwrap_or_else(|error| format!("firmware workflow error: {error}"))
    })
}

#[tauri::command]
fn firmware_environment(
    state: tauri::State<'_, AppState>,
) -> firmware_workflow::FirmwareEnvironmentStatus {
    state.firmware.environment()
}

#[tauri::command]
fn list_firmware_templates() -> Vec<firmware_workspace::TemplateInfo> {
    firmware_workspace::FirmwareWorkspace::templates()
}

#[tauri::command]
fn create_firmware_project(
    state: tauri::State<'_, AppState>,
    request: firmware_workspace::CreateProjectRequest,
) -> Result<firmware_workspace::FirmwareProject, String> {
    state
        .firmware
        .workspace()
        .create_project(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn open_firmware_project(
    state: tauri::State<'_, AppState>,
    project_folder: String,
) -> Result<firmware_workspace::FirmwareProject, String> {
    state
        .firmware
        .workspace()
        .open_project(&project_folder)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_firmware_project(
    state: tauri::State<'_, AppState>,
    request: firmware_workspace::SaveProjectRequest,
) -> Result<firmware_workspace::FirmwareProject, String> {
    state
        .firmware
        .workspace()
        .save_project(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_firmware_project_as(
    state: tauri::State<'_, AppState>,
    request: firmware_workspace::SaveAsProjectRequest,
) -> Result<firmware_workspace::FirmwareProject, String> {
    state
        .firmware
        .workspace()
        .save_as_project(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn list_recent_firmware_projects(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    state
        .firmware
        .workspace()
        .recent_projects()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn restore_firmware_project_saved_source(
    state: tauri::State<'_, AppState>,
    project_folder: String,
) -> Result<String, String> {
    state
        .firmware
        .workspace()
        .restore_saved_source(&project_folder)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn start_firmware_compile(
    state: tauri::State<'_, AppState>,
    request: firmware_workflow::CompileProjectRequest,
) -> Result<firmware_workflow::FirmwareJobStatus, String> {
    firmware_error(state.firmware.start_compile(request))
}

#[tauri::command]
fn start_firmware_upload(
    state: tauri::State<'_, AppState>,
    request: firmware_workflow::UploadProjectRequest,
) -> Result<firmware_workflow::FirmwareJobStatus, String> {
    firmware_error(state.firmware.start_upload(request))
}

#[tauri::command]
fn restore_wvu_reference_firmware(
    state: tauri::State<'_, AppState>,
    request: firmware_workflow::RestoreReferenceRequest,
) -> Result<firmware_workflow::FirmwareJobStatus, String> {
    firmware_error(state.firmware.start_restore_reference(request))
}

#[tauri::command]
fn cancel_firmware_job(
    state: tauri::State<'_, AppState>,
) -> Result<firmware_workflow::FirmwareWorkflowStatus, String> {
    firmware_error(state.firmware.cancel_active_job())
}

#[tauri::command]
fn get_firmware_workflow_status(
    state: tauri::State<'_, AppState>,
) -> Result<firmware_workflow::FirmwareWorkflowStatus, String> {
    firmware_error(state.firmware.status())
}

#[tauri::command]
fn verify_wvu_reference_firmware(
    state: tauri::State<'_, AppState>,
    port: String,
) -> Result<firmware_workflow::FirmwareVerification, String> {
    firmware_error(state.firmware.verify_existing_reference(port))
}

#[tauri::command]
fn list_acquisition_profiles(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<profiles::AcquisitionProfile>, String> {
    state.profiles.list().map_err(|error| error.to_string())
}

#[tauri::command]
fn get_profile_mode(state: tauri::State<'_, AppState>) -> Result<profiles::ProfileMode, String> {
    state.profiles.mode().map_err(|error| error.to_string())
}

#[tauri::command]
fn set_profile_mode(
    state: tauri::State<'_, AppState>,
    mode: profiles::ProfileMode,
    acknowledgement: bool,
) -> Result<profiles::ProfileMode, String> {
    state
        .profiles
        .set_mode(mode, acknowledgement)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn duplicate_profile_to_draft(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    draft_id: String,
) -> Result<profiles::AcquisitionProfile, String> {
    state
        .profiles
        .duplicate_to_draft(&profile_id, &draft_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn update_profile_draft_description(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    profile_version: String,
    description: String,
) -> Result<profiles::AcquisitionProfile, String> {
    state
        .profiles
        .update_draft_description(&profile_id, &profile_version, description)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn finalize_profile_draft(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    profile_version: String,
    final_version: String,
) -> Result<profiles::AcquisitionProfile, String> {
    state
        .profiles
        .finalize_draft(&profile_id, &profile_version, final_version)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn retire_profile(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    profile_version: String,
) -> Result<(), String> {
    state
        .profiles
        .retire(&profile_id, &profile_version)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn export_profile_package(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    profile_version: String,
    destination: String,
) -> Result<(), String> {
    state
        .profiles
        .export_profile(&profile_id, &profile_version, &PathBuf::from(destination))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn import_profile_package(
    state: tauri::State<'_, AppState>,
    source: String,
) -> Result<profiles::AcquisitionProfile, String> {
    state
        .profiles
        .import_profile(&PathBuf::from(source))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn list_validation_evidence(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<validation::ValidationEvidence>, String> {
    state.validations.list().map_err(|error| error.to_string())
}

#[tauri::command]
fn get_profile_validation_status(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<validation::ValidationStatusSummary, String> {
    let profile = state
        .profiles
        .get_locked(&profile_id)
        .map_err(|error| error.to_string())?;
    state
        .validations
        .profile_status(&profile)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn create_validation_draft(
    state: tauri::State<'_, AppState>,
    request: CreateValidationDraftRequest,
) -> Result<validation::ValidationEvidence, String> {
    ensure_validation_session_idle(&state)?;
    let mode = state.profiles.mode().map_err(|error| error.to_string())?;
    let profile = state
        .profiles
        .get_locked(&request.profile_id)
        .map_err(|error| error.to_string())?;
    ensure_bench_validation_profile(&profile)?;
    let created = state
        .validations
        .create_draft(
            mode.clone(),
            &profile,
            request.validation_id.clone(),
            request.hardware.clone(),
        )
        .map_err(|error| error.to_string())?;
    state
        .validations
        .update_draft_details(
            mode,
            &created.validation_id,
            request.hardware,
            request.equipment,
            request.test_conditions,
            request.notes,
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn start_validation_simulator_recording(
    state: tauri::State<'_, AppState>,
    request: ValidationRunStartRequest,
) -> Result<session::SessionStatus, String> {
    checked_output_directory(&request.output_directory)?;
    let mode = state.profiles.mode().map_err(|error| error.to_string())?;
    if mode != profiles::ProfileMode::InstructorAuthoring {
        return Err("Student mode cannot start an instructor bench-validation run.".into());
    }
    let profile = state
        .profiles
        .get_locked(&request.profile_id)
        .map_err(|error| error.to_string())?;
    ensure_bench_validation_profile(&profile)?;
    let evidence = state
        .validations
        .get(&request.validation_id)
        .map_err(|error| error.to_string())?;
    if evidence.status != validation::ValidationEvidenceStatus::Draft {
        return Err("validation runs may be added only to a draft validation record".into());
    }
    evidence
        .matches_profile(&profile)
        .map_err(|error| error.to_string())?;
    if !request.bench_validation_acknowledged {
        return Err("acknowledge that validation is bench-only with no person or electrode system connected".into());
    }
    let context = validation_context_from_request(&request, true)?;
    state
        .session
        .start_simulator_validation(
            profile.snapshot(true),
            request.duration,
            PathBuf::from(request.output_directory),
            context,
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn start_validation_hardware_recording(
    state: tauri::State<'_, AppState>,
    request: ValidationRunStartRequest,
) -> Result<session::SessionStatus, String> {
    checked_output_directory(&request.output_directory)?;
    let port = request
        .port
        .clone()
        .ok_or_else(|| "select a detected UNO R4 WiFi port for hardware validation".to_string())?;
    let mode = state.profiles.mode().map_err(|error| error.to_string())?;
    if mode != profiles::ProfileMode::InstructorAuthoring {
        return Err("Student mode cannot start an instructor bench-validation run.".into());
    }
    let profile = state
        .profiles
        .get_locked(&request.profile_id)
        .map_err(|error| error.to_string())?;
    ensure_bench_validation_profile(&profile)?;
    let evidence = state
        .validations
        .get(&request.validation_id)
        .map_err(|error| error.to_string())?;
    if evidence.status != validation::ValidationEvidenceStatus::Draft {
        return Err("validation runs may be added only to a draft validation record".into());
    }
    evidence
        .matches_profile(&profile)
        .map_err(|error| error.to_string())?;
    if !request.bench_validation_acknowledged {
        return Err("acknowledge that validation is bench-only with no person or electrode system connected".into());
    }
    if !firmware_error(state.firmware.is_acquisition_allowed(&port))? {
        return Err("Bench validation requires verified controlled WVU firmware on the selected UNO R4 WiFi.".into());
    }
    let cli = arduino_cli::ArduinoCli::discover(None).map_err(|error| error.to_string())?;
    if !cli
        .boards()
        .map_err(|error| error.to_string())?
        .iter()
        .any(|board| board.port.eq_ignore_ascii_case(&port))
    {
        return Err("select a currently detected Arduino UNO R4 WiFi port; unrelated serial ports are not allowed".into());
    }
    let context = validation_context_from_request(&request, false)?;
    state
        .session
        .start_serial_validation(
            profile.snapshot(true),
            port,
            request.duration,
            PathBuf::from(request.output_directory),
            context,
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn complete_validation_run(
    state: tauri::State<'_, AppState>,
    request: CompleteValidationRunRequest,
) -> Result<validation::ValidationEvidence, String> {
    ensure_validation_session_idle(&state)?;
    let mode = state.profiles.mode().map_err(|error| error.to_string())?;
    let summary = state
        .session
        .status()
        .map_err(|error| error.to_string())?
        .last_summary
        .ok_or_else(|| {
            "stop and finalize the validation recording before computing its metrics".to_string()
        })?;
    let context = summary
        .validation_context
        .as_ref()
        .ok_or_else(|| "the latest recording is not a validation run".to_string())?;
    if context.validation_id != request.validation_id
        || context.test_type != request.test_type.label()
        || context.run_number != request.run_number
    {
        return Err("the latest finalized recording does not match this validation ID, test type, and run number".into());
    }
    let (_metrics_summary, metrics) = validation::metrics_for_validation_run(
        PathBuf::from(&summary.bmeg_path).as_path(),
        &request.test_type,
        request.source_setpoint_v,
        request.source_frequency_hz,
    )
    .map_err(|error| error.to_string())?;
    let criteria = validation::evaluate_criteria(&metrics, &request.criteria);
    state
        .validations
        .add_run(
            mode,
            &request.validation_id,
            validation::ValidationRun {
                run_number: request.run_number,
                test_type: request.test_type,
                source_description: request.source_description,
                source_setpoint_v: request.source_setpoint_v,
                source_frequency_hz: request.source_frequency_hz,
                source_peak_to_peak_v: request.source_peak_to_peak_v,
                bmeg_path: summary.bmeg_path,
                metadata_path: summary.metadata_path,
                csv_path: summary.csv_path,
                raw_sample_count: summary.samples,
                algorithm_version: validation::METRIC_ALGORITHM_VERSION.into(),
                metrics,
                criteria,
                notes: request.notes,
            },
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn set_validation_acceptance_summary(
    state: tauri::State<'_, AppState>,
    validation_id: String,
    summary: Vec<validation::CriterionResult>,
    accepted: bool,
) -> Result<validation::ValidationEvidence, String> {
    ensure_validation_session_idle(&state)?;
    let mode = state.profiles.mode().map_err(|error| error.to_string())?;
    state
        .validations
        .set_acceptance_summary(mode, &validation_id, summary, accepted)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn finalize_validation_evidence(
    state: tauri::State<'_, AppState>,
    validation_id: String,
    profile_id: String,
) -> Result<validation::ValidationEvidence, String> {
    ensure_validation_session_idle(&state)?;
    let mode = state.profiles.mode().map_err(|error| error.to_string())?;
    let profile = state
        .profiles
        .get_locked(&profile_id)
        .map_err(|error| error.to_string())?;
    ensure_bench_validation_profile(&profile)?;
    state
        .validations
        .finalize(mode, &validation_id, &profile)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn retire_validation_evidence(
    state: tauri::State<'_, AppState>,
    validation_id: String,
) -> Result<(), String> {
    ensure_validation_session_idle(&state)?;
    let mode = state.profiles.mode().map_err(|error| error.to_string())?;
    state
        .validations
        .retire(mode, &validation_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn export_validation_package(
    state: tauri::State<'_, AppState>,
    validation_id: String,
    destination: String,
) -> Result<String, String> {
    ensure_validation_session_idle(&state)?;
    let mode = state.profiles.mode().map_err(|error| error.to_string())?;
    if destination.trim().is_empty() || destination.contains('\0') {
        return Err("choose a valid validation package folder".into());
    }
    state
        .validations
        .export_package(mode, &validation_id, &PathBuf::from(destination))
        .map(|path| path.display().to_string())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn import_validation_package(
    state: tauri::State<'_, AppState>,
    source: String,
    profile_id: String,
) -> Result<validation::ValidationEvidence, String> {
    ensure_validation_session_idle(&state)?;
    let mode = state.profiles.mode().map_err(|error| error.to_string())?;
    let profile = state
        .profiles
        .get_locked(&profile_id)
        .map_err(|error| error.to_string())?;
    ensure_bench_validation_profile(&profile)?;
    state
        .validations
        .import_package(mode, &PathBuf::from(source), &profile)
        .map_err(|error| error.to_string())
}

/// Combined Phase 1 connect/handshake/configure/start command. The worker owns transport I/O.
#[tauri::command]
fn start_simulator_recording(
    state: tauri::State<'_, AppState>,
    output_directory: String,
    duration: recording::RecordingDuration,
) -> Result<session::SessionStatus, String> {
    checked_output_directory(&output_directory)?;
    state
        .session
        .start_simulator(duration, PathBuf::from(output_directory))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn start_profile_simulator_recording(
    state: tauri::State<'_, AppState>,
    output_directory: String,
    duration: recording::RecordingDuration,
    profile_id: String,
    bench_notice_acknowledged: bool,
) -> Result<session::SessionStatus, String> {
    checked_output_directory(&output_directory)?;
    let profile = state
        .profiles
        .get_locked(&profile_id)
        .map_err(|error| error.to_string())?;
    state
        .session
        .start_simulator_with_profile(
            profile.snapshot(bench_notice_acknowledged),
            duration,
            PathBuf::from(output_directory),
        )
        .map_err(|error| error.to_string())
}

/// Combined Phase 1 connect/handshake/configure/start command for a discovered UNO R4 WiFi.
#[tauri::command]
fn start_hardware_recording(
    state: tauri::State<'_, AppState>,
    port: String,
    output_directory: String,
    duration: recording::RecordingDuration,
) -> Result<session::SessionStatus, String> {
    checked_output_directory(&output_directory)?;
    if !firmware_error(state.firmware.is_acquisition_allowed(&port))? {
        return Err("firmware compatibility is not verified. Open Firmware, select the UNO R4 WiFi, and verify or restore the WVU reference firmware before acquisition.".into());
    }
    if !serialport::available_ports()
        .map_err(|error| format!("could not enumerate serial ports: {error}"))?
        .iter()
        .any(|candidate| candidate.port_name.eq_ignore_ascii_case(&port))
    {
        return Err("select a currently enumerated serial port".into());
    }
    let cli = arduino_cli::ArduinoCli::discover(None).map_err(|error| error.to_string())?;
    let supported = cli
        .boards()
        .map_err(|error| error.to_string())?
        .into_iter()
        .any(|board| board.port.eq_ignore_ascii_case(&port));
    if !supported {
        return Err(
            "selected port is not a detected Arduino UNO R4 WiFi; refresh boards or use Simulator"
                .into(),
        );
    }
    state
        .session
        .start_serial(port, duration, PathBuf::from(output_directory))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn start_profile_hardware_recording(
    state: tauri::State<'_, AppState>,
    port: String,
    output_directory: String,
    duration: recording::RecordingDuration,
    profile_id: String,
    bench_notice_acknowledged: bool,
) -> Result<session::SessionStatus, String> {
    checked_output_directory(&output_directory)?;
    let profile = state
        .profiles
        .get_locked(&profile_id)
        .map_err(|error| error.to_string())?;
    if !firmware_error(state.firmware.is_acquisition_allowed(&port))? {
        return Err("Profile requires the controlled WVU firmware. Verify or restore it in Firmware before hardware recording.".into());
    }
    if !serialport::available_ports()
        .map_err(|error| format!("could not enumerate serial ports: {error}"))?
        .iter()
        .any(|candidate| candidate.port_name.eq_ignore_ascii_case(&port))
    {
        return Err("select a currently enumerated serial port".into());
    }
    let cli = arduino_cli::ArduinoCli::discover(None).map_err(|error| error.to_string())?;
    if !cli
        .boards()
        .map_err(|error| error.to_string())?
        .into_iter()
        .any(|board| board.port.eq_ignore_ascii_case(&port))
    {
        return Err(
            "selected port is not a detected Arduino UNO R4 WiFi; refresh boards or use Simulator"
                .into(),
        );
    }
    state
        .session
        .start_serial_with_profile(
            profile.snapshot(bench_notice_acknowledged),
            port,
            duration,
            PathBuf::from(output_directory),
        )
        .map_err(|error| error.to_string())
}

/// Explicit recovery action for a previously discovered UNO R4 WiFi. It performs
/// only a 1200-bps touch/reset and a fresh protocol handshake; it never uploads firmware.
#[tauri::command]
fn reset_board_and_retry(
    state: tauri::State<'_, AppState>,
    port: String,
) -> Result<session::ResetRetryResult, String> {
    if port.trim().is_empty() || port.contains('\0') {
        return Err("select a valid discovered UNO R4 WiFi port".into());
    }
    let cli = arduino_cli::ArduinoCli::discover(None).map_err(|error| error.to_string())?;
    let board = cli
        .boards()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|board| board.port.eq_ignore_ascii_case(&port))
        .ok_or_else(|| {
            "selected port is not a currently discovered Arduino UNO R4 WiFi; refresh devices first"
                .to_string()
        })?;
    state
        .session
        .reset_and_retry(session::ResetTarget {
            port: board.port,
            serial_number: board.serial_number,
        })
        .map_err(|error| error.to_string())
}

/// Explicit no-reset retry for a discovered UNO R4 WiFi. This is distinct from
/// Reset and retry so normal connection attempts never touch the board at 1200 bps.
#[tauri::command]
fn retry_hardware_handshake(
    state: tauri::State<'_, AppState>,
    port: String,
) -> Result<session::HandshakeRetryResult, String> {
    if port.trim().is_empty() || port.contains('\0') {
        return Err("select a valid discovered UNO R4 WiFi port".into());
    }
    let cli = arduino_cli::ArduinoCli::discover(None).map_err(|error| error.to_string())?;
    let board = cli
        .boards()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|board| board.port.eq_ignore_ascii_case(&port))
        .ok_or_else(|| {
            "selected port is not a currently discovered Arduino UNO R4 WiFi; refresh devices first"
                .to_string()
        })?;
    state
        .session
        .retry_handshake(session::ResetTarget {
            port: board.port,
            serial_number: board.serial_number,
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn stop_recording(state: tauri::State<'_, AppState>) -> Result<session::SessionStatus, String> {
    state
        .session
        .request_stop()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn disconnect_session(state: tauri::State<'_, AppState>) -> Result<session::SessionStatus, String> {
    state
        .session
        .disconnect()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_session_status(state: tauri::State<'_, AppState>) -> Result<session::SessionStatus, String> {
    state.session.status().map_err(|error| error.to_string())
}

/// Bounded snapshot for a 20–30 Hz polling UI; it never emits per-sample events.
#[tauri::command]
fn get_recent_display_data(state: tauri::State<'_, AppState>) -> Result<Vec<RecentPoint>, String> {
    state
        .session
        .recent_samples()
        .map_err(|error| error.to_string())
        .map(|samples| {
            samples
                .into_iter()
                .map(|sample| RecentPoint {
                    sequence: sample.sequence,
                    timestamp_us: sample.timestamp_us,
                    counts: sample.counts,
                })
                .collect()
        })
}

/// CSV is finalized automatically from the BMEG stream. This exposes its recorded path.
#[tauri::command]
fn export_session_csv(state: tauri::State<'_, AppState>) -> Result<String, String> {
    state
        .session
        .status()
        .map_err(|error| error.to_string())?
        .last_summary
        .map(|summary| summary.csv_path)
        .ok_or_else(|| "no finalized recording is available to export".into())
}

fn checked_output_directory(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("choose a non-empty output directory".into());
    }
    if value.contains('\0') {
        return Err("output directory contains a NUL character".into());
    }
    Ok(())
}

fn ensure_validation_session_idle(state: &AppState) -> Result<(), String> {
    if state
        .session
        .is_recording()
        .map_err(|error| error.to_string())?
    {
        Err("stop and finalize the active acquisition before changing validation evidence".into())
    } else {
        Ok(())
    }
}

fn ensure_bench_validation_profile(profile: &profiles::AcquisitionProfile) -> Result<(), String> {
    if matches!(profile.category.as_str(), "ecg" | "emg") {
        Ok(())
    } else {
        Err(
            "bench validation is available only for the locked ECG or EMG raw-output profiles"
                .into(),
        )
    }
}

fn validation_context_from_request(
    request: &ValidationRunStartRequest,
    simulator: bool,
) -> Result<recording::ValidationRunContext, String> {
    if request.run_number == 0
        || request.validation_id.trim().is_empty()
        || request.source_description.trim().is_empty()
    {
        return Err(
            "validation ID, nonzero run number, and source description are required".into(),
        );
    }
    for value in [request.source_setpoint_v, request.source_peak_to_peak_v] {
        if value.is_some_and(|volts| !(0.0..=5.0).contains(&volts)) {
            return Err("bench source voltage values must remain within 0 to 5 V".into());
        }
    }
    if request
        .source_frequency_hz
        .is_some_and(|frequency| frequency <= 0.0)
    {
        return Err("bench sine frequency must be positive".into());
    }
    Ok(recording::ValidationRunContext {
        validation_id: request.validation_id.clone(),
        test_type: request.test_type.label().into(),
        run_number: request.run_number,
        bench_only: true,
        source_description: request.source_description.clone(),
        source_setpoint_v: request.source_setpoint_v,
        source_offset_v: request.source_offset_v,
        source_frequency_hz: request.source_frequency_hz,
        source_peak_to_peak_v: request.source_peak_to_peak_v,
        equipment_metadata: request.equipment_metadata.clone(),
        simulator_parameters: if simulator {
            BTreeMap::from([
                ("seed".into(), "phase3b-deterministic-v1".into()),
                ("test_type".into(), request.test_type.label().into()),
                (
                    "offset_v".into(),
                    request.source_offset_v.unwrap_or(2.5).to_string(),
                ),
                (
                    "setpoint_v".into(),
                    request
                        .source_setpoint_v
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
                (
                    "frequency_hz".into(),
                    request
                        .source_frequency_hz
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
                (
                    "peak_to_peak_v".into(),
                    request
                        .source_peak_to_peak_v
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
            ])
        } else {
            BTreeMap::new()
        },
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // The acquisition controller is intentionally shared with the firmware workflow:
    // an upload first releases this one serial owner before Arduino CLI opens the port.
    let session = session::SessionController::default();
    let firmware = firmware_workflow::FirmwareWorkflow::new(session.clone());
    let profiles = profiles::ProfileStore::default();
    let validations = validation::ValidationStore::default();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            session,
            firmware,
            profiles,
            validations,
        })
        .invoke_handler(tauri::generate_handler![
            list_boards,
            list_serial_ports,
            arduino_cli_version,
            firmware_environment,
            list_firmware_templates,
            create_firmware_project,
            open_firmware_project,
            save_firmware_project,
            save_firmware_project_as,
            list_recent_firmware_projects,
            restore_firmware_project_saved_source,
            start_firmware_compile,
            start_firmware_upload,
            restore_wvu_reference_firmware,
            cancel_firmware_job,
            get_firmware_workflow_status,
            verify_wvu_reference_firmware,
            list_acquisition_profiles,
            get_profile_mode,
            set_profile_mode,
            duplicate_profile_to_draft,
            update_profile_draft_description,
            finalize_profile_draft,
            retire_profile,
            export_profile_package,
            import_profile_package,
            list_validation_evidence,
            get_profile_validation_status,
            create_validation_draft,
            start_validation_simulator_recording,
            start_validation_hardware_recording,
            complete_validation_run,
            set_validation_acceptance_summary,
            finalize_validation_evidence,
            retire_validation_evidence,
            export_validation_package,
            import_validation_package,
            start_simulator_recording,
            start_hardware_recording,
            start_profile_simulator_recording,
            start_profile_hardware_recording,
            reset_board_and_retry,
            retry_hardware_handshake,
            stop_recording,
            disconnect_session,
            get_session_status,
            get_recent_display_data,
            export_session_csv
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let session = window.state::<AppState>().session.clone();
                if matches!(session.is_recording(), Ok(true)) {
                    // There is no dialog plugin in this intentionally small Phase 1 shell.
                    // Preventing close and finalizing first avoids silently abandoning a writer.
                    api.prevent_close();
                    let closing_window = window.clone();
                    std::thread::spawn(move || {
                        let _ = session
                            .request_stop_with_reason(recording::StopReason::ApplicationClose);
                        let _ = session.wait_for_worker();
                        let _ = closing_window.close();
                    });
                }
            }
        })
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| {
            eprintln!("WVU Bioinstrumentation Studio failed to start: {error}")
        });
}

#[cfg(test)]
mod tests {
    use super::{checked_output_directory, ensure_bench_validation_profile};
    use crate::profiles::built_in_profiles;

    #[test]
    fn command_input_validation_rejects_empty_or_nul_directories() {
        assert!(checked_output_directory("").is_err());
        assert!(checked_output_directory("recordings\0bad").is_err());
        assert!(checked_output_directory("recordings").is_ok());
    }

    #[test]
    fn bench_validation_command_guard_allows_only_locked_ecg_or_emg_profiles() {
        let profiles = built_in_profiles().unwrap_or_else(|error| panic!("{error}"));
        assert!(ensure_bench_validation_profile(&profiles[0]).is_err());
        assert!(ensure_bench_validation_profile(&profiles[1]).is_ok());
        assert!(ensure_bench_validation_profile(&profiles[2]).is_ok());
    }
}
