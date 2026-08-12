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
pub mod session;

use serde::Serialize;
use std::path::PathBuf;
use tauri::{Manager, WindowEvent};

struct AppState {
    /// Exactly one controller owns the acquisition-session lifecycle. It has no frontend serial handle.
    session: session::SessionController,
    /// One firmware workflow coordinates CLI jobs with the same serial-session boundary.
    firmware: firmware_workflow::FirmwareWorkflow,
    /// Lab definitions are independent of firmware restoration and bind every recording.
    profiles: profiles::ProfileStore,
    /// Student-facing local calibration presets. They never modify a locked profile.
    calibrations: calibration::CalibrationStore,
}

#[derive(serde::Deserialize)]
struct StartProfileHardwareRequest {
    port: String,
    project_folder: String,
    output_folder: String,
    duration: recording::RecordingDuration,
    profile_id: String,
    bench_notice_acknowledged: bool,
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

#[tauri::command]
async fn prepare_arduino_runtime(
    app: tauri::AppHandle,
) -> Result<arduino_runtime::RuntimeStatus, String> {
    tauri::async_runtime::spawn_blocking(move || arduino_runtime::prepare_runtime(&app))
        .await
        .map_err(|error| format!("Arduino tool preparation task failed: {error}"))?
        .map_err(|error| error.to_string())
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

fn firmware_error<T>(result: Result<T, firmware_workflow::FirmwareFailure>) -> Result<T, String> {
    result.map_err(|failure| {
        serde_json::to_string(&failure)
            .unwrap_or_else(|error| format!("firmware workflow error: {error}"))
    })
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
async fn verify_wvu_reference_firmware(
    state: tauri::State<'_, AppState>,
    port: String,
) -> Result<firmware_workflow::FirmwareVerification, String> {
    let firmware = state.firmware.clone();
    tauri::async_runtime::spawn_blocking(move || firmware.verify_existing_reference(port))
        .await
        .map_err(|error| format!("firmware verification task failed: {error}"))
        .and_then(firmware_error)
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

#[tauri::command]
fn start_profile_simulator_recording(
    state: tauri::State<'_, AppState>,
    project_folder: String,
    output_folder: String,
    duration: recording::RecordingDuration,
    profile_id: String,
    bench_notice_acknowledged: bool,
    calibration: Option<calibration::RecordingCalibration>,
) -> Result<session::SessionStatus, String> {
    let destination =
        project_paths::resolve_recording_destination(&project_folder, &output_folder)?;
    let profile = state
        .profiles
        .get_locked(&profile_id)
        .map_err(|error| error.to_string())?;
    let calibration = checked_recording_calibration(&profile, calibration.unwrap_or_default())?;
    state
        .session
        .prepare_for_new_recording()
        .map_err(|error| error.to_string())?;
    state
        .session
        .start_simulator_with_profile_calibration_and_path_context(
            profile.snapshot(bench_notice_acknowledged),
            duration,
            destination.effective_folder,
            calibration,
            Some(session::RecordingPathContext {
                project_folder: destination.project_folder.to_string_lossy().into_owned(),
                output_folder: destination.output_folder,
            }),
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn start_profile_hardware_recording(
    state: tauri::State<'_, AppState>,
    request: StartProfileHardwareRequest,
) -> Result<session::SessionStatus, StartRecordingFailure> {
    let StartProfileHardwareRequest {
        port,
        project_folder,
        output_folder,
        duration,
        profile_id,
        bench_notice_acknowledged,
        calibration,
    } = request;
    app_log::record(
        "INFO",
        &format!("START_REQUEST port={port} lab={profile_id}"),
    );
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
    let profile = state.profiles.get_locked(&profile_id).map_err(|error| {
        StartRecordingFailure::new(
            "LOAD_LAB",
            "lab_unavailable",
            "The selected lab is not available. Choose the assigned lab and try again.",
            error,
        )
    })?;
    let firmware_allowed =
        firmware_error(state.firmware.is_acquisition_allowed(&port)).map_err(|error| {
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
            "WVU firmware is not ready. Verify or restore the firmware before recording.",
            "firmware workflow did not report a compatible WVU firmware for the selected port",
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
            "Arduino tools need attention. Click Refresh Board, then try again.",
            error,
        )
    })?;
    if !cli
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
        .any(|board| board.port.eq_ignore_ascii_case(&port))
    {
        return Err(StartRecordingFailure::new(
            "BOARD_DISCOVERY",
            "unsupported_board",
            "The selected board is no longer available. Reconnect the UNO R4 WiFi, then click Refresh Board.",
            format!("selected port {port} was not reported as an Arduino UNO R4 WiFi"),
        ));
    }
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
    state.session.prepare_for_new_recording().map_err(|error| {
        StartRecordingFailure::new(
            "PREPARE_SESSION",
            "session_busy",
            "The Arduino is busy. Wait for the current operation to finish and try again.",
            error,
        )
    })?;
    app_log::record("INFO", "SERIAL_OPEN_BEGIN");
    state
        .session
        .start_serial_with_profile_calibration_and_path_context(
            profile.snapshot(bench_notice_acknowledged),
            port,
            duration,
            destination.effective_folder,
            calibration,
            Some(session::RecordingPathContext {
                project_folder: destination.project_folder.to_string_lossy().into_owned(),
                output_folder: destination.output_folder,
            }),
        )
        .map_err(|error| {
            StartRecordingFailure::new(
                "INITIALIZE_SESSION",
                "session_start",
                "The recording could not be prepared. Try again. If it continues, open Advanced details and share the information with your instructor.",
                error,
            )
        })
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
        })
        .invoke_handler(tauri::generate_handler![
            prepare_arduino_runtime,
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
            start_profile_simulator_recording,
            start_profile_hardware_recording,
            reset_board_and_retry,
            retry_hardware_handshake,
            stop_recording,
            add_recording_marker,
            disconnect_session,
            get_session_status,
            get_recent_display_data
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let session = window.state::<AppState>().session.clone();
                if matches!(session.is_recording(), Ok(true)) {
                    // Preventing close and finalizing first avoids abandoning a writer.
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
