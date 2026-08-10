pub mod acquisition;
pub mod arduino_cli;
pub mod calibration;
pub mod firmware_workflow;
pub mod firmware_workspace;
pub mod profiles;
pub mod protocol;
pub mod recording;
pub mod session;

use serde::Serialize;
use std::path::PathBuf;
use tauri::{Manager, WindowEvent};

struct AppState {
    /// Exactly one controller owns the Phase 1 session lifecycle. It has no frontend serial handle.
    session: session::SessionController,
    /// One firmware workflow coordinates CLI jobs with the same serial-session boundary.
    firmware: firmware_workflow::FirmwareWorkflow,
    /// Profiles are independent of firmware editing but bind every Phase 3A recording.
    profiles: profiles::ProfileStore,
    /// Student-facing local calibration presets. They never modify a locked profile.
    calibrations: calibration::CalibrationStore,
}

#[derive(Serialize)]
struct RecentPoint {
    sequence: u32,
    timestamp_us: u64,
    values: Vec<u16>,
    status_flags: u16,
}

#[derive(Serialize)]
struct SerialPortInfo {
    port: String,
    kind: String,
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
async fn firmware_environment(
    state: tauri::State<'_, AppState>,
) -> Result<firmware_workflow::FirmwareEnvironmentStatus, String> {
    let firmware = state.firmware.clone();
    tauri::async_runtime::spawn_blocking(move || firmware.environment())
        .await
        .map_err(|error| format!("firmware environment task failed: {error}"))
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
fn update_profile_draft_acquisition(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    profile_version: String,
    acquisition: profiles::AcquisitionSettings,
) -> Result<profiles::AcquisitionProfile, String> {
    state
        .profiles
        .update_draft_acquisition(&profile_id, &profile_version, acquisition)
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
    calibration: Option<calibration::RecordingCalibration>,
) -> Result<session::SessionStatus, String> {
    checked_output_directory(&output_directory)?;
    let profile = state
        .profiles
        .get_locked(&profile_id)
        .map_err(|error| error.to_string())?;
    let calibration = checked_recording_calibration(&profile, calibration.unwrap_or_default())?;
    state
        .session
        .start_simulator_with_profile_and_calibration(
            profile.snapshot(bench_notice_acknowledged),
            duration,
            PathBuf::from(output_directory),
            calibration,
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
    calibration: Option<calibration::RecordingCalibration>,
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
    let calibration = checked_recording_calibration(&profile, calibration.unwrap_or_default())?;
    state
        .session
        .start_serial_with_profile_and_calibration(
            profile.snapshot(bench_notice_acknowledged),
            port,
            duration,
            PathBuf::from(output_directory),
            calibration,
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
                    values: sample.counts,
                    status_flags: sample.status_flags,
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
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            session,
            firmware,
            profiles,
            calibrations,
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
            update_profile_draft_acquisition,
            finalize_profile_draft,
            retire_profile,
            export_profile_package,
            import_profile_package,
            list_calibrations,
            save_calibration,
            delete_calibration,
            fit_xgzp_calibration,
            fit_manual_linear_calibration,
            start_simulator_recording,
            start_hardware_recording,
            start_profile_simulator_recording,
            start_profile_hardware_recording,
            reset_board_and_retry,
            retry_hardware_handshake,
            stop_recording,
            add_recording_marker,
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
    use super::checked_output_directory;

    #[test]
    fn command_input_validation_rejects_empty_or_nul_directories() {
        assert!(checked_output_directory("").is_err());
        assert!(checked_output_directory("recordings\0bad").is_err());
        assert!(checked_output_directory("recordings").is_ok());
    }
}
