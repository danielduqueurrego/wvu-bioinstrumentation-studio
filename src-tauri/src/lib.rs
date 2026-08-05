pub mod acquisition;
pub mod arduino_cli;
pub mod profiles;
pub mod protocol;
pub mod recording;
pub mod session;

use serde::Serialize;
use std::path::PathBuf;

struct AppState {
    /// Exactly one controller owns the Phase 1 session lifecycle. It has no frontend serial handle.
    session: session::SessionController,
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

/// Combined Phase 1 connect/handshake/configure/start command. The worker owns transport I/O.
#[tauri::command]
fn start_simulator_recording(
    state: tauri::State<'_, AppState>,
    output_directory: String,
    seconds: u32,
) -> Result<session::SessionStatus, String> {
    checked_output_directory(&output_directory)?;
    state
        .session
        .start_simulator(seconds, PathBuf::from(output_directory))
        .map_err(|error| error.to_string())
}

/// Combined Phase 1 connect/handshake/configure/start command for a discovered UNO R4 WiFi.
#[tauri::command]
fn start_hardware_recording(
    state: tauri::State<'_, AppState>,
    port: String,
    output_directory: String,
    seconds: u32,
) -> Result<session::SessionStatus, String> {
    checked_output_directory(&output_directory)?;
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
        .start_serial(port, seconds, PathBuf::from(output_directory))
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            session: session::SessionController::default(),
        })
        .invoke_handler(tauri::generate_handler![
            list_boards,
            list_serial_ports,
            arduino_cli_version,
            start_simulator_recording,
            start_hardware_recording,
            stop_recording,
            disconnect_session,
            get_session_status,
            get_recent_display_data,
            export_session_csv
        ])
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
