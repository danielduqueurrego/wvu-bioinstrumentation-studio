//! Small, dependency-free application event log for recoverable production diagnostics.
//!
//! The normal student UI stays concise.  This file retains bounded, stage-level events
//! that an instructor can inspect when an external device or recording startup fails.
use chrono::Utc;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const APP_DATA_FOLDER: &str = "WVU Bioinstrumentation Studio";
const LOG_FILE: &str = "application.log";
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
const RETAINED_LOG_FILES: usize = 5;

fn log_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|root| {
        PathBuf::from(root)
            .join(APP_DATA_FOLDER)
            .join("logs")
            .join(LOG_FILE)
    })
}

/// Recording diagnostics must never interfere with acquisition.  Log-write failures
/// are deliberately ignored after the attempt has been made.
pub fn record(level: &str, event: &str) {
    let Some(path) = log_path() else {
        return;
    };
    let _ = append_at(&path, level, event);
}

fn append_at(path: &Path, level: &str, event: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    rotate_if_needed(path, event.len() as u64 + 64)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(
        file,
        "{} {} {}",
        Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        level,
        event.replace(['\r', '\n'], " ")
    )
}

/// Keep production diagnostics useful without allowing an unattended app to
/// grow one append-only log forever. Rotation happens before opening the live
/// file so Windows can rename it safely.
fn rotate_if_needed(path: &Path, incoming_bytes: u64) -> std::io::Result<()> {
    let current_bytes = fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if current_bytes.saturating_add(incoming_bytes) <= MAX_LOG_BYTES {
        return Ok(());
    }

    let oldest = path.with_extension(RETAINED_LOG_FILES.to_string());
    if oldest.exists() {
        fs::remove_file(&oldest)?;
    }
    for index in (1..RETAINED_LOG_FILES).rev() {
        let from = path.with_extension(index.to_string());
        let to = path.with_extension((index + 1).to_string());
        if from.exists() {
            fs::rename(from, to)?;
        }
    }
    if path.exists() {
        fs::rename(path, path.with_extension("1"))?;
    }
    Ok(())
}

/// Fatal startup failures happen before the webview can render an error. Keep
/// a durable record even in release builds, where the Windows console is
/// intentionally suppressed.
pub fn record_startup_failure(error: &str) {
    record("ERROR", &format!("APP_STARTUP_FAILED detail={error}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn app_log_creates_a_parent_and_keeps_entries_on_one_line() {
        let temporary = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = temporary.path().join("logs").join("application.log");
        append_at(&path, "INFO", "START_REQUEST\nport=COM5")
            .unwrap_or_else(|error| panic!("{error}"));
        let line = fs::read_to_string(path).unwrap_or_else(|error| panic!("{error}"));
        assert!(line.contains("INFO START_REQUEST port=COM5"));
        assert_eq!(line.lines().count(), 1);
    }

    #[test]
    fn app_log_rotates_before_exceeding_the_retention_limit() {
        let temporary = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = temporary.path().join("application.log");
        fs::write(&path, vec![b'x'; MAX_LOG_BYTES as usize])
            .unwrap_or_else(|error| panic!("{error}"));
        append_at(&path, "INFO", "ROTATED").unwrap_or_else(|error| panic!("{error}"));
        assert!(path.is_file());
        assert!(path.with_extension("1").is_file());
        assert!(fs::read_to_string(path)
            .unwrap_or_default()
            .contains("INFO ROTATED"));
    }
}
