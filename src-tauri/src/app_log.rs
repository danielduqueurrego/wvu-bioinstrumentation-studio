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
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(
        file,
        "{} {} {}",
        Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        level,
        event.replace(['\r', '\n'], " ")
    )
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
}
