//! Acceptance harness for the same production SessionController used by Tauri commands.
use std::{
    env,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use wvu_bioinstrumentation_studio_lib::{arduino_cli::ArduinoCli, session::SessionController};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = env::args()
        .nth(1)
        .unwrap_or_else(|| "simulator".to_string());
    let seconds = env::args()
        .nth(2)
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(5);
    let session = SessionController::default();
    let output_dir = PathBuf::from("recordings");
    if mode == "hardware" {
        let cli = ArduinoCli::discover(None)?;
        let board = cli
            .boards()?
            .into_iter()
            .next()
            .ok_or("no UNO R4 WiFi discovered")?;
        session.start_serial(board.port, seconds, output_dir)?;
    } else {
        session.start_simulator(seconds, output_dir)?;
    }
    let deadline = Instant::now() + Duration::from_secs(u64::from(seconds) + 20);
    loop {
        let status = session.status()?;
        if let Some(summary) = status.last_summary {
            println!("{}", serde_json::to_string_pretty(&summary)?);
            return if summary.error.is_some() {
                Err("session faulted".into())
            } else {
                Ok(())
            };
        }
        if Instant::now() >= deadline {
            session.request_stop()?;
            session.wait_for_worker()?;
            return Err("timed out waiting for session worker".into());
        }
        thread::sleep(Duration::from_millis(40));
    }
}
