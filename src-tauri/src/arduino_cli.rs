//! Safe Arduino CLI subprocess adapter. Arguments are never passed through a shell.
//!
//! Every Arduino CLI process runs in its own kill-on-close Windows Job Object,
//! so a cancel, a timeout, the end of the command, or the application exiting
//! ends the whole tool tree (bossac, the compiler, discovery tools), not only
//! `arduino-cli.exe`. Every command also has a deadline.
use serde::Serialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

/// Deadline for queries that cannot be canceled (`board list`, `version`,
/// `core list`). Discovery normally answers within a few seconds; a hung
/// discovery tool or driver is stopped instead of freezing the caller.
pub const QUERY_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
/// Safety bound for the cancelable reference compile on a slow lab computer.
pub const COMPILE_COMMAND_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// The upload cannot be canceled once it starts (stopping a flash part-way
/// helps no one), so it is bounded instead. bossac normally needs 10-20 s.
pub const UPLOAD_COMMAND_TIMEOUT: Duration = Duration::from_secs(3 * 60);
/// After the tool tree has ended, its output is collected for at most this long.
const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(40);
const OUTPUT_UNAVAILABLE: &str =
    "[output unavailable: a tool process kept its output open after it was stopped]";

/// Arduino CLI and UNO R4 core versions recorded as recording provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolVersions {
    pub arduino_cli_version: String,
    pub uno_r4_core_version: String,
}

static TOOL_VERSIONS: OnceLock<ToolVersions> = OnceLock::new();

/// The bundled tools cannot change while the application runs, so their
/// versions are probed once (normally right after runtime preparation) and
/// reused by every recording. Only a successful probe is cached; a failure is
/// returned to the caller and probed again next time.
pub fn cached_tool_versions() -> Result<ToolVersions, CliError> {
    if let Some(versions) = TOOL_VERSIONS.get() {
        return Ok(versions.clone());
    }
    let cli = ArduinoCli::discover(None)?;
    let probed = ToolVersions {
        arduino_cli_version: cli.version()?.stdout.trim().to_owned(),
        uno_r4_core_version: cli.uno_r4_core_version()?,
    };
    Ok(TOOL_VERSIONS.get_or_init(|| probed).clone())
}

/// The cached versions, if a probe has already succeeded. Never starts a process.
pub fn known_tool_versions() -> Option<ToolVersions> {
    TOOL_VERSIONS.get().cloned()
}

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
    /// The command exceeded its deadline and its tool tree was stopped.
    pub timed_out: bool,
}

impl CommandLog {
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0) && !self.canceled && !self.timed_out
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
    #[error("Arduino tools were not found. Select Repair Arduino tools. If that does not help, reinstall WVU Bioinstrumentation Studio or contact your instructor.")]
    NotFound,
    #[error("Arduino tools are not ready yet. Wait for setup to finish. If Arduino tools shows Needs attention, select Repair Arduino tools.")]
    RuntimeUnavailable,
    #[error("Arduino CLI command failed (exit {exit_code:?})")]
    CommandFailed {
        log: Box<CommandLog>,
        exit_code: Option<i32>,
    },
    #[error("Arduino tools did not respond within {timeout_seconds} seconds and were stopped.")]
    TimedOut {
        log: Box<CommandLog>,
        timeout_seconds: u64,
    },
    #[error("Arduino CLI emitted invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl CliError {
    pub fn command_log(&self) -> Option<&CommandLog> {
        match self {
            Self::CommandFailed { log, .. } | Self::TimedOut { log, .. } => Some(log),
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
    /// nonzero exit is an expected workflow failure. These queries cannot be
    /// canceled, so they are bounded by `QUERY_COMMAND_TIMEOUT`.
    pub fn run_capture(&self, args: &[String]) -> Result<CommandLog, CliError> {
        self.run_supervised(args, &AtomicBool::new(false), QUERY_COMMAND_TIMEOUT)
    }

    /// A compile worker polls this cancellation flag; a cancel ends the
    /// command's own tool tree. No unrelated application is killed.
    pub fn run_cancellable(
        &self,
        args: &[String],
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<CommandLog, CliError> {
        self.run_supervised(args, cancel, timeout)
    }

    /// Runs one command in its own process tree. A cancel or a missed
    /// `timeout` stops the whole tree; when the command ends, any helper it
    /// left behind is stopped too, so no tool outlives its command. A timeout
    /// is reported as `CliError::TimedOut` with the captured output.
    fn run_supervised(
        &self,
        args: &[String],
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<CommandLog, CliError> {
        // Repair Arduino tools cannot replace the tools while this runs (and
        // a command asked for during a repair starts on the repaired tools).
        let _tools = crate::arduino_runtime::tools_in_use();
        let started = Instant::now();
        let mut command = hidden_command(&self.executable);
        let mut child = command
            .args(self.configured_arguments(args))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // Adopted immediately after spawn; see `ProcessTree` for the race.
        // Dropping the tree on any early return below ends the tools as well.
        let tree = ProcessTree::adopt(&child);
        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            tree.terminate(&mut child);
            let _ = child.wait();
            return Err(std::io::Error::other("Arduino CLI output pipes were unavailable").into());
        };
        let (sender, outputs) = mpsc::channel();
        let stdout_sender = sender.clone();
        thread::spawn(move || {
            let _ = stdout_sender.send((OutputStream::Stdout, read_pipe(stdout)));
        });
        thread::spawn(move || {
            let _ = sender.send((OutputStream::Stderr, read_pipe(stderr)));
        });
        let mut canceled = false;
        let mut timed_out = false;
        let status = loop {
            if !canceled && !timed_out {
                if cancel.load(Ordering::Acquire) {
                    canceled = true;
                    tree.terminate(&mut child);
                } else if started.elapsed() >= timeout {
                    timed_out = true;
                    tree.terminate(&mut child);
                }
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            thread::sleep(CHILD_POLL_INTERVAL);
        };
        // arduino-cli has exited; a helper it started must not keep running
        // (or keep its output pipe open).
        tree.end_remaining();
        let (stdout, stderr) = drain_outputs(&outputs, OUTPUT_DRAIN_TIMEOUT)?;
        let log = CommandLog {
            command: std::iter::once(self.executable.display().to_string())
                .chain(self.configured_arguments(args))
                .collect(),
            exit_code: status.code(),
            stdout,
            stderr,
            duration_ms: started.elapsed().as_millis(),
            canceled,
            timed_out,
        };
        if timed_out {
            crate::app_log::record(
                "WARN",
                &format!(
                    "ARDUINO_CLI_TIMEOUT seconds={} command={}",
                    timeout.as_secs(),
                    args.join(" ")
                ),
            );
            return Err(CliError::TimedOut {
                log: Box::new(log),
                timeout_seconds: timeout.as_secs(),
            });
        }
        Ok(log)
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
        self.run_cancellable(&args, cancel, COMPILE_COMMAND_TIMEOUT)
    }

    /// Writes the reference binary. Deliberately not cancelable: it is only
    /// bounded by `UPLOAD_COMMAND_TIMEOUT` (or ended if the application exits).
    pub fn upload_input(&self, binary: &Path, port: &str) -> Result<CommandLog, CliError> {
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
        self.run_supervised(&args, &AtomicBool::new(false), UPLOAD_COMMAND_TIMEOUT)
    }
}

/// Every helper process (Arduino CLI, File Explorer) starts without a console
/// window: the application uses the Windows GUI subsystem.
pub(crate) fn hidden_command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// The tools started for one Arduino CLI command.
///
/// On Windows the child is assigned to a new Job Object created with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Everything the child starts
/// afterwards joins the job automatically, and Windows ends all of it when
/// the job handle closes: when `terminate` or `end_remaining` runs, when this
/// value is dropped, and when the application exits for any reason. The job
/// handle is not inheritable, so no tool can keep the job open.
///
/// Known race, accepted: the child is assigned right after `spawn`, not
/// created suspended (std cannot resume a suspended main thread). A process
/// started by `arduino-cli.exe` in that gap would not be in the job. The gap
/// is a single system call, while arduino-cli loads its configuration before
/// it starts any tool, so it is not expected in practice. If the job cannot
/// be created or assigned, the command still runs and cancel or timeout falls
/// back to ending `arduino-cli.exe` alone (logged as `ARDUINO_CLI_JOB_UNAVAILABLE`).
struct ProcessTree {
    #[cfg(windows)]
    job: Option<kill_on_close_job::Job>,
}

impl ProcessTree {
    fn adopt(child: &Child) -> Self {
        #[cfg(windows)]
        {
            let job = kill_on_close_job::Job::create().and_then(|job| {
                job.assign(child)?;
                Ok(job)
            });
            match job {
                Ok(job) => Self { job: Some(job) },
                Err(error) => {
                    crate::app_log::record(
                        "WARN",
                        &format!("ARDUINO_CLI_JOB_UNAVAILABLE detail={error}"),
                    );
                    Self { job: None }
                }
            }
        }
        #[cfg(not(windows))]
        {
            let _ = child;
            Self {}
        }
    }

    /// Ends the command and every tool it started.
    fn terminate(&self, child: &mut Child) {
        #[cfg(windows)]
        if self
            .job
            .as_ref()
            .is_some_and(kill_on_close_job::Job::terminate)
        {
            return;
        }
        let _ = child.kill();
    }

    /// Ends any tool still running after the command itself exited.
    fn end_remaining(&self) {
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.terminate();
        }
    }
}

#[cfg(windows)]
mod kill_on_close_job {
    use std::{io, os::windows::io::AsRawHandle, process::Child};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
    };

    /// Exit code reported by tools ended through the job.
    const TERMINATED_EXIT_CODE: u32 = 1;

    /// An owned, non-inheritable kill-on-close job handle.
    pub struct Job(HANDLE);

    impl Job {
        pub fn create() -> io::Result<Self> {
            // SAFETY: null attributes and name create an anonymous job whose
            // handle is not inheritable. The returned handle is owned by `Job`.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = Self(handle);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: `limits` is a fully initialized structure of the size
            // passed, and it outlives the call.
            let configured = unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    std::ptr::from_ref(&limits).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if configured == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }

        pub fn assign(&self, child: &Child) -> io::Result<()> {
            // SAFETY: both handles are valid for the duration of the call; the
            // child handle is borrowed from a live `Child`.
            let assigned = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle()) };
            if assigned == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        /// Ends every process in the job. Returns false if Windows refused.
        pub fn terminate(&self) -> bool {
            // SAFETY: the handle is owned and open until `drop`.
            unsafe { TerminateJobObject(self.0, TERMINATED_EXIT_CODE) != 0 }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle is owned and closed exactly once. Closing the
            // last handle ends any process still in the job.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum OutputStream {
    Stdout,
    Stderr,
}

/// Collects both output streams. A reader that does not finish within
/// `timeout` (a tool outside the job kept the pipe open) is abandoned and its
/// output marked unavailable, so a command never waits forever on a pipe.
fn drain_outputs(
    outputs: &mpsc::Receiver<(OutputStream, std::io::Result<String>)>,
    timeout: Duration,
) -> Result<(String, String), CliError> {
    let deadline = Instant::now() + timeout;
    let mut stdout = None;
    let mut stderr = None;
    while stdout.is_none() || stderr.is_none() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match outputs.recv_timeout(remaining) {
            Ok((OutputStream::Stdout, text)) => stdout = Some(text?),
            Ok((OutputStream::Stderr, text)) => stderr = Some(text?),
            Err(_) => break,
        }
    }
    Ok((
        stdout.unwrap_or_else(|| OUTPUT_UNAVAILABLE.into()),
        stderr.unwrap_or_else(|| OUTPUT_UNAVAILABLE.into()),
    ))
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
                timed_out: false,
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

    /// `cmd.exe` stands in for arduino-cli; `ping` stands in for a tool it
    /// starts (bossac, the compiler, a discovery tool). The tool inherits the
    /// output pipe, so the output can only be collected promptly if the
    /// whole tree really ended.
    #[cfg(windows)]
    fn command_shell() -> ArduinoCli {
        ArduinoCli {
            executable: std::env::var_os("ComSpec")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("C:\\Windows\\System32\\cmd.exe")),
            config_file: None,
        }
    }

    #[cfg(windows)]
    fn args(text: &[&str]) -> Vec<String> {
        text.iter().map(|item| (*item).to_owned()).collect()
    }

    #[cfg(windows)]
    #[test]
    fn a_timeout_stops_the_whole_tool_tree() {
        let started = Instant::now();
        let error = command_shell()
            .run_supervised(
                &args(&["/d", "/c", "ping -n 30 127.0.0.1"]),
                &AtomicBool::new(false),
                Duration::from_millis(800),
            )
            .err()
            .unwrap_or_else(|| panic!("the command must time out"));
        let CliError::TimedOut {
            log,
            timeout_seconds,
        } = error
        else {
            panic!("expected a timeout, got {error}");
        };
        assert_eq!(timeout_seconds, 0);
        assert!(log.timed_out);
        assert!(!log.succeeded());
        assert_ne!(log.stdout, OUTPUT_UNAVAILABLE, "the tool kept running");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(windows)]
    #[test]
    fn cancel_stops_the_whole_tool_tree() {
        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        let log = thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(500));
                cancel.store(true, Ordering::Release);
            });
            command_shell().run_cancellable(
                &args(&["/d", "/c", "ping -n 30 127.0.0.1"]),
                &cancel,
                COMPILE_COMMAND_TIMEOUT,
            )
        })
        .unwrap_or_else(|error| panic!("{error}"));
        assert!(log.canceled);
        assert!(!log.timed_out);
        assert!(!log.succeeded());
        assert_ne!(log.stdout, OUTPUT_UNAVAILABLE, "the tool kept running");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(windows)]
    #[test]
    fn no_tool_outlives_a_command_that_finished() {
        // `start /b` leaves ping running after cmd.exe itself has exited.
        let started = Instant::now();
        let log = command_shell()
            .run_supervised(
                &args(&["/d", "/c", "start /b ping -n 30 127.0.0.1 & echo done"]),
                &AtomicBool::new(false),
                QUERY_COMMAND_TIMEOUT,
            )
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(log.succeeded());
        assert!(log.stdout.contains("done"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
