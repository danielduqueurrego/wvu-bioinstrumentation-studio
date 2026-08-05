//! Safe Arduino CLI subprocess adapter. Arguments are never passed through a shell.
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize)]
pub struct CommandLog {
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u128,
}
#[derive(Clone, Debug, Serialize)]
pub struct BoardInfo {
    pub port: String,
    pub name: String,
    pub fqbn: String,
    pub serial_number: Option<String>,
}
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("Arduino CLI was not found. Install it or set an instructor-approved path.")]
    NotFound,
    #[error("Arduino CLI failed: {0}")]
    Failed(String),
    #[error("Arduino CLI emitted invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
#[derive(Clone, Debug)]
pub struct ArduinoCli {
    executable: PathBuf,
}
impl ArduinoCli {
    pub fn discover(override_path: Option<&Path>) -> Result<Self, CliError> {
        if let Some(path) = override_path {
            if path.is_file() {
                return Ok(Self {
                    executable: path.to_path_buf(),
                });
            }
            return Err(CliError::NotFound);
        }
        if let Some(path) = std::env::var_os("BMEG_ARDUINO_CLI") {
            let p = PathBuf::from(path);
            if p.is_file() {
                return Ok(Self { executable: p });
            }
        }
        for candidate in [
            PathBuf::from("C:\\arduino-cli\\arduino-cli.exe"),
            PathBuf::from("arduino-cli"),
        ] {
            if candidate == std::path::Path::new("arduino-cli") || candidate.is_file() {
                return Ok(Self {
                    executable: candidate,
                });
            }
        }
        Err(CliError::NotFound)
    }
    pub fn run(&self, args: &[&str]) -> Result<CommandLog, CliError> {
        let start = Instant::now();
        let out = Command::new(&self.executable)
            .args(args)
            .stdin(Stdio::null())
            .output()?;
        let log = CommandLog {
            command: std::iter::once(self.executable.display().to_string())
                .chain(args.iter().map(|x| (*x).to_string()))
                .collect(),
            exit_code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            duration_ms: start.elapsed().as_millis(),
        };
        if out.status.success() {
            Ok(log)
        } else {
            Err(CliError::Failed(format!("{}\n{}", log.stdout, log.stderr)))
        }
    }
    pub fn version(&self) -> Result<CommandLog, CliError> {
        self.run(&["version"])
    }
    pub fn boards(&self) -> Result<Vec<BoardInfo>, CliError> {
        let log = self.run(&["board", "list", "--format", "json"])?;
        let root: serde_json::Value = serde_json::from_str(&log.stdout)?;
        let mut result = Vec::new();
        if let Some(ports) = root.get("detected_ports").and_then(|x| x.as_array()) {
            for item in ports {
                if let (Some(port), Some(board)) = (
                    item.get("port"),
                    item.get("matching_boards")
                        .and_then(|x| x.as_array())
                        .and_then(|x| x.first()),
                ) {
                    let fqbn = board
                        .get("fqbn")
                        .and_then(|x| x.as_str())
                        .unwrap_or_default();
                    if fqbn == "arduino:renesas_uno:unor4wifi" {
                        result.push(BoardInfo {
                            port: port
                                .get("address")
                                .and_then(|x| x.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            name: board
                                .get("name")
                                .and_then(|x| x.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            fqbn: fqbn.to_string(),
                            serial_number: port
                                .get("properties")
                                .and_then(|x| x.get("serialNumber"))
                                .and_then(|x| x.as_str())
                                .map(str::to_string),
                        });
                    }
                }
            }
        }
        Ok(result)
    }
    pub fn uno_r4_core_version(&self) -> Result<String, CliError> {
        let log = self.run(&["core", "list", "--format", "json"])?;
        let root: serde_json::Value = serde_json::from_str(&log.stdout)?;
        root.get("platforms")
            .and_then(|platforms| platforms.as_array())
            .and_then(|platforms| {
                platforms.iter().find(|platform| {
                    platform.get("id").and_then(|id| id.as_str()) == Some("arduino:renesas_uno")
                })
            })
            .and_then(|platform| platform.get("installed_version"))
            .and_then(|version| version.as_str())
            .map(str::to_owned)
            .ok_or_else(|| CliError::Failed("UNO R4 core is not installed".into()))
    }
    pub fn compile(&self, sketch: &Path) -> Result<CommandLog, CliError> {
        self.run(&[
            "compile",
            "--fqbn",
            "arduino:renesas_uno:unor4wifi",
            "--format",
            "json",
            &sketch.display().to_string(),
        ])
    }
    pub fn upload(&self, sketch: &Path, port: &str) -> Result<CommandLog, CliError> {
        self.run(&[
            "upload",
            "--fqbn",
            "arduino:renesas_uno:unor4wifi",
            "--port",
            port,
            &sketch.display().to_string(),
        ])
    }
}
pub fn duration_to_ms(duration: Duration) -> u128 {
    duration.as_millis()
}

#[cfg(test)]
mod tests {
    #[test]
    fn uno_core_version_is_read_from_cli_json_shape() {
        let document = serde_json::json!({
            "platforms": [{ "id": "arduino:renesas_uno", "installed_version": "1.6.0" }]
        });
        let version = document
            .get("platforms")
            .and_then(|platforms| platforms.as_array())
            .and_then(|platforms| {
                platforms
                    .iter()
                    .find(|platform| platform["id"] == "arduino:renesas_uno")
            })
            .and_then(|platform| platform["installed_version"].as_str());
        assert_eq!(version, Some("1.6.0"));
    }
}
