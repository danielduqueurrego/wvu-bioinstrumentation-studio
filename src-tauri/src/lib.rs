pub mod acquisition;
pub mod app_log;
pub mod arduino_cli;
pub mod arduino_runtime;
pub mod calibration;
pub mod firmware_workflow;
pub mod profiles;
pub mod project_paths;
pub mod protocol;
pub mod recording;
pub mod reference_firmware;
pub mod serial_lease;
pub mod session;

use serde::Serialize;
use serial_lease::SerialOwner;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager, WindowEvent};

struct AppState {
    /// Exactly one controller owns the acquisition-session lifecycle. It has no frontend serial handle.
    session: session::SessionController,
    /// One firmware workflow coordinates CLI jobs with the same serial-session boundary.
    firmware: firmware_workflow::FirmwareWorkflow,
    /// Lab definitions are independent of firmware restoration and bind every recording.
    profiles: profiles::ProfileStore,
    /// Student-facing local calibration presets. They never modify a locked profile.
    calibrations: calibration::CalibrationStore,
    /// Window-close progress (`CLOSE_*`): closing during a recording first
    /// saves it, and repeated close clicks never start a second save.
    closing: Arc<AtomicU8>,
}

/// No close is in progress.
const CLOSE_IDLE: u8 = 0;
/// The window asked to close during a recording, which is being saved first.
const CLOSE_SAVING: u8 = 1;
/// The recording was saved; the next close request is allowed.
const CLOSE_READY: u8 = 2;
/// After the recording worker has been joined, how long the close waits for
/// the session to report that it is no longer recording.
const CLOSE_SETTLE_LIMIT: Duration = Duration::from_secs(120);
/// Sent to the page when a close request was refused or deferred.
const CLOSE_BLOCKED_EVENT: &str = "close-blocked";

#[derive(Clone, Serialize)]
struct CloseBlocked {
    reason: &'static str,
    message: &'static str,
}

#[derive(serde::Deserialize)]
struct StartProfileHardwareRequest {
    port: String,
    project_folder: String,
    output_folder: String,
    duration: recording::RecordingDuration,
    profile_id: String,
    bench_notice_acknowledged: bool,
    recording_note: String,
    calibration: Option<calibration::RecordingCalibration>,
}

/// Inputs for the pre-Start storage estimate. Field names match the start
/// requests so the frontend can send the same values.
#[derive(serde::Deserialize)]
struct EstimateRecordingStorageRequest {
    profile_id: String,
    duration: recording::RecordingDuration,
    calibration: Option<calibration::RecordingCalibration>,
}

#[derive(serde::Deserialize)]
struct StartProfileSimulatorRequest {
    project_folder: String,
    output_folder: String,
    duration: recording::RecordingDuration,
    profile_id: String,
    bench_notice_acknowledged: bool,
    recording_note: String,
    calibration: Option<calibration::RecordingCalibration>,
}

/// A concise, structured failure returned only when a recording cannot enter the
/// session worker.  Asynchronous transport failures remain on `SessionStatus`,
/// where their exact detail is retained in `last_error`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartRecordingFailure {
    stage: &'static str,
    code: &'static str,
    user_message: &'static str,
    technical_detail: String,
}

impl StartRecordingFailure {
    fn new(
        stage: &'static str,
        code: &'static str,
        user_message: &'static str,
        technical_detail: impl ToString,
    ) -> Self {
        let technical_detail = technical_detail.to_string();
        app_log::record(
            "WARN",
            &format!("START_FAIL stage={stage} code={code} detail={technical_detail}"),
        );
        Self {
            stage,
            code,
            user_message,
            technical_detail,
        }
    }

    /// Another operation owns the board, or the session is still saving.
    fn busy(error: session::SessionError) -> Self {
        let user_message = match &error {
            session::SessionError::Busy(busy) => busy.message(),
            _ => "The Arduino is busy. Wait for the current operation to finish and try again.",
        };
        Self::new("CLAIM_SERIAL", "session_busy", user_message, error)
    }

    /// A failure while the session prepares the recording. A refused request
    /// (for example an oversized lab header) carries its own student text.
    fn session_start(error: session::SessionError) -> Self {
        let user_message = match &error {
            session::SessionError::InvalidRequest(message) => *message,
            session::SessionError::Busy(busy) => busy.message(),
            _ => "The recording could not be prepared. Try again. If it continues, open Advanced details and share the information with your instructor.",
        };
        Self::new("INITIALIZE_SESSION", "session_start", user_message, error)
    }
}

/// Every board command runs off the main thread; a task that cannot run is
/// reported like any other start failure.
fn start_task_failure(error: impl ToString) -> StartRecordingFailure {
    StartRecordingFailure::new(
        "START_TASK",
        "start_task",
        "The recording could not be prepared. Try again. If it continues, open Advanced details and share the information with your instructor.",
        error,
    )
}

#[derive(Serialize)]
struct RecentPoint {
    sequence: u32,
    timestamp_us: u64,
    values: Vec<u16>,
    status_flags: u16,
}

#[tauri::command]
async fn list_boards(cli_path: Option<String>) -> Result<Vec<arduino_cli::BoardInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let override_path = cli_path.as_deref().map(PathBuf::from);
        let cli = arduino_cli::ArduinoCli::discover(override_path.as_deref())
            .map_err(|error| error.to_string())?;
        cli.boards().map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("board discovery task failed: {error}"))?
}

/// Tool preparation and repair reject with `ArduinoToolsFailure` (title,
/// explanation, recommended action), never with a raw I/O string.
#[tauri::command]
async fn prepare_arduino_runtime(
    app: tauri::AppHandle,
) -> Result<arduino_runtime::RuntimeStatus, arduino_runtime::ArduinoToolsFailure> {
    tauri::async_runtime::spawn_blocking(move || {
        let status = arduino_runtime::prepare_runtime(&app).map_err(|error| {
            app_log::record(
                "WARN",
                &format!("ARDUINO_TOOLS_PREPARE_FAIL detail={error}"),
            );
            error.failure()
        })?;
        probe_tool_versions();
        Ok(status)
    })
    .await
    .map_err(|error| {
        arduino_runtime::ArduinoToolsFailure::task_failure(format!(
            "Arduino tool preparation task failed: {error}"
        ))
    })?
}

/// Probe the tool versions once per run, now that the tools exist.
/// Recordings reuse them; a failure only makes provenance unavailable.
fn probe_tool_versions() {
    if let Err(error) = arduino_cli::cached_tool_versions() {
        app_log::record("WARN", &format!("TOOL_VERSIONS_UNAVAILABLE detail={error}"));
    }
}

/// Repair Arduino tools: extracts the included offline tools again (it never
/// downloads anything). Allowed in both student and instructor mode.
#[tauri::command]
async fn repair_arduino_runtime(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<arduino_runtime::RuntimeStatus, arduino_runtime::ArduinoToolsFailure> {
    let session = state.session.clone();
    tauri::async_runtime::spawn_blocking(move || repair_tools(&app, &session))
        .await
        .map_err(|error| {
            arduino_runtime::ArduinoToolsFailure::task_failure(format!(
                "Arduino tool repair task failed: {error}"
            ))
        })?
}

fn repair_tools(
    app: &tauri::AppHandle,
    session: &session::SessionController,
) -> Result<arduino_runtime::RuntimeStatus, arduino_runtime::ArduinoToolsFailure> {
    // Start, Verify, Retry connection, and Restore all run the tools being
    // replaced and all claim this lease, so none can run during the repair
    // (and a repair waits for none of them: it is refused instead).
    let _lease = session
        .claim_serial(SerialOwner::ToolRepair)
        .map_err(|error| arduino_runtime::ArduinoToolsFailure::busy(&error.to_string()))?;
    app_log::record("INFO", "ARDUINO_TOOLS_REPAIR_BEGIN");
    let status = arduino_runtime::repair_runtime(app).map_err(|error| {
        app_log::record("WARN", &format!("ARDUINO_TOOLS_REPAIR_FAIL detail={error}"));
        error.failure()
    })?;
    probe_tool_versions();
    app_log::record("INFO", "ARDUINO_TOOLS_REPAIR_OK");
    Ok(status)
}

#[tauri::command]
fn get_project_folder() -> Result<project_paths::ProjectFolderSettings, String> {
    project_paths::load_project_folder()
}

#[tauri::command]
fn set_project_folder(
    project_folder: String,
) -> Result<project_paths::ProjectFolderSettings, String> {
    project_paths::save_project_folder(&project_folder)
}

/// Browser-side chart failures must never influence the acquisition controller.
/// Retain a bounded diagnostic event so an instructor can distinguish a display
/// issue from a serial/protocol session failure.
#[tauri::command]
fn record_frontend_plot_error(stage: String, detail: String) {
    let stage = stage.replace(['\r', '\n'], " ");
    let detail: String = detail
        .replace(['\r', '\n'], " ")
        .chars()
        .take(1_000)
        .collect();
    app_log::record(
        "WARN",
        &format!("PLOT_RENDER_FAIL stage={stage} detail={detail}"),
    );
}

#[tauri::command]
async fn firmware_environment(
    state: tauri::State<'_, AppState>,
) -> Result<firmware_workflow::FirmwareEnvironmentStatus, String> {
    let firmware = state.firmware.clone();
    tauri::async_runtime::spawn_blocking(move || firmware.environment())
        .await
        .map_err(|error| format!("firmware environment task failed: {error}"))
}

/// Firmware commands return `FirmwareFailure` itself as the error, so the
/// frontend receives its title, explanation, and recommended action.
#[tauri::command]
async fn restore_wvu_reference_firmware(
    state: tauri::State<'_, AppState>,
    request: firmware_workflow::RestoreReferenceRequest,
) -> Result<firmware_workflow::FirmwareJobStatus, firmware_workflow::FirmwareFailure> {
    let firmware = state.firmware.clone();
    // Board discovery runs Arduino CLI, so it stays off the main thread.
    tauri::async_runtime::spawn_blocking(move || firmware.start_restore_reference(request))
        .await
        .map_err(|error| {
            firmware_workflow::FirmwareFailure::task_failure(format!(
                "firmware restore task failed: {error}"
            ))
        })?
}

#[tauri::command]
fn cancel_firmware_job(
    state: tauri::State<'_, AppState>,
) -> Result<firmware_workflow::FirmwareWorkflowStatus, firmware_workflow::FirmwareFailure> {
    state.firmware.cancel_active_job()
}

#[tauri::command]
fn get_firmware_workflow_status(
    state: tauri::State<'_, AppState>,
) -> Result<firmware_workflow::FirmwareWorkflowStatus, firmware_workflow::FirmwareFailure> {
    state.firmware.status()
}

#[tauri::command]
async fn verify_wvu_reference_firmware(
    state: tauri::State<'_, AppState>,
    port: String,
) -> Result<firmware_workflow::FirmwareVerification, firmware_workflow::FirmwareFailure> {
    let firmware = state.firmware.clone();
    tauri::async_runtime::spawn_blocking(move || firmware.verify_existing_reference(port))
        .await
        .map_err(|error| {
            firmware_workflow::FirmwareFailure::task_failure(format!(
                "firmware verification task failed: {error}"
            ))
        })?
}

#[tauri::command]
fn list_acquisition_profiles(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<profiles::AcquisitionProfile>, String> {
    state.profiles.list().map_err(|error| error.to_string())
}

#[tauri::command]
fn list_instructor_labs(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<profiles::LabListEntry>, String> {
    state.profiles.list_all().map_err(|error| error.to_string())
}

#[tauri::command]
fn begin_lab_edit(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<profiles::AcquisitionProfile, String> {
    state
        .profiles
        .begin_lab_edit(&profile_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn duplicate_lab(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    lab_id: String,
) -> Result<profiles::AcquisitionProfile, String> {
    state
        .profiles
        .duplicate_lab(&profile_id, &lab_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn create_blank_simultaneous_lab(
    state: tauri::State<'_, AppState>,
    lab_id: String,
) -> Result<profiles::AcquisitionProfile, String> {
    state
        .profiles
        .create_blank_simultaneous_lab(&lab_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_lab_draft(
    state: tauri::State<'_, AppState>,
    draft: profiles::AcquisitionProfile,
    base_version: Option<String>,
    request_id: String,
) -> Result<profiles::AcquisitionProfile, String> {
    state
        .profiles
        .save_lab_draft(draft, base_version, request_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn restore_retired_lab(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    profile_version: String,
) -> Result<profiles::AcquisitionProfile, String> {
    state
        .profiles
        .restore_retired(&profile_id, &profile_version)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn restore_course_default_lab(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<profiles::AcquisitionProfile, String> {
    state
        .profiles
        .restore_course_default(&profile_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn set_active_lab_version(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    profile_version: String,
) -> Result<profiles::AcquisitionProfile, String> {
    state
        .profiles
        .set_active_version(&profile_id, &profile_version)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn reset_local_lab_customizations(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state
        .profiles
        .reset_local_customizations()
        .map_err(|error| error.to_string())
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
fn list_calibrations(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    channel_id: String,
) -> Result<Vec<calibration::CalibrationPreset>, String> {
    state
        .calibrations
        .list(&profile_id, &channel_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_calibration(
    state: tauri::State<'_, AppState>,
    calibration: calibration::CalibrationPreset,
) -> Result<calibration::CalibrationPreset, String> {
    state
        .calibrations
        .save(calibration)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_calibration(
    state: tauri::State<'_, AppState>,
    calibration_id: String,
) -> Result<(), String> {
    state
        .calibrations
        .delete(&calibration_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn fit_xgzp_calibration(
    request: calibration::XgzpFitRequest,
) -> Result<calibration::LinearFit, String> {
    calibration::fit_xgzp_from_recording(&request).map_err(|error| error.to_string())
}

#[tauri::command]
fn fit_manual_linear_calibration(
    points: Vec<calibration::CalibrationPoint>,
) -> Result<calibration::LinearFit, String> {
    calibration::fit_linear(&points).map_err(|error| error.to_string())
}

fn checked_recording_calibration(
    profile: &profiles::AcquisitionProfile,
    calibration: calibration::RecordingCalibration,
) -> Result<calibration::RecordingCalibration, String> {
    calibration.validate().map_err(|error| error.to_string())?;
    for preset in &calibration.active_calibrations {
        if preset.profile_id != profile.profile_id {
            return Err("a calibration can be used only with the profile that created it".into());
        }
        if !profile
            .acquisition
            .resolved_channels()
            .iter()
            .any(|channel| channel.id == preset.channel_id)
        {
            return Err(
                "a calibration can be used only with a channel in the selected profile".into(),
            );
        }
    }
    Ok(calibration)
}

/// Projected BMEG + CSV size and the free space a timed recording needs, so
/// it can be shown before Start. It reads the lab catalog only: no port, file,
/// or acquisition command is touched.
#[tauri::command]
fn estimate_recording_storage(
    state: tauri::State<'_, AppState>,
    request: EstimateRecordingStorageRequest,
) -> Result<session::RecordingStorageEstimate, String> {
    let EstimateRecordingStorageRequest {
        profile_id,
        duration,
        calibration,
    } = request;
    let profile = state
        .profiles
        .get_locked(&profile_id)
        .map_err(|error| error.to_string())?;
    let calibration = checked_recording_calibration(&profile, calibration.unwrap_or_default())?;
    Ok(session::estimate_recording_storage(
        &profile.snapshot(false),
        &calibration,
        &duration,
    ))
}

#[tauri::command]
async fn start_profile_simulator_recording(
    state: tauri::State<'_, AppState>,
    request: StartProfileSimulatorRequest,
) -> Result<session::SessionStatus, StartRecordingFailure> {
    let session = state.session.clone();
    let profiles = state.profiles.clone();
    tauri::async_runtime::spawn_blocking(move || {
        start_simulator_recording(&session, &profiles, request)
    })
    .await
    .map_err(start_task_failure)?
}

fn start_simulator_recording(
    session: &session::SessionController,
    profiles: &profiles::ProfileStore,
    request: StartProfileSimulatorRequest,
) -> Result<session::SessionStatus, StartRecordingFailure> {
    let StartProfileSimulatorRequest {
        project_folder,
        output_folder,
        duration,
        profile_id,
        bench_notice_acknowledged,
        recording_note,
        calibration,
    } = request;
    // The session (and the board) are claimed first, so a Verify, Retry, or
    // restore cannot change the session state while this start is prepared.
    let lease = session
        .claim_serial(SerialOwner::Session)
        .map_err(StartRecordingFailure::busy)?;
    let destination = project_paths::resolve_recording_destination(&project_folder, &output_folder)
        .map_err(|error| {
            StartRecordingFailure::new(
                "VALIDATE_PATHS",
                "recording_folder",
                "The recording folder is not writable. Choose another Project or Output folder before recording.",
                error,
            )
        })?;
    let profile = profiles.get_locked(&profile_id).map_err(|error| {
        StartRecordingFailure::new(
            "LOAD_LAB",
            "lab_unavailable",
            "The selected lab is not available. Choose the assigned lab and try again.",
            error,
        )
    })?;
    let calibration = checked_recording_calibration(&profile, calibration.unwrap_or_default())
        .map_err(|error| {
            StartRecordingFailure::new(
                "CHECK_CALIBRATION",
                "calibration",
                "The selected calibration cannot be used with this lab. Choose another calibration and try again.",
                error,
            )
        })?;
    session.prepare_for_new_recording().map_err(|error| {
        StartRecordingFailure::new(
            "PREPARE_SESSION",
            "session_busy",
            "The Arduino is busy. Wait for the current operation to finish and try again.",
            error,
        )
    })?;
    session
        .start_simulator_leased(
            lease,
            profile.snapshot(bench_notice_acknowledged),
            session::RecordingStartOptions {
                duration,
                output_dir: destination.effective_folder,
                calibration,
                recording_path_context: Some(session::RecordingPathContext {
                    project_folder: destination.project_folder.to_string_lossy().into_owned(),
                    output_folder: destination.output_folder,
                }),
                recording_context: session::RecordingContext { recording_note },
            },
        )
        .map_err(StartRecordingFailure::session_start)
}

#[tauri::command]
async fn start_profile_hardware_recording(
    state: tauri::State<'_, AppState>,
    request: StartProfileHardwareRequest,
) -> Result<session::SessionStatus, StartRecordingFailure> {
    let session = state.session.clone();
    let firmware = state.firmware.clone();
    let profiles = state.profiles.clone();
    // Board discovery and session preparation run Arduino CLI and wait on
    // the previous worker, so they never block the window.
    tauri::async_runtime::spawn_blocking(move || {
        start_hardware_recording(&session, &firmware, &profiles, request)
    })
    .await
    .map_err(start_task_failure)?
}

fn start_hardware_recording(
    session: &session::SessionController,
    firmware: &firmware_workflow::FirmwareWorkflow,
    profiles: &profiles::ProfileStore,
    request: StartProfileHardwareRequest,
) -> Result<session::SessionStatus, StartRecordingFailure> {
    let StartProfileHardwareRequest {
        port,
        project_folder,
        output_folder,
        duration,
        profile_id,
        bench_notice_acknowledged,
        recording_note,
        calibration,
    } = request;
    app_log::record(
        "INFO",
        &format!("START_REQUEST port={port} lab={profile_id}"),
    );
    // Claimed before any check: no restore, Verify, or Retry can run between
    // the firmware check and the worker opening the port. The worker keeps the
    // lease until the recording is saved.
    let lease = session
        .claim_serial(SerialOwner::Session)
        .map_err(StartRecordingFailure::busy)?;
    app_log::record("INFO", "VALIDATE_PATHS_BEGIN");
    let destination = project_paths::resolve_recording_destination(&project_folder, &output_folder)
        .map_err(|error| {
            StartRecordingFailure::new(
                "VALIDATE_PATHS",
                "recording_folder",
                "The recording folder is not writable. Choose another Project or Output folder before recording.",
                error,
            )
        })?;
    app_log::record(
        "INFO",
        &format!(
            "VALIDATE_PATHS_OK destination={}",
            destination.effective_folder.display()
        ),
    );
    let profile = profiles.get_locked(&profile_id).map_err(|error| {
        StartRecordingFailure::new(
            "LOAD_LAB",
            "lab_unavailable",
            "The selected lab is not available. Choose the assigned lab and try again.",
            error,
        )
    })?;
    let firmware_allowed = firmware.is_acquisition_allowed(&port).map_err(|error| {
        StartRecordingFailure::new(
            "CHECK_FIRMWARE",
            "firmware_status",
            "WVU firmware is not ready. Verify or restore the firmware before recording.",
            error,
        )
    })?;
    if !firmware_allowed {
        return Err(StartRecordingFailure::new(
            "CHECK_FIRMWARE",
            "firmware_not_ready",
            "WVU firmware is not ready on this board. Select Verify Firmware, then try again.",
            "compatible WVU firmware was not verified on the selected port",
        ));
    }
    if !serialport::available_ports()
        .map_err(|error| {
            StartRecordingFailure::new(
                "SERIAL_ENUMERATE",
                "serial_enumeration",
                "The Arduino could not be opened. Reconnect it or click Refresh Board, then try again.",
                error,
            )
        })?
        .iter()
        .any(|candidate| candidate.port_name.eq_ignore_ascii_case(&port))
    {
        return Err(StartRecordingFailure::new(
            "SERIAL_ENUMERATE",
            "selected_port_missing",
            "The selected Arduino is no longer available. Reconnect it, then click Refresh Board.",
            format!("selected port {port} was not present in the operating-system serial-port list"),
        ));
    }
    app_log::record("INFO", "BOARD_DISCOVERY_BEGIN");
    let cli = arduino_cli::ArduinoCli::discover(None).map_err(|error| {
        StartRecordingFailure::new(
            "BOARD_DISCOVERY",
            "arduino_tools",
            "Arduino tools need attention. Select Repair Arduino tools, then try again.",
            error,
        )
    })?;
    // This is the only board scan for a Start; its serial number is reused as
    // recording provenance.
    let board = cli
        .boards()
        .map_err(|error| {
            StartRecordingFailure::new(
                "BOARD_DISCOVERY",
                "board_scan",
                "The Arduino could not be confirmed. Reconnect it, then click Refresh Board.",
                error,
            )
        })?
        .into_iter()
        .find(|board| board.port.eq_ignore_ascii_case(&port))
        .ok_or_else(|| {
            StartRecordingFailure::new(
                "BOARD_DISCOVERY",
                "unsupported_board",
                "The selected board is no longer available. Reconnect the UNO R4 WiFi, then click Refresh Board.",
                format!("selected port {port} was not reported as an Arduino UNO R4 WiFi"),
            )
        })?;
    app_log::record("INFO", "BOARD_DISCOVERY_OK");
    let calibration = checked_recording_calibration(&profile, calibration.unwrap_or_default())
        .map_err(|error| {
            StartRecordingFailure::new(
                "CHECK_CALIBRATION",
                "calibration",
                "The selected calibration cannot be used with this lab. Choose another calibration and try again.",
                error,
            )
        })?;
    session.prepare_for_new_recording().map_err(|error| {
        StartRecordingFailure::new(
            "PREPARE_SESSION",
            "session_busy",
            "The Arduino is busy. Wait for the current operation to finish and try again.",
            error,
        )
    })?;
    app_log::record("INFO", "SERIAL_OPEN_BEGIN");
    session
        .start_serial_leased(
            lease,
            profile.snapshot(bench_notice_acknowledged),
            port,
            session::RecordingStartOptions {
                duration,
                output_dir: destination.effective_folder,
                calibration,
                recording_path_context: Some(session::RecordingPathContext {
                    project_folder: destination.project_folder.to_string_lossy().into_owned(),
                    output_folder: destination.output_folder,
                }),
                recording_context: session::RecordingContext { recording_note },
            },
            board.serial_number,
        )
        .map_err(StartRecordingFailure::session_start)
}

/// Retry connection: a fresh protocol handshake on a discovered UNO R4 WiFi.
/// It never resets the board at 1200 bps and never uploads firmware.
#[tauri::command]
async fn retry_hardware_handshake(
    state: tauri::State<'_, AppState>,
    port: String,
) -> Result<session::HandshakeRetryResult, String> {
    let session = state.session.clone();
    tauri::async_runtime::spawn_blocking(move || retry_connection(&session, &port))
        .await
        .map_err(|error| format!("connection retry task failed: {error}"))?
}

fn retry_connection(
    session: &session::SessionController,
    port: &str,
) -> Result<session::HandshakeRetryResult, String> {
    if port.trim().is_empty() || port.contains('\0') {
        return Err("Select a detected Arduino UNO R4 WiFi, then try again.".into());
    }
    // Released when this function returns, on success, error, or panic.
    let _lease = session
        .claim_serial(SerialOwner::Retry)
        .map_err(|error| error.to_string())?;
    let cli = arduino_cli::ArduinoCli::discover(None).map_err(|error| error.to_string())?;
    let board = cli
        .boards()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|board| board.port.eq_ignore_ascii_case(port))
        .ok_or_else(|| {
            "The selected board is no longer detected. Select Refresh Board, then try again."
                .to_string()
        })?;
    session
        .retry_handshake(&board.port, session::FaultOrigin::Connection)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn stop_recording(state: tauri::State<'_, AppState>) -> Result<session::SessionStatus, String> {
    state
        .session
        .request_stop()
        .map_err(|error| error.to_string())
}

/// Stores a short timestamped user annotation with the active recording. The worker owns the
/// raw stream; this command only appends bounded metadata and never emits a sample event.
#[tauri::command]
fn add_recording_marker(
    state: tauri::State<'_, AppState>,
    label: String,
) -> Result<recording::RecordingMarker, String> {
    state
        .session
        .add_marker(label)
        .map_err(|error| error.to_string())
}

/// Requests the stop and returns at once; the recording finishes saving on
/// its worker and polling shows the final state. An idle fault is cleared.
#[tauri::command]
async fn disconnect_session(
    state: tauri::State<'_, AppState>,
) -> Result<session::SessionStatus, String> {
    let session = state.session.clone();
    tauri::async_runtime::spawn_blocking(move || session.request_disconnect())
        .await
        .map_err(|error| format!("disconnect task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_session_status(state: tauri::State<'_, AppState>) -> Result<session::SessionStatus, String> {
    state.session.status().map_err(|error| error.to_string())
}

/// Bounded, display-only snapshot for a 20–30 Hz polling UI. The window and
/// render-point budget affect neither recording nor the raw BMEG writer.
#[tauri::command]
fn get_recent_display_data(
    state: tauri::State<'_, AppState>,
    window_seconds: Option<f64>,
    max_points: Option<usize>,
) -> Result<Vec<RecentPoint>, String> {
    state
        .session
        .recent_display_samples(
            window_seconds.unwrap_or(session::DEFAULT_DISPLAY_WINDOW_SECONDS),
            max_points.unwrap_or(session::MAX_DISPLAY_RENDER_POINTS),
        )
        .map_err(|error| error.to_string())
        .map(|samples| {
            samples
                .into_iter()
                .map(|sample| RecentPoint {
                    sequence: sample.sequence,
                    timestamp_us: sample.timestamp_us,
                    values: sample.counts,
                    status_flags: sample.status_flags,
                })
                .collect()
        })
}

#[derive(Debug, Eq, PartialEq)]
enum CloseDecision {
    /// Nothing needs to finish first.
    Allow,
    /// A firmware restore is running: the window stays open.
    RefuseDuringFirmwareJob,
    /// This close is already saving the recording: wait for it.
    AlreadySaving,
    /// Stop and save the recording, then close.
    SaveRecordingThenClose,
}

fn close_decision(firmware_job_active: bool, close_state: u8, recording: bool) -> CloseDecision {
    if firmware_job_active {
        return CloseDecision::RefuseDuringFirmwareJob;
    }
    match close_state {
        CLOSE_READY => CloseDecision::Allow,
        CLOSE_SAVING => CloseDecision::AlreadySaving,
        _ if recording => CloseDecision::SaveRecordingThenClose,
        _ => CloseDecision::Allow,
    }
}

fn notify_close_blocked(window: &tauri::Window, reason: &'static str, message: &'static str) {
    let _ = window.emit(CLOSE_BLOCKED_EVENT, CloseBlocked { reason, message });
}

/// Close-request policy.
///
/// A firmware restore is never interrupted by closing the window: its
/// Arduino tools would be ended and the board could be left part-way written.
/// The page is told why the window stayed open. During a recording the
/// recording is stopped and saved before the window closes, by exactly one
/// thread no matter how many times close is clicked.
fn handle_close_request(window: &tauri::Window, api: &tauri::CloseRequestApi) {
    let state = window.state::<AppState>();
    let decision = close_decision(
        state.firmware.has_active_job(),
        state.closing.load(Ordering::Acquire),
        matches!(state.session.is_recording(), Ok(true)),
    );
    match decision {
        CloseDecision::Allow => {}
        CloseDecision::RefuseDuringFirmwareJob => {
            api.prevent_close();
            // A save that finished just before a restore began no longer
            // authorizes closing; the next request is evaluated afresh.
            state.closing.store(CLOSE_IDLE, Ordering::Release);
            notify_close_blocked(
                window,
                "firmware_job",
                "WVU Firmware is being restored. The window stays open until the restore finishes; do not unplug the board.",
            );
        }
        CloseDecision::AlreadySaving => {
            api.prevent_close();
            notify_close_blocked(
                window,
                "saving_recording",
                "The recording is being saved. The window will close when saving finishes.",
            );
        }
        CloseDecision::SaveRecordingThenClose => {
            // Preventing close and finalizing first avoids abandoning a writer.
            api.prevent_close();
            if state
                .closing
                .compare_exchange(
                    CLOSE_IDLE,
                    CLOSE_SAVING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
            {
                return;
            }
            notify_close_blocked(
                window,
                "saving_recording",
                "The recording is being saved. The window will close when saving finishes.",
            );
            let session = state.session.clone();
            let closing = Arc::clone(&state.closing);
            let closing_window = window.clone();
            std::thread::spawn(move || {
                let _ = session.request_stop_with_reason(recording::StopReason::ApplicationClose);
                let _ = session.wait_for_worker();
                // Normally already final once the worker is joined; this
                // covers a worker handle that another caller was joining.
                let settle_started = Instant::now();
                while matches!(session.is_recording(), Ok(true))
                    && settle_started.elapsed() < CLOSE_SETTLE_LIMIT
                {
                    std::thread::sleep(Duration::from_millis(50));
                }
                closing.store(CLOSE_READY, Ordering::Release);
                let _ = closing_window.close();
            });
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // The acquisition controller is intentionally shared with the firmware workflow:
    // an upload first releases this one serial owner before Arduino CLI opens the port.
    let session = session::SessionController::default();
    let firmware = firmware_workflow::FirmwareWorkflow::new(session.clone());
    let profiles = profiles::ProfileStore::default();
    let calibrations = calibration::CalibrationStore::default();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            session,
            firmware,
            profiles,
            calibrations,
            closing: Arc::new(AtomicU8::new(CLOSE_IDLE)),
        })
        .invoke_handler(tauri::generate_handler![
            prepare_arduino_runtime,
            repair_arduino_runtime,
            get_project_folder,
            set_project_folder,
            record_frontend_plot_error,
            list_boards,
            firmware_environment,
            restore_wvu_reference_firmware,
            cancel_firmware_job,
            get_firmware_workflow_status,
            verify_wvu_reference_firmware,
            list_acquisition_profiles,
            list_instructor_labs,
            begin_lab_edit,
            duplicate_lab,
            create_blank_simultaneous_lab,
            save_lab_draft,
            restore_retired_lab,
            restore_course_default_lab,
            set_active_lab_version,
            reset_local_lab_customizations,
            get_profile_mode,
            set_profile_mode,
            retire_profile,
            export_profile_package,
            import_profile_package,
            list_calibrations,
            save_calibration,
            delete_calibration,
            fit_xgzp_calibration,
            fit_manual_linear_calibration,
            estimate_recording_storage,
            start_profile_simulator_recording,
            start_profile_hardware_recording,
            retry_hardware_handshake,
            stop_recording,
            add_recording_marker,
            disconnect_session,
            get_session_status,
            get_recent_display_data
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                handle_close_request(window, api);
            }
        })
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| {
            app_log::record_startup_failure(&error.to_string());
            eprintln!("WVU Bioinstrumentation Studio failed to start: {error}");
            show_startup_failure_dialog(&error.to_string());
        });
}

#[cfg(windows)]
fn show_startup_failure_dialog(error: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let message = format!(
        "WVU Bioinstrumentation Studio could not start.\n\n{error}\n\nA diagnostic entry was written to %LOCALAPPDATA%\\WVU Bioinstrumentation Studio\\logs\\application.log."
    );
    let to_wide = |text: &str| {
        OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let title = to_wide("WVU Bioinstrumentation Studio");
    let message = to_wide(&message);
    // This is only reached after Tauri failed before a window could be
    // created, so a native message box is the only actionable UI available.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn show_startup_failure_dialog(_error: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_never_interrupts_a_firmware_restore() {
        for close_state in [CLOSE_IDLE, CLOSE_SAVING, CLOSE_READY] {
            for recording in [false, true] {
                assert_eq!(
                    close_decision(true, close_state, recording),
                    CloseDecision::RefuseDuringFirmwareJob
                );
            }
        }
    }

    #[test]
    fn closing_during_a_recording_saves_it_once_then_closes() {
        assert_eq!(
            close_decision(false, CLOSE_IDLE, false),
            CloseDecision::Allow
        );
        assert_eq!(
            close_decision(false, CLOSE_IDLE, true),
            CloseDecision::SaveRecordingThenClose
        );
        // Further clicks while saving start no second save thread.
        assert_eq!(
            close_decision(false, CLOSE_SAVING, true),
            CloseDecision::AlreadySaving
        );
        assert_eq!(
            close_decision(false, CLOSE_SAVING, false),
            CloseDecision::AlreadySaving
        );
        // The save thread's own close request goes through.
        assert_eq!(
            close_decision(false, CLOSE_READY, false),
            CloseDecision::Allow
        );
    }
}
