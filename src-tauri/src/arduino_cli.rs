//! Safe Arduino CLI subprocess adapter. Arguments are never passed through a shell.
use serde::Serialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Prevents command-prompt flashes when Arduino CLI and its helpers run from
/// the packaged Windows application.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub const UNO_R4_WIFI_FQBN: &str = "arduino:renesas_uno:unor4wifi";
/// Override the upstream UNO R4 WiFi board recipe only for the controlled
/// reference firmware. Omitting `NO_USB` makes `Serial` use RA4M1 native USB
/// CDC instead of the intermittent RA4M1-to-ESP32 serial bridge.
pub const NATIVE_USB_REFERENCE_BUILD_DEFINES: &str =
    "-DF_CPU=48000000 -DBACKTRACE_SUPPORT -DARDUINO_UNOR4_WIFI";

#[derive(Clone, Debug, Serialize)]
pub struct CommandLog {
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u128,
    pub canceled: bool,
}

impl CommandLog {
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0) && !self.canceled
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BoardInfo {
    pub port: String,
    pub name: String,
    pub fqbn: String,
    pub serial_number: Option<String>,
    /// Arduino CLI's USB identity is needed to recognize the controlled
    /// firmware's RA4M1 native-USB application port after an upload.
    pub usb_vid: Option<u16>,
    pub usb_pid: Option<u16>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompilerDiagnostic {
    pub file: String,
    pub line: u32,
    pub column: Option<u32>,
    pub severity: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct CompileUsage {
    pub sketch_bytes: Option<u64>,
    pub sketch_percent: Option<u8>,
    pub ram_bytes: Option<u64>,
    pub ram_percent: Option<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("Arduino tools were not found. Reinstall WVU Bioinstrumentation Studio or contact your instructor.")]
    NotFound,
    #[error("Arduino tools are not ready. Wait for setup to finish or reinstall WVU Bioinstrumentation Studio.")]
    RuntimeUnavailable,
    #[error("Arduino CLI command failed (exit {exit_code:?})")]
    CommandFailed {
        log: Box<CommandLog>,
        exit_code: Option<i32>,
    },
    #[error("Arduino CLI emitted invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl CliError {
    pub fn command_log(&self) -> Option<&CommandLog> {
        match self {
            Self::CommandFailed { log, .. } => Some(log),
            Self::NotFound | Self::RuntimeUnavailable | Self::Json(_) | Self::Io(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ArduinoCli {
    executable: PathBuf,
    config_file: Option<PathBuf>,
}

impl ArduinoCli {
    pub fn discover(override_path: Option<&Path>) -> Result<Self, CliError> {
        if let Some(runtime) = crate::arduino_runtime::active_runtime() {
            return Ok(Self {
                executable: runtime.executable.clone(),
                config_file: Some(runtime.config_file.clone()),
            });
        }
        // Production intentionally has no PATH/global-Arduino fallback. The
        // bundled runtime is initialized before the startup board scan.
        if !cfg!(debug_assertions) {
            return Err(CliError::RuntimeUnavailable);
        }
        if let Some(path) = override_path {
            if path.is_file() {
                return Ok(Self {
                    executable: path.to_path_buf(),
                    config_file: None,
                });
            }
            return Err(CliError::NotFound);
        }
        if let Some(path) = std::env::var_os("BMEG_ARDUINO_CLI") {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Ok(Self {
                    executable: path,
                    config_file: None,
                });
            }
        }
        let bundled = PathBuf::from("C:\\arduino-cli\\arduino-cli.exe");
        if bundled.is_file() {
            return Ok(Self {
                executable: bundled,
                config_file: None,
            });
        }
        if let Some(path) = executable_from_path() {
            return Ok(Self {
                executable: path,
                config_file: None,
            });
        }
        Err(CliError::NotFound)
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    fn configured_arguments(&self, args: &[String]) -> Vec<String> {
        self.config_file
            .iter()
            .flat_map(|path| ["--config-file".to_owned(), path.display().to_string()])
            .chain(args.iter().cloned())
            .collect()
    }

    pub fn run(&self, args: &[&str]) -> Result<CommandLog, CliError> {
        let args: Vec<String> = args.iter().map(|argument| (*argument).to_owned()).collect();
        let log = self.run_capture(&args)?;
        if log.succeeded() {
            Ok(log)
        } else {
            let exit_code = log.exit_code;
            Err(CliError::CommandFailed {
                log: Box::new(log),
                exit_code,
            })
        }
    }

    /// Captures stdout/stderr for every exit result. The caller decides whether a
    /// nonzero exit is an expected workflow failure.
    pub fn run_capture(&self, args: &[String]) -> Result<CommandLog, CliError> {
        self.run_cancellable(args, &AtomicBool::new(false))
    }

    /// A compile or upload worker polls this cancellation flag and terminates its
    /// own child process. No global shell process or unrelated application is killed.
    pub fn run_cancellable(
        &self,
        args: &[String],
        cancel: &AtomicBool,
    ) -> Result<CommandLog, CliError> {
        let started = Instant::now();
        let mut command = hidden_command(&self.executable);
        let mut child = command
            .args(self.configured_arguments(args))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("Arduino CLI stdout pipe was unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| std::io::Error::other("Arduino CLI stderr pipe was unavailable"))?;
        let stdout_reader = thread::spawn(move || read_pipe(stdout));
        let stderr_reader = thread::spawn(move || read_pipe(stderr));
        let mut canceled = false;
        let status = loop {
            if cancel.load(Ordering::Acquire) {
                canceled = true;
                let _ = child.kill();
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            thread::sleep(Duration::from_millis(40));
        };
        let stdout = join_pipe(stdout_reader)?;
        let stderr = join_pipe(stderr_reader)?;
        Ok(CommandLog {
            command: std::iter::once(self.executable.display().to_string())
                .chain(self.configured_arguments(args))
                .collect(),
            exit_code: status.code(),
            stdout,
            stderr,
            duration_ms: started.elapsed().as_millis(),
            canceled,
        })
    }

    pub fn version(&self) -> Result<CommandLog, CliError> {
        self.run(&["version"])
    }

    pub fn boards(&self) -> Result<Vec<BoardInfo>, CliError> {
        let log = self.run(&["board", "list", "--format", "json"])?;
        parse_boards_json(&log.stdout)
    }

    pub fn uno_r4_core_version(&self) -> Result<String, CliError> {
        let log = self.run(&["core", "list", "--format", "json"])?;
        parse_uno_core_json(&log.stdout)
    }

    pub fn compile_to(
        &self,
        sketch: &Path,
        output_dir: &Path,
        cancel: &AtomicBool,
    ) -> Result<CommandLog, CliError> {
        let args = vec![
            "compile".into(),
            "--fqbn".into(),
            UNO_R4_WIFI_FQBN.into(),
            "--build-property".into(),
            format!("build.defines={NATIVE_USB_REFERENCE_BUILD_DEFINES}"),
            "--output-dir".into(),
            output_dir.display().to_string(),
            sketch.display().to_string(),
        ];
        self.run_cancellable(&args, cancel)
    }

    pub fn upload_input(
        &self,
        binary: &Path,
        port: &str,
        cancel: &AtomicBool,
    ) -> Result<CommandLog, CliError> {
        let args = vec![
            "upload".into(),
            "--fqbn".into(),
            UNO_R4_WIFI_FQBN.into(),
            "--port".into(),
            port.to_owned(),
            "--input-file".into(),
            binary.display().to_string(),
            "--verbose".into(),
            "--verify".into(),
        ];
        self.run_cancellable(&args, cancel)
    }
}

fn hidden_command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

fn executable_from_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let executable_names: &[&str] = if cfg!(windows) {
        &["arduino-cli.exe", "arduino-cli"]
    } else {
        &["arduino-cli"]
    };
    std::env::split_paths(&path)
        .flat_map(|directory| {
            executable_names
                .iter()
                .map(move |name| directory.join(name))
        })
        .find(|candidate| candidate.is_file())
}

pub fn parse_boards_json(text: &str) -> Result<Vec<BoardInfo>, CliError> {
    let root: serde_json::Value = serde_json::from_str(text)?;
    let mut result = Vec::new();
    if let Some(ports) = root
        .get("detected_ports")
        .and_then(|value| value.as_array())
    {
        for item in ports {
            if let (Some(port), Some(board)) = (
                item.get("port"),
                item.get("matching_boards")
                    .and_then(|value| value.as_array())
                    .and_then(|boards| boards.first()),
            ) {
                let fqbn = board
                    .get("fqbn")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                if fqbn == UNO_R4_WIFI_FQBN {
                    result.push(BoardInfo {
                        port: port
                            .get("address")
                            .and_then(|value| value.as_str())
                            .unwrap_or_default()
                            .to_owned(),
                        name: board
                            .get("name")
                            .and_then(|value| value.as_str())
                            .unwrap_or_default()
                            .to_owned(),
                        fqbn: fqbn.to_owned(),
                        serial_number: port
                            .get("properties")
                            .and_then(|properties| properties.get("serialNumber"))
                            .and_then(|value| value.as_str())
                            .map(str::to_owned),
                        usb_vid: port
                            .get("properties")
                            .and_then(|properties| properties.get("vid"))
                            .and_then(|value| value.as_str())
                            .and_then(parse_usb_id),
                        usb_pid: port
                            .get("properties")
                            .and_then(|properties| properties.get("pid"))
                            .and_then(|value| value.as_str())
                            .and_then(parse_usb_id),
                    });
                }
            }
        }
    }
    Ok(result)
}

fn parse_usb_id(value: &str) -> Option<u16> {
    u16::from_str_radix(value.trim().trim_start_matches("0x"), 16).ok()
}

pub fn parse_uno_core_json(text: &str) -> Result<String, CliError> {
    let root: serde_json::Value = serde_json::from_str(text)?;
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
        .ok_or_else(|| CliError::CommandFailed {
            log: Box::new(CommandLog {
                command: vec!["arduino-cli core list --format json".into()],
                exit_code: Some(0),
                stdout: text.into(),
                stderr: "UNO R4 core is not installed".into(),
                duration_ms: 0,
                canceled: false,
            }),
            exit_code: Some(0),
        })
}

pub fn parse_compile_usage(text: &str) -> CompileUsage {
    let mut usage = CompileUsage::default();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Sketch uses ") {
            usage.sketch_bytes = parse_first_number(rest);
            usage.sketch_percent = parse_percent(rest);
        }
        if let Some(rest) = line.strip_prefix("Global variables use ") {
            usage.ram_bytes = parse_first_number(rest);
            usage.ram_percent = parse_percent(rest);
        }
    }
    usage
}

pub fn parse_compiler_diagnostics(text: &str) -> Vec<CompilerDiagnostic> {
    text.lines().filter_map(parse_compiler_diagnostic).collect()
}

fn parse_compiler_diagnostic(line: &str) -> Option<CompilerDiagnostic> {
    let marker = ".ino:";
    let file_end = line.find(marker)? + ".ino".len();
    let file = line[..file_end].to_owned();
    let rest = &line[file_end + 1..];
    let mut parts = rest.splitn(3, ':');
    let line_number = parts.next()?.trim().parse::<u32>().ok()?;
    let second = parts.next()?.trim();
    let (column, message) = match second.parse::<u32>() {
        Ok(column) => (
            Some(column),
            parts.next().unwrap_or_default().trim().to_owned(),
        ),
        Err(_) => (
            None,
            [second, parts.next().unwrap_or_default().trim()].join(":"),
        ),
    };
    let lowercase = message.to_ascii_lowercase();
    let severity = if lowercase.contains("warning") {
        "warning"
    } else if lowercase.contains("error") {
        "error"
    } else {
        "message"
    };
    Some(CompilerDiagnostic {
        file,
        line: line_number,
        column,
        severity: severity.into(),
        message,
    })
}

fn parse_first_number(text: &str) -> Option<u64> {
    text.split_whitespace()
        .next()?
        .replace(',', "")
        .parse::<u64>()
        .ok()
}

fn parse_percent(text: &str) -> Option<u8> {
    let before = text.split('%').next()?;
    let number = before.split('(').next_back()?.trim();
    number.parse::<u8>().ok()
}

fn read_pipe<T: Read>(mut pipe: T) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn join_pipe(handle: thread::JoinHandle<std::io::Result<String>>) -> Result<String, CliError> {
    handle
        .join()
        .map_err(|_| std::io::Error::other("Arduino CLI output reader panicked"))?
        .map_err(CliError::Io)
}

pub fn duration_to_ms(duration: Duration) -> u128 {
    duration.as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uno_core_version_is_read_from_cli_json_shape() {
        let document =
            r#"{"platforms":[{"id":"arduino:renesas_uno","installed_version":"1.6.0"}]}"#;
        assert_eq!(parse_uno_core_json(document).unwrap_or_default(), "1.6.0");
    }

    #[test]
    fn board_and_compiler_output_parsing_preserve_useful_diagnostics() {
        let boards = parse_boards_json(
            r#"{"detected_ports":[{"port":{"address":"COM12","properties":{"serialNumber":"ABC","vid":"0x2341","pid":"0x006D"}},"matching_boards":[{"name":"Arduino UNO R4 WiFi","fqbn":"arduino:renesas_uno:unor4wifi"}]}]}"#,
        )
        .unwrap_or_default();
        assert_eq!(boards[0].port, "COM12");
        assert_eq!(boards[0].usb_vid, Some(0x2341));
        assert_eq!(boards[0].usb_pid, Some(0x006d));
        let usage = parse_compile_usage(
            "Sketch uses 53508 bytes (20%) of program storage space.\nGlobal variables use 7940 bytes (24%) of dynamic memory.",
        );
        assert_eq!(usage.sketch_bytes, Some(53_508));
        assert_eq!(usage.ram_percent, Some(24));
        let diagnostics = parse_compiler_diagnostics(
            "C:\\projects\\Demo\\Demo.ino:12:5: error: expected ';' before '}' token",
        );
        assert_eq!(diagnostics[0].line, 12);
        assert_eq!(diagnostics[0].column, Some(5));
        assert_eq!(diagnostics[0].severity, "error");
    }

    #[test]
    fn invalid_instructor_override_reports_a_missing_cli_without_shell_fallback() {
        let missing = PathBuf::from("C:\\not-a-real-arduino-cli\\arduino-cli.exe");
        assert!(matches!(
            ArduinoCli::discover(Some(&missing)),
            Err(CliError::NotFound)
        ));
    }

    #[test]
    fn bundled_runtime_commands_always_receive_the_app_config_file() {
        let cli = ArduinoCli {
            executable: PathBuf::from("C:\\runtime\\arduino-cli.exe"),
            config_file: Some(PathBuf::from("C:\\runtime\\arduino-cli.yaml")),
        };
        assert_eq!(
            cli.configured_arguments(&["board".into(), "list".into()]),
            vec![
                "--config-file".to_owned(),
                "C:\\runtime\\arduino-cli.yaml".to_owned(),
                "board".to_owned(),
                "list".to_owned()
            ]
        );
    }

    #[test]
    fn controlled_reference_compile_uses_ra4m1_native_usb_without_no_usb() {
        assert!(NATIVE_USB_REFERENCE_BUILD_DEFINES.contains("-DARDUINO_UNOR4_WIFI"));
        assert!(!NATIVE_USB_REFERENCE_BUILD_DEFINES.contains("NO_USB"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_processes_use_the_no_window_creation_flag() {
        assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
    }
}
