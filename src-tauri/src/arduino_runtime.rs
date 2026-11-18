//! App-owned, offline Arduino tool runtime. Production never relies on a
//! student's Arduino IDE installation, PATH, or global Arduino15 directory.
//!
//! Preparation is serialized inside the process and, through a lock file next
//! to the runtime folder, across copies of the application, so a second launch
//! or a webview reload cannot delete a first-run extraction that is still
//! running. An extracted runtime is used only if every file listed in the
//! bundled archive is present with its archived size; otherwise it is
//! extracted again (the archive's CRC-32 checks run during extraction).
//! Repair Arduino tools forces that re-extraction.
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock, PoisonError, RwLock, RwLockReadGuard},
    thread,
    time::{Duration, Instant},
};
use tauri::Manager;
use zip::ZipArchive;

static ACTIVE_RUNTIME: OnceLock<RuntimePaths> = OnceLock::new();
/// Serializes preparation and repair inside this process.
static PREPARATION: Mutex<()> = Mutex::new(());
/// Every Arduino CLI command holds this shared while it runs; Repair holds it
/// exclusively while it replaces the tools. Commands that do not take the
/// serial lease (Refresh Board, the tools check) therefore never start from,
/// or keep files open in, a runtime folder that Repair is renaming.
static TOOLS_IN_USE: RwLock<()> = RwLock::new(());
/// How long a second copy of the application waits for a running extraction.
const PREPARATION_LOCK_WAIT: Duration = Duration::from_secs(10 * 60);
const PREPARATION_LOCK_POLL: Duration = Duration::from_millis(250);
/// Archive files that are rewritten after extraction by design: the generated
/// CLI configuration and Arduino CLI's own inventory file. They must still be
/// present, but their archived size is not compared. The version-pinned core
/// header patch is handled by `patched_core_header`.
const REWRITTEN_RUNTIME_FILES: &[&str] = &["arduino-cli.yaml", "data/inventory.yaml"];

#[derive(Clone, Debug)]
pub struct RuntimePaths {
    pub executable: PathBuf,
    pub config_file: PathBuf,
    pub root: PathBuf,
    pub cli_version: String,
    pub core_version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct RuntimeManifest {
    pub runtime_schema: u32,
    pub arduino_cli: String,
    pub renesas_uno_core: String,
    pub bundle_revision: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuntimeStatus {
    pub ready: bool,
    pub cli_version: String,
    pub core_version: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("The Arduino tools included with WVU Bioinstrumentation Studio are missing.")]
    MissingBundle,
    #[error("The Arduino tools could not be prepared: {0}")]
    Io(#[from] std::io::Error),
    #[error("The included Arduino tools information is invalid: {0}")]
    Manifest(#[from] serde_json::Error),
    #[error("The included Arduino tools are incompatible or incomplete.")]
    InvalidBundle,
    #[error("Another copy of WVU Bioinstrumentation Studio is preparing the Arduino tools.")]
    Busy,
}

/// The student-facing form of a tool-preparation problem, returned by the
/// prepare and repair commands. Technical details stay in Advanced details.
#[derive(Clone, Debug, Serialize)]
pub struct ArduinoToolsFailure {
    pub title: String,
    pub explanation: String,
    pub recommended_action: String,
    pub technical_details: String,
    /// Whether Repair Arduino tools (extracting the included tools again) can
    /// fix this. A missing or damaged installation needs a reinstall instead.
    pub repairable: bool,
}

impl ArduinoToolsFailure {
    fn new(
        title: &str,
        explanation: &str,
        recommended_action: &str,
        technical_details: impl Into<String>,
        repairable: bool,
    ) -> Self {
        Self {
            title: title.into(),
            explanation: explanation.into(),
            recommended_action: recommended_action.into(),
            technical_details: technical_details.into(),
            repairable,
        }
    }

    /// A preparation or repair task that could not run or did not return.
    pub fn task_failure(details: impl Into<String>) -> Self {
        Self::new(
            "Arduino tools need attention",
            "The Arduino tools could not be checked.",
            "Select Repair Arduino tools. If that does not help, restart the application or contact your instructor.",
            details,
            true,
        )
    }

    /// Another operation (for example a recording or a firmware restore) is
    /// using the Arduino tools, so a repair cannot replace them now.
    pub fn busy(message: &str) -> Self {
        Self::new(
            "Arduino tools are in use",
            message,
            "Wait for the current operation to finish, then select Repair Arduino tools again.",
            "serial lease refused",
            true,
        )
    }
}

impl RuntimeError {
    pub fn failure(&self) -> ArduinoToolsFailure {
        const REINSTALL: &str = "Reinstall WVU Bioinstrumentation Studio or contact your instructor. Simulator recordings still work.";
        match self {
            Self::MissingBundle => ArduinoToolsFailure::new(
                "Arduino tools missing",
                "The Arduino tools included with WVU Bioinstrumentation Studio were not found.",
                REINSTALL,
                self.to_string(),
                false,
            ),
            Self::InvalidBundle | Self::Manifest(_) => ArduinoToolsFailure::new(
                "Arduino tools incomplete",
                "The Arduino tools included with WVU Bioinstrumentation Studio are incomplete.",
                REINSTALL,
                self.to_string(),
                false,
            ),
            Self::Io(_) => ArduinoToolsFailure::new(
                "Arduino tools could not be prepared",
                "Windows could not write the Arduino tools to this computer.",
                "Close any other copy of WVU Bioinstrumentation Studio, then select Repair Arduino tools. If that does not help, restart the computer or contact your instructor.",
                self.to_string(),
                true,
            ),
            Self::Busy => ArduinoToolsFailure::new(
                "Arduino tools are being prepared",
                "Another copy of WVU Bioinstrumentation Studio is preparing the Arduino tools.",
                "Close the other copy of the application, then select Repair Arduino tools.",
                self.to_string(),
                true,
            ),
        }
    }
}

pub fn active_runtime() -> Option<&'static RuntimePaths> {
    ACTIVE_RUNTIME.get()
}

/// Held by an Arduino CLI command for as long as its tools run. A Repair in
/// progress makes the command wait until the new tools are in place.
pub fn tools_in_use() -> RwLockReadGuard<'static, ()> {
    // The lock guards no data, so a poisoned lock is still a valid lock.
    TOOLS_IN_USE.read().unwrap_or_else(PoisonError::into_inner)
}

fn ready_status(paths: &RuntimePaths) -> RuntimeStatus {
    RuntimeStatus {
        ready: true,
        cli_version: paths.cli_version.clone(),
        core_version: paths.core_version.clone(),
        message: "Arduino tools are ready.".into(),
    }
}

pub fn prepare_runtime(app: &tauri::AppHandle) -> Result<RuntimeStatus, RuntimeError> {
    if let Some(paths) = active_runtime() {
        return Ok(ready_status(paths));
    }
    let layout = RuntimeLayout::resolve(app)?;
    let paths = prepare_from_archive(&layout.archive, &layout.manifest, &layout.destination)?;
    let _ = ACTIVE_RUNTIME.set(paths.clone());
    Ok(ready_status(&paths))
}

/// Repair Arduino tools: extracts the included runtime again even if it looks
/// valid, then validates it. The caller ensures no Arduino CLI job is running.
/// The previous runtime is replaced only after the new copy has validated.
pub fn repair_runtime(app: &tauri::AppHandle) -> Result<RuntimeStatus, RuntimeError> {
    let layout = RuntimeLayout::resolve(app)?;
    // Waits for running Arduino CLI commands (each ends by its own deadline)
    // and holds new ones back until the replacement is complete. Released
    // before the caller probes the repaired tools.
    let _no_tools_running = TOOLS_IN_USE.write().unwrap_or_else(PoisonError::into_inner);
    let paths = repair_from_archive(&layout.archive, &layout.manifest, &layout.destination)?;
    let _ = ACTIVE_RUNTIME.set(paths.clone());
    Ok(ready_status(&paths))
}

struct RuntimeLayout {
    archive: PathBuf,
    manifest: PathBuf,
    destination: PathBuf,
}

impl RuntimeLayout {
    fn resolve(app: &tauri::AppHandle) -> Result<Self, RuntimeError> {
        let resource_root = app
            .path()
            .resource_dir()
            .map_err(|_| RuntimeError::MissingBundle)?
            .join("resources");
        // The bundled Renesas GCC toolchain has path-length-sensitive C++
        // multilib discovery on Windows. Keep its app-owned extraction root
        // deliberately short while ordinary application data stays in
        // Tauri's normal directory.
        let local_data_root = match dev_local_data_root() {
            Some(root) => root,
            None => app
                .path()
                .app_local_data_dir()
                .map_err(|_| RuntimeError::MissingBundle)?
                .parent()
                .map(Path::to_path_buf)
                .ok_or(RuntimeError::InvalidBundle)?,
        };
        Ok(Self {
            archive: resource_root.join("arduino-runtime.zip"),
            manifest: resource_root.join("arduino-runtime-manifest.json"),
            destination: local_data_root.join("WVU-BMEG").join("rt"),
        })
    }
}

/// Development builds only: `WVU_STUDIO_DEV_LOCAL_DATA` relocates the
/// extracted Arduino runtime (normally `%LOCALAPPDATA%\WVU-BMEG\rt`, resolved
/// through the Known Folder API, which ignores environment overrides). Combined
/// with a process-level `LOCALAPPDATA` override, this lets a developer run a
/// debug build against a disposable data root - for example with the CI
/// placeholder runtime - without replacing a real extracted runtime. Release
/// builds never read the variable.
#[cfg(debug_assertions)]
fn dev_local_data_root() -> Option<PathBuf> {
    std::env::var_os("WVU_STUDIO_DEV_LOCAL_DATA")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

#[cfg(not(debug_assertions))]
fn dev_local_data_root() -> Option<PathBuf> {
    None
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Preparation {
    /// Keep a runtime that matches the archive; extract only if it does not.
    ReuseValid,
    /// Repair: always extract again.
    Reextract,
}

pub fn prepare_from_archive(
    archive: &Path,
    manifest_path: &Path,
    destination: &Path,
) -> Result<RuntimePaths, RuntimeError> {
    prepare_with(archive, manifest_path, destination, Preparation::ReuseValid)
}

pub fn repair_from_archive(
    archive: &Path,
    manifest_path: &Path,
    destination: &Path,
) -> Result<RuntimePaths, RuntimeError> {
    prepare_with(archive, manifest_path, destination, Preparation::Reextract)
}

fn prepare_with(
    archive: &Path,
    manifest_path: &Path,
    destination: &Path,
    mode: Preparation,
) -> Result<RuntimePaths, RuntimeError> {
    let _this_process = PREPARATION.lock().unwrap_or_else(PoisonError::into_inner);
    let _other_copies = PreparationLock::acquire(destination)?;
    let source_manifest = read_manifest_file(manifest_path)?;
    let expected_files = match archive_entries(archive) {
        Ok(entries) => Some(entries),
        Err(error) => {
            // Without a readable archive nothing can be extracted again; a
            // runtime that passes the basic layout check stays usable.
            crate::app_log::record(
                "WARN",
                &format!("ARDUINO_RUNTIME_ARCHIVE_UNREADABLE detail={error}"),
            );
            None
        }
    };
    let reason = match mode {
        Preparation::Reextract => Some("repair requested".to_owned()),
        Preparation::ReuseValid => {
            runtime_problem(destination, &source_manifest, expected_files.as_deref())
        }
    };
    if let Some(reason) = reason {
        crate::app_log::record("INFO", &format!("ARDUINO_RUNTIME_EXTRACT reason={reason}"));
        replace_runtime_from_archive(
            archive,
            destination,
            &source_manifest,
            expected_files.as_deref(),
            mode,
        )?;
    }
    let executable = destination.join("arduino-cli.exe");
    let data = destination.join("data");
    if !executable.is_file()
        || !data.join("packages/arduino/hardware/renesas_uno").is_dir()
        || !data.join("packages/arduino/tools").is_dir()
    {
        return Err(RuntimeError::InvalidBundle);
    }
    apply_native_usb_core_compatibility_patch(destination, &source_manifest)?;
    let config_file = destination.join("arduino-cli.yaml");
    let downloads = destination.join("downloads");
    let sketchbook = destination.join("sketchbook");
    fs::create_dir_all(&downloads)?;
    fs::create_dir_all(&sketchbook)?;
    fs::write(
        &config_file,
        cli_config_text(&data, &downloads, &sketchbook),
    )?;
    Ok(RuntimePaths {
        executable,
        config_file,
        root: destination.to_path_buf(),
        cli_version: source_manifest.arduino_cli,
        core_version: source_manifest.renesas_uno_core,
    })
}

/// Arduino CLI configuration for the app-owned runtime. Every path is written
/// as a quoted scalar, so a Windows profile path containing ` #` (a YAML
/// comment), `: `, or quotes is read literally.
fn cli_config_text(data: &Path, downloads: &Path, user: &Path) -> String {
    format!(
        "directories:\n  data: {}\n  downloads: {}\n  user: {}\n",
        yaml_path(data),
        yaml_path(downloads),
        yaml_path(user)
    )
}

/// A JSON string is also a valid YAML double-quoted scalar.
fn yaml_path(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    serde_json::Value::String(normalized).to_string()
}

/// Exclusive, cross-process ownership of runtime preparation, held through a
/// lock file beside the runtime folder (outside the folder it protects). A
/// second copy waits for the first to finish rather than deleting its work.
struct PreparationLock {
    file: fs::File,
}

impl PreparationLock {
    fn acquire(destination: &Path) -> Result<Self, RuntimeError> {
        let parent = destination.parent().ok_or(RuntimeError::InvalidBundle)?;
        let name = destination
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(RuntimeError::InvalidBundle)?;
        fs::create_dir_all(parent)?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(parent.join(format!(".{name}.lock")))?;
        let contended = fs2::lock_contended_error().raw_os_error();
        let started = Instant::now();
        loop {
            match FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(Self { file }),
                Err(error) if error.raw_os_error() == contended => {
                    if started.elapsed() >= PREPARATION_LOCK_WAIT {
                        return Err(RuntimeError::Busy);
                    }
                    thread::sleep(PREPARATION_LOCK_POLL);
                }
                Err(error) => return Err(RuntimeError::Io(error)),
            }
        }
    }
}

impl Drop for PreparationLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// One entry of the bundled archive, with a lowercase `/`-separated path.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ArchiveEntry {
    relative: String,
    size: u64,
    directory: bool,
}

/// The archive's central directory: every file the runtime must contain and
/// its size. Reading it does not decompress anything.
///
/// Extraction writes entries in order, so when two entries name the same
/// path (exactly, or differing only in letter case, which Windows treats as
/// one file) the later one is what ends up on disk. Only that one is kept, so
/// such an archive does not look damaged after every extraction.
fn archive_entries(archive: &Path) -> Result<Vec<ArchiveEntry>, RuntimeError> {
    let file = fs::File::open(archive).map_err(|_| RuntimeError::MissingBundle)?;
    let mut archive = ZipArchive::new(file).map_err(|_| RuntimeError::InvalidBundle)?;
    let mut entries: Vec<ArchiveEntry> = Vec::with_capacity(archive.len());
    let mut positions: HashMap<String, usize> = HashMap::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|_| RuntimeError::InvalidBundle)?;
        let Some(relative) = entry.enclosed_name() else {
            return Err(RuntimeError::InvalidBundle);
        };
        let entry = ArchiveEntry {
            relative: normalized_relative(&relative),
            size: entry.size(),
            directory: entry.is_dir(),
        };
        match positions.get(&entry.relative) {
            Some(&position) => entries[position] = entry,
            None => {
                positions.insert(entry.relative.clone(), entries.len());
                entries.push(entry);
            }
        }
    }
    Ok(entries)
}

fn normalized_relative(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Files and folders under `root`, keyed like `ArchiveEntry::relative`. File
/// sizes come from the directory listing, so no file is opened.
fn installed_files(root: &Path) -> io::Result<(HashMap<String, u64>, HashSet<String>)> {
    let mut files = HashMap::new();
    let mut directories = HashSet::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((directory, prefix)) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_lowercase();
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let kind = entry.file_type()?;
            if kind.is_dir() {
                directories.insert(relative.clone());
                pending.push((entry.path(), relative));
            } else if kind.is_file() {
                files.insert(relative, entry.metadata()?.len());
            }
        }
    }
    Ok((files, directories))
}

fn patched_core_header(manifest: &RuntimeManifest) -> String {
    format!(
        "data/packages/arduino/hardware/renesas_uno/{}/cores/arduino/usb/serialusb.h",
        manifest.renesas_uno_core.to_lowercase()
    )
}

fn runtime_is_valid(
    destination: &Path,
    expected_manifest: &RuntimeManifest,
    expected_files: Option<&[ArchiveEntry]>,
) -> bool {
    runtime_problem(destination, expected_manifest, expected_files).is_none()
}

/// Why `destination` cannot be used as the runtime, or `None` if it can.
///
/// A matching manifest alone is not enough: a prior extraction may have been
/// interrupted, or antivirus may have removed or changed a tool. Every file
/// the archive lists must be present with its archived size (except files
/// rewritten by design). Without the archive, only the minimum layout is
/// checked.
fn runtime_problem(
    destination: &Path,
    expected_manifest: &RuntimeManifest,
    expected_files: Option<&[ArchiveEntry]>,
) -> Option<String> {
    inspect_runtime(destination, expected_manifest, expected_files).map(|problem| problem.reason)
}

/// Why a runtime folder does not match the bundled archive.
#[derive(Debug, Eq, PartialEq)]
struct RuntimeProblem {
    reason: String,
    /// The manifest, the required layout, or a listed file or folder is
    /// missing (or the folder could not be read). Otherwise everything is
    /// present and only a file's size differs from the archive.
    missing: bool,
}

impl RuntimeProblem {
    fn missing(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            missing: true,
        }
    }
}

/// See `runtime_problem`. A missing item is reported before any size change.
fn inspect_runtime(
    destination: &Path,
    expected_manifest: &RuntimeManifest,
    expected_files: Option<&[ArchiveEntry]>,
) -> Option<RuntimeProblem> {
    if read_manifest(destination).ok().as_ref() != Some(expected_manifest) {
        return Some(RuntimeProblem::missing(
            "runtime manifest missing or different",
        ));
    }
    if !destination.join("arduino-cli.exe").is_file()
        || !destination
            .join("data/packages/arduino/hardware/renesas_uno")
            .is_dir()
        || !destination.join("data/packages/arduino/tools").is_dir()
    {
        return Some(RuntimeProblem::missing("required runtime layout missing"));
    }
    let expected_files = expected_files?;
    let (files, directories) = match installed_files(destination) {
        Ok(listing) => listing,
        Err(error) => {
            return Some(RuntimeProblem::missing(format!(
                "runtime folder could not be listed: {error}"
            )))
        }
    };
    let patched_header = patched_core_header(expected_manifest);
    let mut first_size_change = None;
    for entry in expected_files {
        if entry.relative.is_empty() {
            continue;
        }
        if entry.directory {
            if !directories.contains(&entry.relative) {
                return Some(RuntimeProblem::missing(format!(
                    "folder missing: {}",
                    entry.relative
                )));
            }
            continue;
        }
        match files.get(&entry.relative) {
            None => {
                return Some(RuntimeProblem::missing(format!(
                    "file missing: {}",
                    entry.relative
                )))
            }
            Some(size)
                if *size != entry.size
                    && entry.relative != patched_header
                    && !REWRITTEN_RUNTIME_FILES.contains(&entry.relative.as_str()) =>
            {
                first_size_change.get_or_insert_with(|| {
                    format!(
                        "file size changed: {} ({size} bytes, expected {})",
                        entry.relative, entry.size
                    )
                });
            }
            Some(_) => {}
        }
    }
    first_size_change.map(|reason| RuntimeProblem {
        reason,
        missing: false,
    })
}

/// Extract to a sibling staging directory and validate it before replacing the
/// active runtime. If Windows refuses the final rename, restore the previous
/// runtime rather than leaving the installation without usable tools.
fn replace_runtime_from_archive(
    archive: &Path,
    destination: &Path,
    expected_manifest: &RuntimeManifest,
    expected_files: Option<&[ArchiveEntry]>,
    mode: Preparation,
) -> Result<(), RuntimeError> {
    let parent = destination.parent().ok_or(RuntimeError::InvalidBundle)?;
    fs::create_dir_all(parent)?;
    let destination_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(RuntimeError::InvalidBundle)?;
    // Fixed sibling names make an interrupted replacement recoverable on the
    // next launch. PID-suffixed backups could not be rediscovered after the
    // process that created them exited unexpectedly. The preparation lock
    // guarantees no other process is using these names right now.
    let staging = parent.join(format!(".{destination_name}-staging"));
    let backup = parent.join(format!(".{destination_name}-previous"));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    if backup.exists() {
        // A repair always extracts a fresh copy instead of trusting a backup.
        if mode == Preparation::ReuseValid
            && runtime_is_valid(&backup, expected_manifest, expected_files)
        {
            if destination.exists() {
                if destination.is_dir() {
                    fs::remove_dir_all(destination)?;
                } else {
                    fs::remove_file(destination)?;
                }
            }
            fs::rename(&backup, destination)?;
            return Ok(());
        }
        fs::remove_dir_all(&backup)?;
    }

    if let Err(error) = extract_archive(archive, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    // A fresh copy is refused only if something is missing. Every entry's
    // CRC-32 was checked while it was extracted, so a size that differs from
    // the archive listing can only come from an unusual archive layout; it
    // is logged for the maintainer rather than leaving the student with no
    // tools at all.
    match inspect_runtime(&staging, expected_manifest, expected_files) {
        Some(problem) if problem.missing => {
            crate::app_log::record(
                "WARN",
                &format!("ARDUINO_RUNTIME_EXTRACT_INVALID reason={}", problem.reason),
            );
            let _ = fs::remove_dir_all(&staging);
            return Err(RuntimeError::InvalidBundle);
        }
        Some(problem) => crate::app_log::record(
            "WARN",
            &format!(
                "ARDUINO_RUNTIME_EXTRACT_SIZE_MISMATCH reason={}",
                problem.reason
            ),
        ),
        None => {}
    }

    let had_existing_runtime = destination.exists();
    if had_existing_runtime {
        fs::rename(destination, &backup)?;
    }
    if let Err(error) = fs::rename(&staging, destination) {
        if had_existing_runtime {
            let _ = fs::rename(&backup, destination);
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(RuntimeError::Io(error));
    }
    if backup.exists() {
        // The replacement has committed. A stale backup is harmless and will
        // be recovered or removed on the next launch, so cleanup failure must
        // not make a usable runtime appear unavailable.
        if let Err(error) = fs::remove_dir_all(&backup) {
            crate::app_log::record(
                "WARN",
                &format!(
                    "ARDUINO_RUNTIME_BACKUP_CLEANUP_FAILED path={} detail={error}",
                    backup.display()
                ),
            );
        }
    }
    Ok(())
}

/// The pinned Arduino Renesas UNO core routes the C-library `_write` hook through
/// `Serial.write_raw`, but its native-USB `SerialUSB` class omits that member.
/// The standard UNO R4 WiFi build defines `NO_USB`, so the upstream omission is
/// normally hidden. WVU's controlled firmware deliberately uses the RA4M1 native
/// USB CDC path to avoid the proven intermittent RA4M1-to-ESP32 bridge stall.
///
/// Apply the smallest possible, version-pinned compatibility shim to the
/// app-owned extracted runtime. It is idempotent, never modifies an Arduino IDE
/// installation, and rejects an unexpected upstream core layout rather than
/// silently patching unknown third-party source.
fn apply_native_usb_core_compatibility_patch(
    destination: &Path,
    manifest: &RuntimeManifest,
) -> Result<(), RuntimeError> {
    if manifest.renesas_uno_core != "1.6.0" {
        return Err(RuntimeError::InvalidBundle);
    }
    let header = destination
        .join("data/packages/arduino/hardware/renesas_uno")
        .join(&manifest.renesas_uno_core)
        .join("cores/arduino/usb/SerialUSB.h");
    let source = fs::read_to_string(&header).map_err(|_| RuntimeError::InvalidBundle)?;
    if source.contains("size_t write_raw(uint8_t *p, size_t len)") {
        return Ok(());
    }
    let marker = "    virtual size_t write(const uint8_t *p, size_t len) override;";
    if !source.contains(marker) {
        return Err(RuntimeError::InvalidBundle);
    }
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let replacement = format!(
        "{marker}{newline}    // WVU native-USB compatibility shim for the pinned core's libc _write hook.{newline}    size_t write_raw(uint8_t *p, size_t len) {{ return write(p, len); }}"
    );
    fs::write(header, source.replacen(marker, &replacement, 1))?;
    Ok(())
}

fn read_manifest(root: &Path) -> Result<RuntimeManifest, RuntimeError> {
    read_manifest_file(&root.join("runtime-manifest.json"))
}

fn read_manifest_file(path: &Path) -> Result<RuntimeManifest, RuntimeError> {
    if !path.is_file() {
        return Err(RuntimeError::MissingBundle);
    }
    let manifest: RuntimeManifest = serde_json::from_slice(&fs::read(path)?)?;
    if manifest.runtime_schema != 1
        || manifest.arduino_cli.trim().is_empty()
        || manifest.renesas_uno_core.trim().is_empty()
    {
        return Err(RuntimeError::InvalidBundle);
    }
    Ok(manifest)
}

/// Extracts every entry. The zip reader checks each file's CRC-32 as it is
/// read, so a corrupted archive member fails here instead of being installed.
fn extract_archive(archive: &Path, destination: &Path) -> Result<(), RuntimeError> {
    let archive = fs::File::open(archive).map_err(|_| RuntimeError::MissingBundle)?;
    let mut archive = ZipArchive::new(archive).map_err(|_| RuntimeError::InvalidBundle)?;
    fs::create_dir_all(destination)?;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|_| RuntimeError::InvalidBundle)?;
        let Some(relative) = file.enclosed_name() else {
            return Err(RuntimeError::InvalidBundle);
        };
        let output = destination.join(relative);
        if file.is_dir() {
            fs::create_dir_all(output)?;
        } else {
            let parent = output.parent().ok_or(RuntimeError::InvalidBundle)?;
            fs::create_dir_all(parent)?;
            let mut destination_file = fs::File::create(output)?;
            io::copy(&mut file, &mut destination_file)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::mpsc;
    use tempfile::tempdir;
    use zip::{write::SimpleFileOptions, ZipWriter};

    const MANIFEST: &str = r#"{"runtime_schema":1,"arduino_cli":"1.5.2-rc.1","renesas_uno_core":"1.6.0","bundle_revision":1}"#;
    const SERIAL_USB_HEADER: &str =
        "data/packages/arduino/hardware/renesas_uno/1.6.0/cores/arduino/usb/SerialUSB.h";
    const BOSSAC: &str = "data/packages/arduino/tools/bossac/1.9.1-arduino5/bossac.exe";

    /// Writes a small runtime archive and its manifest into `source`.
    fn write_runtime_archive(source: &Path) -> (PathBuf, PathBuf) {
        let manifest_path = source.join("arduino-runtime-manifest.json");
        fs::write(&manifest_path, MANIFEST).unwrap_or_else(|error| panic!("{error}"));
        let archive_path = source.join("arduino-runtime.zip");
        let mut archive = ZipWriter::new(
            fs::File::create(&archive_path).unwrap_or_else(|error| panic!("{error}")),
        );
        let options = SimpleFileOptions::default();
        archive
            .add_directory("data/packages/arduino/tools/", options)
            .unwrap_or_else(|error| panic!("{error}"));
        for (name, bytes) in [
            ("runtime-manifest.json", MANIFEST.as_bytes()),
            ("arduino-cli.exe", b"test".as_slice()),
            (
                "data/packages/arduino/hardware/renesas_uno/1.6.0/.keep",
                b"".as_slice(),
            ),
            (
                SERIAL_USB_HEADER,
                b"class SerialUSB {\n    virtual size_t write(const uint8_t *p, size_t len) override;\n    using Print::write;\n};\n".as_slice(),
            ),
            (BOSSAC, b"bossac test binary".as_slice()),
            ("data/inventory.yaml", b"installation:\n".as_slice()),
        ] {
            archive
                .start_file(name, options)
                .unwrap_or_else(|error| panic!("{error}"));
            archive
                .write_all(bytes)
                .unwrap_or_else(|error| panic!("{error}"));
        }
        archive.finish().unwrap_or_else(|error| panic!("{error}"));
        (archive_path, manifest_path)
    }

    fn manifest() -> RuntimeManifest {
        serde_json::from_str(MANIFEST).unwrap_or_else(|error| panic!("{error}"))
    }

    fn entries(archive: &Path) -> Vec<ArchiveEntry> {
        archive_entries(archive).unwrap_or_else(|error| panic!("{error}"))
    }

    #[test]
    fn runtime_bootstrap_is_idempotent_and_generates_an_app_owned_config() {
        let source = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let destination = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (archive_path, manifest_path) = write_runtime_archive(source.path());
        let root = destination.path().join("runtime");
        let first = prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        let second = prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(first.config_file, second.config_file);
        assert!(first.config_file.is_file());
        assert!(fs::read_to_string(first.config_file)
            .unwrap_or_default()
            .contains("directories:"));
        let serial_usb = root.join(SERIAL_USB_HEADER);
        assert!(fs::read_to_string(serial_usb)
            .unwrap_or_default()
            .contains("size_t write_raw(uint8_t *p, size_t len)"));
        // The patched header and the generated config do not count as damage.
        assert_eq!(
            runtime_problem(&root, &manifest(), Some(&entries(&archive_path))),
            None
        );
    }

    #[test]
    fn matching_manifest_with_incomplete_runtime_is_repaired() {
        let source = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let destination = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (archive_path, manifest_path) = write_runtime_archive(source.path());

        let root = destination.path().join("runtime");
        fs::create_dir_all(&root).unwrap_or_else(|error| panic!("{error}"));
        fs::write(root.join("runtime-manifest.json"), MANIFEST)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!root.join("arduino-cli.exe").exists());

        let prepared = prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(prepared.executable.is_file());
        assert!(runtime_is_valid(
            &root,
            &read_manifest_file(&manifest_path).unwrap_or_else(|error| panic!("{error}")),
            Some(&entries(&archive_path))
        ));
    }

    #[test]
    fn a_runtime_with_a_missing_or_changed_tool_is_extracted_again() {
        let source = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let destination = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (archive_path, manifest_path) = write_runtime_archive(source.path());
        let root = destination.path().join("runtime");
        prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        let expected = entries(&archive_path);

        // For example antivirus quarantined the upload tool: the manifest and
        // the old presence-only layout check would still pass.
        fs::remove_file(root.join(BOSSAC)).unwrap_or_else(|error| panic!("{error}"));
        let problem = runtime_problem(&root, &manifest(), Some(&expected))
            .unwrap_or_else(|| panic!("a missing tool must be detected"));
        assert!(problem.contains("bossac.exe"), "{problem}");
        prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            fs::read(root.join(BOSSAC)).unwrap_or_default(),
            b"bossac test binary"
        );

        // A truncated tool is detected by its size.
        fs::write(root.join(BOSSAC), b"trunc").unwrap_or_else(|error| panic!("{error}"));
        let problem = runtime_problem(&root, &manifest(), Some(&expected))
            .unwrap_or_else(|| panic!("a changed tool must be detected"));
        assert!(problem.contains("size changed"), "{problem}");
        prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(runtime_problem(&root, &manifest(), Some(&expected)), None);

        // Files rewritten by design are not damage.
        fs::write(
            root.join("data/inventory.yaml"),
            b"installation:\n  id: generated\n",
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(runtime_problem(&root, &manifest(), Some(&expected)), None);
    }

    #[test]
    fn a_missing_file_is_reported_before_a_size_change() {
        let source = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let destination = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (archive_path, manifest_path) = write_runtime_archive(source.path());
        let root = destination.path().join("runtime");
        prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        let expected = entries(&archive_path);

        // Only a size differs: a fresh extraction with this result is kept
        // (and logged) instead of leaving the student without tools.
        fs::write(root.join(BOSSAC), b"trunc").unwrap_or_else(|error| panic!("{error}"));
        let changed = inspect_runtime(&root, &manifest(), Some(&expected))
            .unwrap_or_else(|| panic!("a changed tool must be detected"));
        assert!(!changed.missing, "{changed:?}");

        // Something is missing as well: that is what is reported, and a
        // fresh extraction like this is refused.
        fs::remove_file(root.join("data/inventory.yaml")).unwrap_or_else(|error| panic!("{error}"));
        let missing = inspect_runtime(&root, &manifest(), Some(&expected))
            .unwrap_or_else(|| panic!("a missing file must be detected"));
        assert!(missing.missing, "{missing:?}");
        assert!(missing.reason.contains("inventory.yaml"), "{missing:?}");
    }

    /// Windows treats names that differ only in letter case as one file, so
    /// the later archive entry is the one on disk.
    #[cfg(windows)]
    #[test]
    fn case_colliding_archive_entries_do_not_force_extraction_on_every_launch() {
        let source = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let destination = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (base_archive, manifest_path) = write_runtime_archive(source.path());
        let archive_path = source.path().join("case-colliding.zip");
        {
            let mut base = ZipArchive::new(
                fs::File::open(&base_archive).unwrap_or_else(|error| panic!("{error}")),
            )
            .unwrap_or_else(|error| panic!("{error}"));
            let mut archive = ZipWriter::new(
                fs::File::create(&archive_path).unwrap_or_else(|error| panic!("{error}")),
            );
            for index in 0..base.len() {
                let entry = base
                    .by_index_raw(index)
                    .unwrap_or_else(|error| panic!("{error}"));
                archive
                    .raw_copy_file(entry)
                    .unwrap_or_else(|error| panic!("{error}"));
            }
            let options = SimpleFileOptions::default();
            for (name, bytes) in [
                ("data/notes/README.txt", b"short".as_slice()),
                ("data/notes/readme.txt", b"a longer replacement".as_slice()),
            ] {
                archive
                    .start_file(name, options)
                    .unwrap_or_else(|error| panic!("{error}"));
                archive
                    .write_all(bytes)
                    .unwrap_or_else(|error| panic!("{error}"));
            }
            archive.finish().unwrap_or_else(|error| panic!("{error}"));
        }
        let expected = entries(&archive_path);
        let notes: Vec<_> = expected
            .iter()
            .filter(|entry| entry.relative == "data/notes/readme.txt")
            .collect();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].size, 20);

        let root = destination.path().join("runtime");
        prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        // The next launch finds the runtime valid and does not extract again.
        assert_eq!(runtime_problem(&root, &manifest(), Some(&expected)), None);
    }

    #[test]
    fn repair_waits_for_running_arduino_cli_commands() {
        let command = tools_in_use();
        // A repair (the exclusive holder) cannot start while a command runs.
        assert!(TOOLS_IN_USE.try_write().is_err());
        drop(command);
    }

    #[test]
    fn repair_extracts_again_even_when_the_runtime_looks_valid() {
        let source = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let destination = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (archive_path, manifest_path) = write_runtime_archive(source.path());
        let root = destination.path().join("runtime");
        prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        // Same size, different content: only a fresh extraction restores it.
        fs::write(root.join(BOSSAC), b"BOSSAC TEST BINARY")
            .unwrap_or_else(|error| panic!("{error}"));
        let stray = root.join("data/packages/arduino/tools/stray.tmp");
        fs::write(&stray, b"left behind").unwrap_or_else(|error| panic!("{error}"));
        let repaired = repair_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(repaired.config_file.is_file());
        assert_eq!(
            fs::read(root.join(BOSSAC)).unwrap_or_default(),
            b"bossac test binary"
        );
        assert!(!stray.exists());
        assert!(!destination.path().join(".runtime-previous").exists());
        assert!(!destination.path().join(".runtime-staging").exists());
    }

    #[test]
    fn a_failed_repair_keeps_the_working_runtime() {
        let source = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let destination = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (archive_path, manifest_path) = write_runtime_archive(source.path());
        let root = destination.path().join("runtime");
        prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        fs::remove_file(&archive_path).unwrap_or_else(|error| panic!("{error}"));
        let error = repair_from_archive(&archive_path, &manifest_path, &root)
            .err()
            .unwrap_or_else(|| panic!("repair needs the bundled archive"));
        assert!(matches!(error, RuntimeError::MissingBundle));
        assert!(!error.failure().repairable);
        assert!(root.join("arduino-cli.exe").is_file());
        assert!(root.join(BOSSAC).is_file());
    }

    #[test]
    fn interrupted_runtime_replacement_restores_the_valid_previous_runtime() {
        let source = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let destination = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (archive_path, manifest_path) = write_runtime_archive(source.path());

        let root = destination.path().join("runtime");
        prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        let backup = destination.path().join(".runtime-previous");
        fs::rename(&root, &backup).unwrap_or_else(|error| panic!("{error}"));

        let prepared = prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(prepared.executable.is_file());
        assert!(runtime_is_valid(
            &root,
            &read_manifest_file(&manifest_path).unwrap_or_else(|error| panic!("{error}")),
            Some(&entries(&archive_path))
        ));
        assert!(!backup.exists());
    }

    #[test]
    fn preparation_waits_for_another_copy_that_holds_the_lock() {
        let source = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let destination = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (archive_path, manifest_path) = write_runtime_archive(source.path());
        let root = destination.path().join("runtime");
        // Stands in for another application copy in the middle of extracting.
        let other_copy = PreparationLock::acquire(&root).unwrap_or_else(|error| panic!("{error}"));
        let (done, finished) = mpsc::channel();
        let worker_root = root.clone();
        let worker = thread::spawn(move || {
            let result = prepare_from_archive(&archive_path, &manifest_path, &worker_root);
            let _ = done.send(());
            result
        });
        assert!(
            finished.recv_timeout(Duration::from_millis(400)).is_err(),
            "preparation must wait while another copy holds the lock"
        );
        assert!(!root.exists());
        drop(other_copy);
        finished
            .recv_timeout(Duration::from_secs(30))
            .unwrap_or_else(|error| panic!("preparation did not resume: {error}"));
        let prepared = worker
            .join()
            .unwrap_or_else(|_| panic!("preparation thread panicked"))
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(prepared.executable.is_file());
    }

    #[test]
    fn generated_config_quotes_paths_with_yaml_special_characters() {
        let data = Path::new("C:\\Users\\Lab #3\\it's \"here\": x\\WVU-BMEG\\rt\\data");
        let downloads = Path::new("C:\\Users\\Lab #3\\downloads");
        let user = Path::new("C:\\Users\\Lab #3\\sketchbook");
        let text = cli_config_text(data, downloads, user);
        let values: Vec<String> = text
            .lines()
            .skip(1)
            .map(|line| {
                let (_, value) = line
                    .split_once(": ")
                    .unwrap_or_else(|| panic!("unexpected line {line}"));
                // A JSON string is a YAML double-quoted scalar; decoding it
                // proves the whole path, including ` #`, `: `, and quotes,
                // is read as one literal value.
                serde_json::from_str::<String>(value).unwrap_or_else(|error| panic!("{error}"))
            })
            .collect();
        assert_eq!(
            values,
            vec![
                "C:/Users/Lab #3/it's \"here\": x/WVU-BMEG/rt/data".to_owned(),
                "C:/Users/Lab #3/downloads".to_owned(),
                "C:/Users/Lab #3/sketchbook".to_owned(),
            ]
        );
    }

    #[test]
    fn a_runtime_under_a_profile_path_with_a_hash_is_prepared() {
        let source = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let destination = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (archive_path, manifest_path) = write_runtime_archive(source.path());
        let root = destination.path().join("Lab #3 it's").join("rt");
        let prepared = prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        let config = fs::read_to_string(&prepared.config_file).unwrap_or_default();
        let data_line = config
            .lines()
            .find(|line| line.trim_start().starts_with("data:"))
            .unwrap_or_else(|| panic!("no data line in {config}"));
        assert!(data_line.contains("Lab #3 it's/rt/data\""), "{data_line}");
    }

    #[test]
    fn tool_failures_point_to_repair_or_reinstall_in_plain_words() {
        for (error, repairable) in [
            (RuntimeError::MissingBundle, false),
            (RuntimeError::InvalidBundle, false),
            (RuntimeError::Io(io::Error::other("denied")), true),
            (RuntimeError::Busy, true),
        ] {
            let failure = error.failure();
            assert_eq!(failure.repairable, repairable);
            let advice = &failure.recommended_action;
            if repairable {
                assert!(advice.contains("Repair Arduino tools"), "{advice}");
            } else {
                assert!(advice.contains("Reinstall"), "{advice}");
            }
            assert!(!advice.contains("arduino-cli"), "{advice}");
        }
    }
}
