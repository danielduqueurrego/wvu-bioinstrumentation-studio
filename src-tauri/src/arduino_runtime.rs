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
//!
//! The runtime root must stay shorter than `RUNTIME_ROOT_LENGTH_LIMIT`
//! characters for the bundled compiler; `resolve_runtime_root` picks a
//! private `%ProgramData%` folder when the usual one is too long.
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
/// CLI configuration and Arduino CLI's own inventory file. If an archive
/// lists them they must still be present, but their archived size is not
/// compared. Runtime revision 4 ships neither (scripts/build_arduino_runtime.ps1
/// leaves them out), so this only matters for an older archive. The
/// version-pinned core header patch is handled by `patched_core_header`.
const REWRITTEN_RUNTIME_FILES: &[&str] = &["arduino-cli.yaml", "data/inventory.yaml"];
/// GCC 7-2017q4, the bundled arm-none-eabi toolchain, builds its C++ multilib
/// include paths from its own folder with unresolved `..` segments, so a long
/// runtime folder pushes them past the Windows path limit. Measured on
/// 2026-11-18 with runtime revision 4 and the reference sketch: a runtime root
/// of 59 characters compiles; from 60 characters on GCC stops with a missing
/// header, such as "bits/c++config.h: No such file or directory" (also
/// "bits/c++allocator.h" or "bits/cpu_defines.h"). Revision 3 has the same
/// compiler and limit. The usual root,
/// `C:\Users\<profile folder>\AppData\Local\WVU-BMEG\rt`, is 35 characters
/// plus the profile folder name, so it reaches the limit with profile folder
/// names longer than 24 characters. A runtime root must be shorter than this;
/// `resolve_runtime_root` moves a longer one to `%ProgramData%`.
pub const RUNTIME_ROOT_LENGTH_LIMIT: usize = 60;

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
    /// No usable runtime folder is shorter than `RUNTIME_ROOT_LENGTH_LIMIT`.
    #[error(
        "The Arduino tools folder {} is {length} characters long; the included compiler needs fewer than {} characters, and no shorter folder (a private %ProgramData% folder) could be used.",
        .path.display(),
        RUNTIME_ROOT_LENGTH_LIMIT
    )]
    PathTooLong { path: PathBuf, length: usize },
}

/// Tool failures that the page explains with its own text
/// (`ARDUINO_TOOLS_FAILURE_MESSAGES` in `src/lib/messages.ts`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArduinoToolsFailureCategory {
    /// The Windows profile path is too long for the bundled compiler and no
    /// shorter folder could be used (`RuntimeError::PathTooLong`).
    RuntimePathTooLong,
}

/// The student-facing form of a tool-preparation problem, returned by the
/// prepare and repair commands. Technical details stay in Advanced details.
#[derive(Clone, Debug, Serialize)]
pub struct ArduinoToolsFailure {
    /// Set for failures with their own text on the page; `null` otherwise.
    pub category: Option<ArduinoToolsFailureCategory>,
    pub title: String,
    pub explanation: String,
    pub recommended_action: String,
    pub technical_details: String,
    /// Whether Repair Arduino tools (extracting the included tools again) can
    /// fix this. A missing or damaged installation needs a reinstall instead.
    pub repairable: bool,
    /// Set by a failed Repair: the tools this run was already using are still
    /// in place on disk (the repair was refused, or failed before or while
    /// swapping and was rolled back), so firmware operations keep working.
    /// False when that is unknown or not the case.
    pub previous_runtime_kept: bool,
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
            category: None,
            title: title.into(),
            explanation: explanation.into(),
            recommended_action: recommended_action.into(),
            technical_details: technical_details.into(),
            repairable,
            previous_runtime_kept: false,
        }
    }

    fn with_category(mut self, category: ArduinoToolsFailureCategory) -> Self {
        self.category = Some(category);
        self
    }

    /// Records whether the tools in use before a failed Repair are still in
    /// place (see `active_runtime_intact`).
    pub fn after_failed_repair(mut self) -> Self {
        self.previous_runtime_kept = active_runtime_intact();
        self
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
            // Repair extracts to the same folder, so it cannot help.
            Self::PathTooLong { .. } => ArduinoToolsFailure::new(
                "Arduino tools cannot be set up for this Windows account",
                "The folder name of this Windows user account is too long for the included Arduino compiler, and no shorter folder could be used on this computer.",
                "Ask your IT support or instructor for help and show them Advanced details. Simulator recordings still work.",
                self.to_string(),
                false,
            )
            .with_category(ArduinoToolsFailureCategory::RuntimePathTooLong),
        }
    }
}

pub fn active_runtime() -> Option<&'static RuntimePaths> {
    ACTIVE_RUNTIME.get()
}

/// Whether the tools this run prepared are still in place on disk: their
/// manifest still names the same versions, and the CLI, its configuration,
/// and the UNO R4 core and tools are present. A failed Repair can leave them
/// untouched (refused, or failed before the swap), restore them (rollback),
/// or lose them (a rollback that also failed); this checks the folder itself.
pub fn active_runtime_intact() -> bool {
    ACTIVE_RUNTIME.get().is_some_and(runtime_paths_intact)
}

fn runtime_paths_intact(paths: &RuntimePaths) -> bool {
    let root = &paths.root;
    let data = root.join("data");
    let Ok(manifest) = read_manifest(root) else {
        return false;
    };
    manifest.arduino_cli == paths.cli_version
        && manifest.renesas_uno_core == paths.core_version
        && paths.executable.is_file()
        && data.join("packages/arduino/tools").is_dir()
        // Preparation finished: the native-USB shim is applied and the CLI
        // configuration names this runtime's own folders.
        && fs::read_to_string(native_usb_core_header(root, &manifest))
            .is_ok_and(|header| header.contains(NATIVE_USB_SHIM_SIGNATURE))
        && fs::read_to_string(&paths.config_file).is_ok_and(|config| {
            config == cli_config_text(&data, &root.join("downloads"), &root.join("sketchbook"))
        })
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
        // multilib discovery on Windows (`RUNTIME_ROOT_LENGTH_LIMIT`). Keep
        // its app-owned extraction root deliberately short while ordinary
        // application data stays in Tauri's normal directory.
        let destination = match dev_local_data_root() {
            // A development override keeps everything inside the folder the
            // developer chose, even one too long for the compiler (the
            // Simulator and labs do not need it); see start_dev_sandbox.ps1.
            Some(root) => {
                let preferred = root.join("WVU-BMEG").join("rt");
                if !fits_compiler_limit(&preferred) {
                    crate::app_log::record(
                        "WARN",
                        &format!(
                            "ARDUINO_RUNTIME_ROOT_TOO_LONG_DEV length={} path={} (compiles need a root shorter than {RUNTIME_ROOT_LENGTH_LIMIT} characters)",
                            path_length(&preferred),
                            preferred.display()
                        ),
                    );
                }
                preferred
            }
            None => {
                let local_data_root = app
                    .path()
                    .app_local_data_dir()
                    .map_err(|_| RuntimeError::MissingBundle)?
                    .parent()
                    .map(Path::to_path_buf)
                    .ok_or(RuntimeError::InvalidBundle)?;
                let preferred = local_data_root.join("WVU-BMEG").join("rt");
                resolve_runtime_root(&preferred, true)?.into_path()
            }
        };
        Ok(Self {
            archive: resource_root.join("arduino-runtime.zip"),
            manifest: resource_root.join("arduino-runtime-manifest.json"),
            destination,
        })
    }
}

/// The folder the runtime is extracted to, and why it was chosen.
///
/// There is deliberately no 8.3 short-path step: Arduino CLI expands a short
/// runtime path back to the long one before it runs the compiler. Measured on
/// 2026-11-18 with `examples/runtime_smoke.rs`: a 63-character root given as
/// its 23-character short form still failed with "bits/c++config.h: No such
/// file or directory", and Arduino CLI reported the long platform path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeRoot {
    /// The usual folder, `%LOCALAPPDATA%\WVU-BMEG\rt`, which is short enough.
    Preferred(PathBuf),
    /// `%ProgramData%\WVU-BMEG-<user id>\rt`, a folder only this Windows user
    /// (with SYSTEM and Administrators) can open.
    SharedFallback(PathBuf),
}

impl RuntimeRoot {
    pub fn path(&self) -> &Path {
        match self {
            Self::Preferred(path) | Self::SharedFallback(path) => path,
        }
    }

    pub fn into_path(self) -> PathBuf {
        match self {
            Self::Preferred(path) | Self::SharedFallback(path) => path,
        }
    }
}

/// The length of a path as Windows counts it (UTF-16 code units).
pub fn path_length(path: &Path) -> usize {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().count()
    }
    #[cfg(not(windows))]
    {
        path.as_os_str().to_string_lossy().chars().count()
    }
}

fn fits_compiler_limit(path: &Path) -> bool {
    path_length(path) < RUNTIME_ROOT_LENGTH_LIMIT
}

/// Chooses the runtime folder for `preferred` (normally
/// `%LOCALAPPDATA%\WVU-BMEG\rt`): that folder when it is shorter than
/// `RUNTIME_ROOT_LENGTH_LIMIT`; otherwise, when `allow_shared_fallback`, a
/// private per-user folder under `%ProgramData%` if that is short enough and
/// can be prepared; otherwise `RuntimeError::PathTooLong`, which the page
/// explains to the student instead of a compiler error. Callers that must not
/// write outside their own folder (`examples/runtime_smoke.rs`) pass
/// `allow_shared_fallback = false`.
pub fn resolve_runtime_root(
    preferred: &Path,
    allow_shared_fallback: bool,
) -> Result<RuntimeRoot, RuntimeError> {
    let root = choose_runtime_root(
        preferred,
        || {
            if allow_shared_fallback {
                shared_runtime_root()
            } else {
                None
            }
        },
        prepare_shared_runtime_root,
    );
    match &root {
        Ok(RuntimeRoot::Preferred(_)) => {}
        Ok(RuntimeRoot::SharedFallback(path)) => crate::app_log::record(
            "INFO",
            &format!(
                "ARDUINO_RUNTIME_ROOT reason=program_data preferred_length={} preferred={} path={}",
                path_length(preferred),
                preferred.display(),
                path.display()
            ),
        ),
        Err(error) => crate::app_log::record(
            "WARN",
            &format!("ARDUINO_RUNTIME_ROOT_TOO_LONG detail={error}"),
        ),
    }
    root
}

/// The decision of `resolve_runtime_root`, with the file-system work
/// injected so tests never touch a real profile or `%ProgramData%`.
/// `shared_fallback` names the `%ProgramData%` folder, or `None` when it is
/// not allowed or unknown; `prepare_shared` creates it privately and checks
/// that it is writable.
fn choose_runtime_root(
    preferred: &Path,
    shared_fallback: impl FnOnce() -> Option<PathBuf>,
    prepare_shared: impl FnOnce(&Path) -> bool,
) -> Result<RuntimeRoot, RuntimeError> {
    if fits_compiler_limit(preferred) {
        return Ok(RuntimeRoot::Preferred(preferred.to_path_buf()));
    }
    if let Some(shared) = shared_fallback().filter(|shared| fits_compiler_limit(shared)) {
        if prepare_shared(&shared) {
            return Ok(RuntimeRoot::SharedFallback(shared));
        }
    }
    Err(RuntimeError::PathTooLong {
        path: preferred.to_path_buf(),
        length: path_length(preferred),
    })
}

/// `%ProgramData%\WVU-BMEG-<id>\rt`, where `<id>` is the first 12 hex digits
/// of the SHA-256 of the current user's SID: stable for the account, short,
/// and distinct per user. Each user's folder sits directly in
/// `%ProgramData%`, which lets users create folders but not remove or change
/// another user's, so no other account controls a folder above it.
fn shared_runtime_root() -> Option<PathBuf> {
    use sha2::Digest;
    let program_data = platform::program_data_folder()?;
    let sid = platform::current_user_sid()?;
    let digest = sha2::Sha256::digest(sid.as_bytes());
    let id: String = digest
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Some(program_data.join(format!("WVU-BMEG-{id}")).join("rt"))
}

/// Creates the folder that holds `root` (and the runtime's lock and staging
/// siblings) for this user only, or accepts it if it already is, and checks
/// that it is writable. `%ProgramData%`'s default permissions would let every
/// local user add files inside it (for example a `platform.local.txt` that
/// changes the compile commands), so the folder gets a protected access list:
/// this user, SYSTEM and Administrators. An existing folder is used only if
/// this user (or SYSTEM or Administrators) owns it, its access list is
/// protected and allows no other account, and it is not a link.
fn prepare_shared_runtime_root(root: &Path) -> bool {
    let Some(folder) = root.parent() else {
        return false;
    };
    match platform::ensure_private_folder(folder) {
        Ok(()) => folder_is_writable(folder),
        Err(error) => {
            crate::app_log::record(
                "WARN",
                &format!(
                    "ARDUINO_RUNTIME_SHARED_ROOT_REFUSED path={} detail={error}",
                    folder.display()
                ),
            );
            false
        }
    }
}

/// Creates, writes and removes a probe file. A probe that already exists (left
/// by another process with the same ID) is not this call's file: it is left
/// alone and the check fails.
fn folder_is_writable(folder: &Path) -> bool {
    use std::io::Write;
    let probe = folder.join(format!(".write-test-{}", std::process::id()));
    let Ok(mut file) = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    else {
        return false;
    };
    let written = file.write_all(b"ok");
    drop(file);
    let removed = fs::remove_file(&probe);
    written.is_ok() && removed.is_ok()
}

/// Windows-only file-system helpers for `resolve_runtime_root`.
#[cfg(windows)]
mod platform {
    use std::{
        ffi::OsString,
        io,
        os::windows::ffi::{OsStrExt, OsStringExt},
        path::{Path, PathBuf},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree, ERROR_ALREADY_EXISTS, HANDLE},
        Security::{
            AclSizeInformation,
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
            },
            GetAce, GetAclInformation, GetSecurityDescriptorControl, GetTokenInformation,
            TokenUser, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION,
            DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
            SECURITY_ATTRIBUTES, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
        },
        Storage::FileSystem::{CreateDirectoryW, FILE_ATTRIBUTE_REPARSE_POINT},
        System::{
            Com::CoTaskMemFree,
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
        UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath, KF_FLAG_DEFAULT},
    };

    /// Accounts accepted besides the user, as the owner of an existing
    /// private folder and in its access list: LocalSystem and the
    /// Administrators group (an elevated copy of the application creates
    /// folders owned by Administrators).
    const TRUSTED_ACCOUNTS: [&str; 2] = ["S-1-5-18", "S-1-5-32-544"];
    /// `ACE_HEADER::AceType` values (winnt.h; windows-sys keeps them in
    /// `Win32_System_SystemServices`, which this crate does not enable).
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const ACCESS_DENIED_ACE_TYPE: u8 = 1;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// Reads a NUL-terminated UTF-16 string.
    ///
    /// # Safety
    /// `text` must point to a NUL-terminated UTF-16 string.
    unsafe fn read_wide(text: *const u16) -> OsString {
        let mut length = 0usize;
        // SAFETY: the caller guarantees a terminating NUL.
        while unsafe { *text.add(length) } != 0 {
            length += 1;
        }
        // SAFETY: `length` units before the NUL were just read.
        OsString::from_wide(unsafe { std::slice::from_raw_parts(text, length) })
    }

    /// `%ProgramData%` from the Known Folder API.
    pub fn program_data_folder() -> Option<PathBuf> {
        let mut raw: windows_sys::core::PWSTR = ptr::null_mut();
        // SAFETY: `raw` receives a NUL-terminated string allocated by the
        // shell; it is read before CoTaskMemFree, which runs whether or not
        // the call succeeded, as the API requires.
        let path = unsafe {
            let result = SHGetKnownFolderPath(
                &FOLDERID_ProgramData,
                KF_FLAG_DEFAULT as u32,
                ptr::null_mut(),
                &mut raw,
            );
            let path = (result >= 0 && !raw.is_null()).then(|| PathBuf::from(read_wide(raw)));
            if !raw.is_null() {
                CoTaskMemFree(raw as *const core::ffi::c_void);
            }
            path
        };
        path.filter(|path| path.is_absolute())
    }

    /// The SID text (S-1-5-21-...) of `sid`.
    ///
    /// # Safety
    /// `sid` must point to a valid SID.
    unsafe fn sid_text(sid: PSID) -> Option<String> {
        let mut text: windows_sys::core::PWSTR = ptr::null_mut();
        // SAFETY: on success `text` is a NUL-terminated string allocated
        // with LocalAlloc, freed below.
        unsafe {
            if ConvertSidToStringSidW(sid, &mut text) == 0 || text.is_null() {
                return None;
            }
            let value = read_wide(text).into_string().ok();
            LocalFree(text.cast());
            value
        }
    }

    /// The SID text of the user this process runs as.
    pub fn current_user_sid() -> Option<String> {
        // SAFETY: the token handle is closed before returning; the buffer is
        // u64-aligned (TOKEN_USER holds a pointer) and sized by the first
        // GetTokenInformation call; the SID it points to lives in the buffer.
        unsafe {
            let mut token: HANDLE = ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return None;
            }
            let mut needed = 0u32;
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut needed);
            let mut buffer = vec![0u64; (needed as usize).div_ceil(8).max(1)];
            let ok = GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 8) as u32,
                &mut needed,
            );
            CloseHandle(token);
            if ok == 0 {
                return None;
            }
            let user = &*(buffer.as_ptr() as *const TOKEN_USER);
            sid_text(user.User.Sid)
        }
    }

    /// The descriptor of a private folder: owned by `user`, with a protected
    /// access list that gives `user`, SYSTEM and Administrators full control,
    /// inherited by everything inside.
    fn private_folder_sddl(user: &str) -> String {
        format!("O:{user}D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)")
    }

    /// Creates `folder` with a protected access list (this user, SYSTEM,
    /// Administrators; inherited by everything inside), or checks that an
    /// existing one is owned by this user (or SYSTEM or Administrators), has
    /// a protected access list that allows no other account, and is not a
    /// link.
    pub fn ensure_private_folder(folder: &Path) -> io::Result<()> {
        let user = current_user_sid().ok_or_else(|| io::Error::other("user SID unavailable"))?;
        if create_folder_with_sddl(folder, &private_folder_sddl(&user))? {
            return Ok(());
        }
        verify_private_folder(folder, &user)
    }

    /// Creates `folder` with the security descriptor `sddl`. `Ok(false)` when
    /// something of that name already exists (it is left as it is).
    pub(super) fn create_folder_with_sddl(folder: &Path, sddl: &str) -> io::Result<bool> {
        let sddl: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated; the descriptor is LocalAlloc'd
        // and freed after CreateDirectoryW.
        unsafe {
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let attributes = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            let created = CreateDirectoryW(wide(folder).as_ptr(), &attributes);
            let error = io::Error::last_os_error();
            LocalFree(descriptor);
            if created == 0 && error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
                return Err(error);
            }
            Ok(created != 0)
        }
    }

    /// The SID texts that the allow entries of `dacl` name. Deny entries only
    /// take access away and are skipped; any other entry type is refused, and
    /// so is a missing access list, which lets everyone in.
    ///
    /// # Safety
    /// `dacl` must be null or point to a valid access list.
    unsafe fn allowed_accounts(dacl: *const ACL) -> io::Result<Vec<String>> {
        if dacl.is_null() {
            return Err(io::Error::other("no access list (everyone has access)"));
        }
        let mut information = ACL_SIZE_INFORMATION {
            AceCount: 0,
            AclBytesInUse: 0,
            AclBytesFree: 0,
        };
        // SAFETY: `dacl` is a valid access list (the caller's guarantee);
        // every entry GetAce returns starts with an ACE_HEADER, and an
        // ACCESS_ALLOWED_ACE's SID starts at its `SidStart` field.
        unsafe {
            if GetAclInformation(
                dacl,
                (&mut information as *mut ACL_SIZE_INFORMATION).cast(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let mut accounts = Vec::new();
            for index in 0..information.AceCount {
                let mut entry: *mut core::ffi::c_void = ptr::null_mut();
                if GetAce(dacl, index, &mut entry) == 0 || entry.is_null() {
                    return Err(io::Error::last_os_error());
                }
                match (*(entry as *const ACE_HEADER)).AceType {
                    ACCESS_DENIED_ACE_TYPE => {}
                    ACCESS_ALLOWED_ACE_TYPE => {
                        let allowed = entry as *const ACCESS_ALLOWED_ACE;
                        let sid = ptr::addr_of!((*allowed).SidStart) as PSID;
                        accounts.push(
                            sid_text(sid)
                                .ok_or_else(|| io::Error::other("access entry unreadable"))?,
                        );
                    }
                    other => {
                        return Err(io::Error::other(format!(
                            "unexpected access entry type {other}"
                        )))
                    }
                }
            }
            Ok(accounts)
        }
    }

    fn verify_private_folder(folder: &Path, user: &str) -> io::Result<()> {
        use std::os::windows::fs::MetadataExt;
        let metadata = std::fs::symlink_metadata(folder)?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::other("not a plain folder"));
        }
        let mut owner: PSID = ptr::null_mut();
        let mut dacl: *mut ACL = ptr::null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: on success `owner` and `dacl` point into `descriptor`, a
        // LocalAlloc'd block read here and freed before returning.
        let (owner_text, protected, allowed) = unsafe {
            let status = GetNamedSecurityInfoW(
                wide(folder).as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                ptr::null_mut(),
                &mut dacl,
                ptr::null_mut(),
                &mut descriptor,
            );
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            let owner_text = sid_text(owner);
            let mut control = 0u16;
            let mut revision = 0u32;
            let protected = GetSecurityDescriptorControl(descriptor, &mut control, &mut revision)
                != 0
                && control & SE_DACL_PROTECTED != 0;
            let allowed = allowed_accounts(dacl);
            LocalFree(descriptor);
            (owner_text, protected, allowed)
        };
        let owner_text = owner_text.ok_or_else(|| io::Error::other("owner unreadable"))?;
        if owner_text != user && !TRUSTED_ACCOUNTS.contains(&owner_text.as_str()) {
            return Err(io::Error::other(format!(
                "owned by another account ({owner_text})"
            )));
        }
        if !protected {
            return Err(io::Error::other("access list is not protected"));
        }
        if let Some(other) = allowed?
            .into_iter()
            .find(|account| account != user && !TRUSTED_ACCOUNTS.contains(&account.as_str()))
        {
            return Err(io::Error::other(format!(
                "access list allows another account ({other})"
            )));
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod platform {
    use std::{
        io,
        path::{Path, PathBuf},
    };

    pub fn program_data_folder() -> Option<PathBuf> {
        None
    }

    pub fn current_user_sid() -> Option<String> {
        None
    }

    pub fn ensure_private_folder(_folder: &Path) -> io::Result<()> {
        Err(io::Error::other("only available on Windows"))
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
    } else {
        // The active runtime is valid, so a backup or staging copy left by an
        // earlier replacement (for example when antivirus blocked its
        // cleanup) is no longer needed for recovery.
        remove_stale_replacement_copies(destination);
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
/// comment), `: `, or quotes is read literally. The runtime is offline: no
/// update notification, and no Arduino Cloud lookup of detected boards.
/// scripts/build_arduino_runtime.ps1 writes the same text for its proofs.
fn cli_config_text(data: &Path, downloads: &Path, user: &Path) -> String {
    format!(
        "directories:\n  data: {}\n  downloads: {}\n  user: {}\nupdater:\n  enable_notification: false\nnetwork:\n  cloud_api:\n    skip_board_detection_calls: true\n",
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

/// Best effort, under the preparation lock and only while the active runtime
/// is valid: removes the `.{name}-previous` and `.{name}-staging` siblings
/// that `replace_runtime_from_archive` uses. Each can hold a full toolchain.
fn remove_stale_replacement_copies(destination: &Path) {
    let (Some(parent), Some(name)) = (
        destination.parent(),
        destination.file_name().and_then(|name| name.to_str()),
    ) else {
        return;
    };
    for leftover in [
        parent.join(format!(".{name}-previous")),
        parent.join(format!(".{name}-staging")),
    ] {
        if !leftover.exists() {
            continue;
        }
        let removed = if leftover.is_dir() {
            fs::remove_dir_all(&leftover)
        } else {
            fs::remove_file(&leftover)
        };
        match removed {
            Ok(()) => crate::app_log::record(
                "INFO",
                &format!(
                    "ARDUINO_RUNTIME_STALE_COPY_REMOVED path={}",
                    leftover.display()
                ),
            ),
            Err(error) => crate::app_log::record(
                "WARN",
                &format!(
                    "ARDUINO_RUNTIME_STALE_COPY_CLEANUP_FAILED path={} detail={error}",
                    leftover.display()
                ),
            ),
        }
    }
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
        // The replacement has committed. A stale backup is harmless and is
        // removed on the next launch that finds the runtime valid
        // (`remove_stale_replacement_copies`), so cleanup failure must not
        // make a usable runtime appear unavailable.
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
    let header = native_usb_core_header(destination, manifest);
    let source = fs::read_to_string(&header).map_err(|_| RuntimeError::InvalidBundle)?;
    if source.contains(NATIVE_USB_SHIM_SIGNATURE) {
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

/// The member the compatibility shim adds to `SerialUSB`.
const NATIVE_USB_SHIM_SIGNATURE: &str = "size_t write_raw(uint8_t *p, size_t len)";

fn native_usb_core_header(destination: &Path, manifest: &RuntimeManifest) -> PathBuf {
    destination
        .join("data/packages/arduino/hardware/renesas_uno")
        .join(&manifest.renesas_uno_core)
        .join("cores/arduino/usb/SerialUSB.h")
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

    const MANIFEST: &str = r#"{"runtime_schema":1,"arduino_cli":"1.6.0","renesas_uno_core":"1.6.0","bundle_revision":1}"#;
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
    fn a_failed_repair_reports_whether_the_tools_in_use_are_still_in_place() {
        let source = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let destination = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (archive_path, manifest_path) = write_runtime_archive(source.path());
        let root = destination.path().join("runtime");
        let prepared = prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        // A repair that failed before or during the swap left them in place.
        assert!(runtime_paths_intact(&prepared));
        // A rollback that also failed leaves no runtime folder at all.
        let moved = destination.path().join(".runtime-previous");
        fs::rename(&root, &moved).unwrap_or_else(|error| panic!("{error}"));
        assert!(!runtime_paths_intact(&prepared));
        fs::rename(&moved, &root).unwrap_or_else(|error| panic!("{error}"));
        assert!(runtime_paths_intact(&prepared));
        // A swap that failed after extraction: the archive's own config, or
        // a core without the native-USB shim, is not a usable runtime.
        let config = fs::read(&prepared.config_file).unwrap_or_else(|error| panic!("{error}"));
        fs::write(&prepared.config_file, b"directories: {}\n")
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!runtime_paths_intact(&prepared));
        fs::write(&prepared.config_file, config).unwrap_or_else(|error| panic!("{error}"));
        let header = root.join(SERIAL_USB_HEADER);
        let patched = fs::read(&header).unwrap_or_else(|error| panic!("{error}"));
        fs::write(&header, b"class SerialUSB {};\n").unwrap_or_else(|error| panic!("{error}"));
        assert!(!runtime_paths_intact(&prepared));
        fs::write(&header, patched).unwrap_or_else(|error| panic!("{error}"));
        assert!(runtime_paths_intact(&prepared));
        // Other tool versions in the folder are not the tools in use. The CLI
        // and the core are both 1.6.0, so replace the CLI field only.
        let older_cli =
            MANIFEST.replace(r#""arduino_cli":"1.6.0""#, r#""arduino_cli":"1.5.2-rc.1""#);
        assert_ne!(older_cli, MANIFEST);
        assert!(older_cli.contains(r#""renesas_uno_core":"1.6.0""#));
        fs::write(root.join("runtime-manifest.json"), older_cli)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!runtime_paths_intact(&prepared));
        // Only a failed Repair reports it; preparation failures never do.
        assert!(!RuntimeError::Busy.failure().previous_runtime_kept);
        // The page reads it by this name (`commandErrorKeptPreviousTools`).
        let serialized = serde_json::to_value(RuntimeError::Busy.failure())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(serialized["previous_runtime_kept"], false);
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
    fn a_stale_backup_beside_a_valid_runtime_is_removed() {
        let source = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let destination = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (archive_path, manifest_path) = write_runtime_archive(source.path());
        let root = destination.path().join("runtime");
        prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        // An earlier replacement whose cleanup failed left both behind.
        let backup = destination.path().join(".runtime-previous");
        let staging = destination.path().join(".runtime-staging");
        for leftover in [&backup, &staging] {
            fs::create_dir_all(leftover.join("data")).unwrap_or_else(|error| panic!("{error}"));
            fs::write(leftover.join("data/tool.bin"), b"old tool")
                .unwrap_or_else(|error| panic!("{error}"));
        }
        let prepared = prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(prepared.executable.is_file());
        assert!(!backup.exists());
        assert!(!staging.exists());
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
            .take(3)
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
        // The offline settings that follow the three directories.
        assert!(text.starts_with("directories:\n  data: "), "{text}");
        assert!(
            text.ends_with(
                "\nupdater:\n  enable_notification: false\nnetwork:\n  cloud_api:\n    skip_board_detection_calls: true\n"
            ),
            "{text}"
        );
    }

    fn long_path(characters: usize) -> PathBuf {
        let base = "C:\\Users\\";
        let suffix = "\\AppData\\Local\\WVU-BMEG\\rt";
        let user = "u".repeat(characters - base.len() - suffix.len());
        PathBuf::from(format!("{base}{user}{suffix}"))
    }

    #[test]
    fn a_runtime_root_shorter_than_the_compiler_limit_is_used_as_is() {
        let preferred = long_path(RUNTIME_ROOT_LENGTH_LIMIT - 1);
        assert_eq!(path_length(&preferred), 59);
        let root = choose_runtime_root(
            &preferred,
            || panic!("no fallback needed"),
            |_| panic!("no fallback needed"),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(root, RuntimeRoot::Preferred(preferred));
    }

    #[test]
    fn a_long_runtime_root_uses_the_private_program_data_folder() {
        for length in [RUNTIME_ROOT_LENGTH_LIMIT, 80] {
            let preferred = long_path(length);
            let shared = PathBuf::from("C:\\ProgramData\\WVU-BMEG-0123456789ab\\rt");
            let offered = shared.clone();
            let mut prepared = None;
            let root = choose_runtime_root(
                &preferred,
                || Some(offered),
                |path| {
                    prepared = Some(path.to_path_buf());
                    true
                },
            )
            .unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(root.path(), shared);
            assert_eq!(root, RuntimeRoot::SharedFallback(shared.clone()));
            assert_eq!(prepared.as_deref(), Some(shared.as_path()));
        }
    }

    #[test]
    fn no_usable_short_folder_is_a_path_too_long_error_for_the_student() {
        let preferred = long_path(70);
        let cases: [(Option<PathBuf>, bool); 4] = [
            // runtime_smoke, or no %ProgramData% folder: no fallback.
            (None, true),
            // %ProgramData% itself is redirected to a long path.
            (Some(long_path(RUNTIME_ROOT_LENGTH_LIMIT)), true),
            // The folder exists but belongs to another account, or is not writable.
            (
                Some(PathBuf::from("C:\\ProgramData\\WVU-BMEG-0123456789ab\\rt")),
                false,
            ),
            (None, false),
        ];
        for (shared, preparable) in cases {
            let tried_long_fallback = shared
                .as_ref()
                .is_some_and(|path| path_length(path) >= RUNTIME_ROOT_LENGTH_LIMIT);
            let error = choose_runtime_root(
                &preferred,
                || shared,
                |_| {
                    assert!(
                        !tried_long_fallback,
                        "a too-long fallback is never prepared"
                    );
                    preparable
                },
            )
            .err()
            .unwrap_or_else(|| panic!("no folder is short enough"));
            assert!(
                matches!(&error, RuntimeError::PathTooLong { path, length } if *path == preferred && *length == 70),
                "{error:?}"
            );
            let failure = error.failure();
            assert_eq!(
                failure.category,
                Some(ArduinoToolsFailureCategory::RuntimePathTooLong)
            );
            assert!(!failure.repairable, "Repair extracts to the same folder");
            assert!(failure.recommended_action.contains("IT support"));
            assert!(!failure.recommended_action.contains("Repair"));
            assert!(failure.technical_details.contains("70 characters"));
            // The page reads the category by this name.
            let serialized =
                serde_json::to_value(&failure).unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(serialized["category"], "runtime_path_too_long");
        }
        // Other failures carry no category.
        let other = serde_json::to_value(RuntimeError::Busy.failure())
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(other["category"].is_null());
    }

    #[test]
    fn path_length_counts_utf16_units_like_windows() {
        assert_eq!(path_length(Path::new("C:\\Users\\Zoë")), 12);
        assert_eq!(path_length(&long_path(RUNTIME_ROOT_LENGTH_LIMIT)), 60);
    }

    #[test]
    fn a_short_runtime_root_resolves_without_touching_other_folders() {
        let folder = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let preferred = folder.path().join("rt");
        if path_length(&preferred) >= RUNTIME_ROOT_LENGTH_LIMIT {
            // A very long temporary folder: the decision tests cover it.
            return;
        }
        let root =
            resolve_runtime_root(&preferred, false).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(root.path(), preferred);
        assert!(
            !preferred.exists(),
            "resolving does not create the runtime folder"
        );
    }

    #[test]
    fn a_too_long_root_without_a_fallback_is_refused_before_anything_is_written() {
        let folder = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let mut preferred = folder.path().to_path_buf();
        while path_length(&preferred.join("rt")) < RUNTIME_ROOT_LENGTH_LIMIT {
            preferred.push("long-profile-folder-name");
        }
        let preferred = preferred.join("rt");
        let error = resolve_runtime_root(&preferred, false)
            .err()
            .unwrap_or_else(|| panic!("a root of 60 or more characters must be refused"));
        assert!(
            matches!(error, RuntimeError::PathTooLong { .. }),
            "{error:?}"
        );
        assert!(!preferred.exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_private_runtime_folder_is_created_for_this_user_only() {
        let folder = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let private = folder.path().join("WVU-BMEG-test");
        platform::ensure_private_folder(&private).unwrap_or_else(|error| panic!("{error}"));
        assert!(folder_is_writable(&private));
        // Accepted again: owned by this user, protected access list.
        platform::ensure_private_folder(&private).unwrap_or_else(|error| panic!("{error}"));
        assert!(prepare_shared_runtime_root(&private.join("rt")));
        // An ordinary folder inherits its parent's permissions: refused.
        let ordinary = folder.path().join("WVU-BMEG-ordinary");
        fs::create_dir(&ordinary).unwrap_or_else(|error| panic!("{error}"));
        let refused = platform::ensure_private_folder(&ordinary)
            .err()
            .unwrap_or_else(|| panic!("an unprotected folder must be refused"));
        assert!(refused.to_string().contains("not protected"), "{refused}");
        assert!(!prepare_shared_runtime_root(&ordinary.join("rt")));
        assert!(platform::current_user_sid().is_some_and(|sid| sid.starts_with("S-1-")));
        // The shared folder name depends only on the account.
        if let Some(root) = shared_runtime_root() {
            let name = root
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned();
            assert!(name.starts_with("WVU-BMEG-") && name.len() == 21, "{name}");
            assert_eq!(shared_runtime_root(), Some(root));
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_protected_folder_that_allows_another_account_is_refused() {
        let folder = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let user = platform::current_user_sid().unwrap_or_else(|| panic!("no user SID"));
        let own = format!("(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)");
        let create = |name: &str, dacl: &str| {
            let path = folder.path().join(name);
            let created = platform::create_folder_with_sddl(&path, &format!("O:{user}D:P{dacl}"))
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(created, "{name} already existed");
            path
        };
        // Protected and owned by this user, but Users may write, directly or
        // only through what is created inside.
        for (name, extra) in [
            ("WVU-BMEG-users", "(A;OICI;0x1301bf;;;BU)"),
            ("WVU-BMEG-inherit-only", "(A;OICIIO;FA;;;BU)"),
        ] {
            let shared = create(name, &format!("{own}{extra}"));
            let refused = platform::ensure_private_folder(&shared)
                .err()
                .unwrap_or_else(|| panic!("{name} allows Users and must be refused"));
            assert!(
                refused
                    .to_string()
                    .contains("allows another account (S-1-5-32-545)"),
                "{refused}"
            );
            assert!(!prepare_shared_runtime_root(&shared.join("rt")));
        }
        // A deny entry only takes access away (Guests, which this user is not).
        let denied = create("WVU-BMEG-deny", &format!("(D;OICI;FA;;;BG){own}"));
        platform::ensure_private_folder(&denied).unwrap_or_else(|error| panic!("{error}"));
        assert!(prepare_shared_runtime_root(&denied.join("rt")));
    }

    #[cfg(windows)]
    #[test]
    fn a_link_named_like_the_private_folder_is_refused() {
        let folder = tempdir().unwrap_or_else(|error| panic!("{error}"));
        // The link points at a folder that would itself be accepted.
        let target = folder.path().join("target");
        platform::ensure_private_folder(&target).unwrap_or_else(|error| panic!("{error}"));
        let link = folder.path().join("WVU-BMEG-link");
        // A directory junction needs no privilege (a symbolic link would).
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap_or_else(|error| panic!("cmd: {error}"));
        assert!(
            output.status.success(),
            "mklink /J failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let refused = platform::ensure_private_folder(&link)
            .err()
            .unwrap_or_else(|| panic!("a junction must be refused"));
        assert!(
            refused.to_string().contains("not a plain folder"),
            "{refused}"
        );
        assert!(!prepare_shared_runtime_root(&link.join("rt")));
        // Nothing was written through the link.
        let written = fs::read_dir(&target)
            .unwrap_or_else(|error| panic!("{error}"))
            .count();
        assert_eq!(written, 0);
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
