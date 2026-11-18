//! Per-user project-folder preferences and safe recording destinations.
//!
//! The installed application is machine-wide, but every recording destination
//! remains owned by the current Windows user.  Only an explicitly selected
//! project root and a relative trial folder are accepted here.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

const SETTINGS_FILE: &str = "project-folder.json";
const APP_DATA_FOLDER: &str = "WVU Bioinstrumentation Studio";
/// Longest accepted Output folder, so a recording path stays well inside the
/// classic 260-character Windows limit that Excel and Explorer still use.
pub const MAX_OUTPUT_FOLDER_CHARS: usize = 160;
/// Names Windows reserves for devices, with or without an extension.
const RESERVED_DEVICE_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// The Project folder shown to the page. `available` and `notice` are
/// additive: a missing removable or network folder is reported instead of
/// being recreated, and an unreadable saved setting is reported instead of
/// being silently replaced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectFolderSettings {
    pub project_folder: String,
    #[serde(default = "folder_available_default")]
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

fn folder_available_default() -> bool {
    true
}

/// The on-disk setting (unchanged format).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct StoredProjectFolder {
    project_folder: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecordingDestination {
    pub project_folder: PathBuf,
    pub output_folder: String,
    pub effective_folder: PathBuf,
}

const DEFAULT_PROJECT_FOLDER_NAME: &str = "BMEG 420L";

/// The default Project folder of earlier versions:
/// `%USERPROFILE%\Documents\BMEG 420L`.
fn legacy_default_project_folder() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("Documents")
        .join(DEFAULT_PROJECT_FOLDER_NAME)
}

/// Default Project folders to try when none was chosen, most preferred
/// first. The user's Documents Known Folder (so a OneDrive-redirected
/// Documents folder is used) comes first, except when the folder earlier
/// versions created already exists elsewhere: a user who never chose a
/// folder keeps saving next to their earlier recordings. The other location
/// remains as a fallback when the first cannot be created (for example a
/// redirected Documents folder on an unreachable network share).
fn default_project_folder_candidates(known: Option<PathBuf>, legacy: PathBuf) -> Vec<PathBuf> {
    let Some(known) = known else {
        return vec![legacy];
    };
    let same = known
        .to_string_lossy()
        .eq_ignore_ascii_case(&legacy.to_string_lossy());
    if same {
        vec![known]
    } else if legacy.is_dir() {
        vec![legacy, known]
    } else {
        vec![known, legacy]
    }
}

fn default_project_folders() -> Vec<PathBuf> {
    default_project_folder_candidates(
        known_documents_folder().map(|documents| documents.join(DEFAULT_PROJECT_FOLDER_NAME)),
        legacy_default_project_folder(),
    )
}

#[cfg(windows)]
fn known_documents_folder() -> Option<PathBuf> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    use windows_sys::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath, KF_FLAG_DEFAULT},
    };
    let mut raw: windows_sys::core::PWSTR = std::ptr::null_mut();
    // SAFETY: `raw` receives a NUL-terminated string allocated by the shell;
    // it is read only before CoTaskMemFree, which is called whether or not
    // the call succeeded, as the API requires.
    let path = unsafe {
        let result = SHGetKnownFolderPath(
            &FOLDERID_Documents,
            KF_FLAG_DEFAULT as u32,
            std::ptr::null_mut(),
            &mut raw,
        );
        let path = if result >= 0 && !raw.is_null() {
            let mut length = 0usize;
            while *raw.add(length) != 0 {
                length += 1;
            }
            Some(PathBuf::from(OsString::from_wide(
                std::slice::from_raw_parts(raw, length),
            )))
        } else {
            None
        };
        if !raw.is_null() {
            CoTaskMemFree(raw as *const core::ffi::c_void);
        }
        path
    };
    path.filter(|path| path.is_absolute())
}

#[cfg(not(windows))]
fn known_documents_folder() -> Option<PathBuf> {
    None
}

/// True when the folder's volume is a fixed local disk (or RAM disk), where
/// recreating a missing folder is harmless. A removable, network, or unknown
/// drive letter may now belong to a different device, so a missing folder
/// there is reported as unavailable instead.
#[cfg(windows)]
fn is_on_fixed_local_volume(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_RAMDISK: u32 = 6;
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return false;
    };
    let letter = match prefix.kind() {
        std::path::Prefix::Disk(letter) | std::path::Prefix::VerbatimDisk(letter) => letter,
        _ => return false,
    };
    let root: Vec<u16> = std::ffi::OsStr::new(&format!("{}:\\", char::from(letter)))
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `root` is a NUL-terminated UTF-16 string that outlives the call.
    let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
    matches!(kind, DRIVE_FIXED | DRIVE_RAMDISK)
}

#[cfg(not(windows))]
fn is_on_fixed_local_volume(_path: &Path) -> bool {
    true
}

fn settings_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        // The same last-resort location as earlier versions.
        .unwrap_or_else(legacy_default_project_folder)
        .join(APP_DATA_FOLDER)
        .join(SETTINGS_FILE)
}

pub fn load_project_folder() -> Result<ProjectFolderSettings, String> {
    load_project_folder_at(
        &settings_path(),
        &default_project_folders(),
        is_on_fixed_local_volume,
    )
}

fn read_stored_project_folder(bytes: &[u8]) -> Result<PathBuf, String> {
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let stored = serde_json::from_slice::<StoredProjectFolder>(text)
        .map_err(|error| format!("invalid JSON: {error}"))?;
    checked_project_root(&stored.project_folder)
}

fn load_project_folder_at(
    path: &Path,
    default_folders: &[PathBuf],
    fixed_volume: fn(&Path) -> bool,
) -> Result<ProjectFolderSettings, String> {
    let saved = match fs::read(path) {
        Ok(bytes) => match read_stored_project_folder(&bytes) {
            Ok(folder) => Some(folder),
            Err(detail) => {
                // Keep the unreadable file (it is replaced only when the user
                // chooses a folder) and say that the default is shown.
                crate::app_log::record(
                    "WARN",
                    &format!(
                        "PROJECT_FOLDER_SETTING_INVALID path={} detail={detail}",
                        path.display()
                    ),
                );
                let mut settings = default_project_folder_settings(default_folders)?;
                settings.notice = Some(
                    "The saved Project folder setting could not be read, so the default folder is shown. Choose Browse to select your Project folder."
                        .into(),
                );
                return Ok(settings);
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "could not read the Project folder setting: {error}"
            ))
        }
    };
    let Some(project_folder) = saved else {
        return default_project_folder_settings(default_folders);
    };
    let display = project_folder.to_string_lossy().into_owned();
    if project_folder.is_dir() {
        return Ok(ProjectFolderSettings {
            project_folder: display,
            available: true,
            notice: None,
        });
    }
    if !fixed_volume(&project_folder) {
        crate::app_log::record(
            "WARN",
            &format!("PROJECT_FOLDER_UNAVAILABLE path={display}"),
        );
        return Ok(ProjectFolderSettings {
            notice: Some(format!(
                "The Project folder {display} is not available. Reconnect the drive that holds it, or choose Browse to select another folder."
            )),
            project_folder: display,
            available: false,
        });
    }
    match fs::create_dir_all(&project_folder) {
        Ok(()) => Ok(ProjectFolderSettings {
            project_folder: display,
            available: true,
            notice: None,
        }),
        Err(error) => Ok(ProjectFolderSettings {
            notice: Some(format!(
                "The Project folder {display} could not be created ({error}). Choose Browse to select another folder."
            )),
            project_folder: display,
            available: false,
        }),
    }
}

/// The first default folder that exists or can be created. Nothing is saved:
/// the setting file is written only when the user chooses a folder.
fn default_project_folder_settings(
    default_folders: &[PathBuf],
) -> Result<ProjectFolderSettings, String> {
    let mut first_error = None;
    for folder in default_folders {
        match fs::create_dir_all(folder) {
            Ok(()) => {
                if first_error.is_some() {
                    crate::app_log::record(
                        "WARN",
                        &format!("PROJECT_FOLDER_DEFAULT_FALLBACK path={}", folder.display()),
                    );
                }
                return Ok(ProjectFolderSettings {
                    project_folder: folder.to_string_lossy().into_owned(),
                    available: true,
                    notice: None,
                });
            }
            Err(error) => {
                crate::app_log::record(
                    "WARN",
                    &format!(
                        "PROJECT_FOLDER_DEFAULT_UNAVAILABLE path={} detail={error}",
                        folder.display()
                    ),
                );
                first_error.get_or_insert(error);
            }
        }
    }
    Err(match first_error {
        Some(error) => format!("could not create the Project folder: {error}"),
        None => "could not determine a default Project folder".into(),
    })
}

pub fn save_project_folder(project_folder: &str) -> Result<ProjectFolderSettings, String> {
    let root = checked_project_root(project_folder)?;
    fs::create_dir_all(&root)
        .map_err(|error| format!("could not create the selected Project folder: {error}"))?;
    ensure_writable(&root)?;
    save_project_folder_at(&settings_path(), root)
}

fn save_project_folder_at(path: &Path, root: PathBuf) -> Result<ProjectFolderSettings, String> {
    let stored = StoredProjectFolder {
        project_folder: root.to_string_lossy().into_owned(),
    };
    let parent = path
        .parent()
        .ok_or_else(|| "could not determine the settings folder".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create the settings folder: {error}"))?;
    // An unreadable previous setting is kept beside the new one, not lost.
    if let Ok(bytes) = fs::read(path) {
        if read_stored_project_folder(&bytes).is_err() {
            let kept = path.with_file_name(format!(
                "project-folder.unreadable-{}.json",
                chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ")
            ));
            if let Err(error) = fs::copy(path, &kept) {
                crate::app_log::record(
                    "WARN",
                    &format!(
                        "PROJECT_FOLDER_SETTING_BACKUP_FAILED path={} detail={error}",
                        kept.display()
                    ),
                );
            }
        }
    }
    let temporary = path.with_extension("tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(&stored)
            .map_err(|error| format!("could not save the Project folder setting: {error}"))?,
    )
    .map_err(|error| format!("could not save the Project folder setting: {error}"))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("could not finalize the Project folder setting: {error}"))?;
    Ok(ProjectFolderSettings {
        project_folder: stored.project_folder,
        available: true,
        notice: None,
    })
}

pub fn resolve_recording_destination(
    project_folder: &str,
    output_folder: &str,
) -> Result<RecordingDestination, String> {
    resolve_recording_destination_with(project_folder, output_folder, is_on_fixed_local_volume)
}

fn resolve_recording_destination_with(
    project_folder: &str,
    output_folder: &str,
    fixed_volume: fn(&Path) -> bool,
) -> Result<RecordingDestination, String> {
    let project_folder = checked_project_root(project_folder)?;
    let output_folder = checked_relative_output_folder(output_folder)?;
    // A missing Project folder on a removable or network drive is not
    // recreated on whatever device now holds that drive letter.
    if !project_folder.is_dir() && !fixed_volume(&project_folder) {
        return Err(format!(
            "the Project folder {} is not available; reconnect the drive that holds it or choose another Project folder",
            project_folder.display()
        ));
    }
    let effective_folder = if output_folder.is_empty() {
        project_folder.clone()
    } else {
        project_folder.join(&output_folder)
    };
    fs::create_dir_all(&effective_folder).map_err(|error| {
        format!(
            "could not create the recording destination {}: {error}",
            effective_folder.display()
        )
    })?;
    ensure_writable(&effective_folder)?;
    Ok(RecordingDestination {
        project_folder,
        output_folder,
        effective_folder,
    })
}

/// Free bytes on the drive that holds `project_folder`, for the Storage
/// display before Start. Nothing is created or written. A Project folder
/// that does not exist yet on a fixed local drive is measured at its nearest
/// existing parent (Start would create it there); a missing folder on a
/// removable or network drive, an invalid path, or a failed query gives
/// `None` ("not yet checked"), never a guessed number.
pub fn project_folder_free_space(project_folder: &str) -> Option<u64> {
    project_folder_free_space_with(project_folder, is_on_fixed_local_volume, |path| {
        fs2::available_space(path)
    })
}

fn project_folder_free_space_with(
    project_folder: &str,
    fixed_volume: fn(&Path) -> bool,
    available_space: impl Fn(&Path) -> std::io::Result<u64>,
) -> Option<u64> {
    let root = checked_project_root(project_folder).ok()?;
    if root.is_dir() {
        return available_space(&root).ok();
    }
    if !fixed_volume(&root) {
        return None;
    }
    let existing = root.ancestors().find(|candidate| candidate.is_dir())?;
    available_space(existing).ok()
}

/// The recording file Show in folder may reveal: an existing file inside the
/// current Project folder. Both paths are canonicalized first, so `..`
/// segments, junctions, and links cannot point outside the Project folder,
/// and a sibling folder that merely shares a name prefix is not "inside".
pub fn checked_file_in_project_folder(path: &str, project_folder: &str) -> Result<PathBuf, String> {
    let value = path.trim();
    if value.is_empty() || value.contains('\0') {
        return Err("No recording file was given.".into());
    }
    let requested = PathBuf::from(value);
    if !requested.is_absolute() {
        return Err("The recording file path is not a full path.".into());
    }
    let project = checked_project_root(project_folder)?;
    let project = fs::canonicalize(&project).map_err(|_| {
        format!(
            "The Project folder {} is not available. Reconnect its drive or choose another Project folder.",
            project.display()
        )
    })?;
    let file = fs::canonicalize(&requested).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "The file no longer exists. It may have been moved, renamed, or deleted.".to_string()
        } else {
            format!("The file could not be found: {error}")
        }
    })?;
    if !file.is_file() {
        return Err("Only a recording file can be shown in its folder.".into());
    }
    if !file.starts_with(&project) {
        return Err("Only files inside the current Project folder can be shown.".into());
    }
    Ok(file)
}

/// `canonicalize` returns verbatim Windows paths (`\\?\C:\…`,
/// `\\?\UNC\server\share\…`), which File Explorer does not accept; this
/// gives the ordinary form of the same path.
pub fn explorer_path(path: &Path) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

fn checked_project_root(value: &str) -> Result<PathBuf, String> {
    let value = value.trim();
    if value.is_empty() || value.contains('\0') {
        return Err("choose a valid Project folder".into());
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("Project folder must be an absolute folder path".into());
    }
    Ok(path)
}

/// Why one Output-folder name cannot be created on Windows, if it cannot.
/// `project-folder.ts` mirrors these rules for the live preview.
fn output_segment_error(segment: &str) -> Option<String> {
    if segment
        .chars()
        .any(|character| character.is_control() || "<>:\"|?*".contains(character))
    {
        return Some(format!(
            "Output folder name \"{segment}\" cannot contain < > : \" | ? * or control characters"
        ));
    }
    if segment.ends_with('.') || segment.ends_with(' ') {
        return Some(format!(
            "Output folder name \"{segment}\" cannot end with a dot or a space"
        ));
    }
    let stem = segment
        .split('.')
        .next()
        .unwrap_or(segment)
        .trim_end_matches(' ');
    if RESERVED_DEVICE_NAMES
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        return Some(format!(
            "Output folder name \"{segment}\" is reserved by Windows; choose another name"
        ));
    }
    None
}

pub fn checked_relative_output_folder(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    if value.contains('\0') {
        return Err("Output folder contains an invalid character".into());
    }
    if value.chars().count() > MAX_OUTPUT_FOLDER_CHARS {
        return Err(format!(
            "Output folder must be {MAX_OUTPUT_FOLDER_CHARS} characters or fewer"
        ));
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("Output folder must stay inside the selected Project folder".into());
    }
    let components = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            Component::CurDir => None,
            _ => None,
        })
        .collect::<Vec<_>>();
    if let Some(error) = components
        .iter()
        .find_map(|segment| output_segment_error(segment))
    {
        return Err(error);
    }
    if components.is_empty() {
        return Ok(String::new());
    }
    Ok(components.join("\\"))
}

fn ensure_writable(path: &Path) -> Result<(), String> {
    let probe = path.join(format!(".bmeg-write-check-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|error| format!("the recording destination is not writable: {error}"))?;
    file.write_all(b"ok")
        .map_err(|error| format!("the recording destination is not writable: {error}"))?;
    drop(file);
    fs::remove_file(probe)
        .map_err(|error| format!("could not finish the recording destination check: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn always_fixed(_path: &Path) -> bool {
        true
    }
    fn never_fixed(_path: &Path) -> bool {
        false
    }

    #[test]
    fn free_space_is_read_without_creating_the_project_folder() {
        let dir = tempdir().unwrap();
        let existing = dir.path().to_string_lossy().into_owned();
        let missing = dir.path().join("BMEG 420L").join("Lab6");
        let missing_text = missing.to_string_lossy().into_owned();
        let measured = |path: &Path| -> std::io::Result<u64> {
            // Only existing folders are measured.
            assert!(path.is_dir(), "{} was measured", path.display());
            Ok(1_234)
        };
        assert_eq!(
            project_folder_free_space_with(&existing, always_fixed, measured),
            Some(1_234)
        );
        // A not-yet-created folder on a fixed drive is measured at its parent.
        assert_eq!(
            project_folder_free_space_with(&missing_text, always_fixed, measured),
            Some(1_234)
        );
        assert!(!missing.exists());
        // A missing folder on a removable drive is unknown, not guessed.
        assert_eq!(
            project_folder_free_space_with(&missing_text, never_fixed, measured),
            None
        );
        assert_eq!(
            project_folder_free_space_with("relative\\folder", always_fixed, measured),
            None
        );
        assert_eq!(
            project_folder_free_space_with(&existing, always_fixed, |_| Err(
                std::io::Error::other("query failed")
            )),
            None
        );
        assert!(project_folder_free_space(&existing).is_some());
    }

    fn write_file(path: &Path) {
        fs::create_dir_all(path.parent().unwrap_or_else(|| panic!("no parent")))
            .unwrap_or_else(|error| panic!("{error}"));
        fs::write(path, b"BMEG").unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    fn show_in_folder_accepts_only_existing_files_inside_the_project_folder() {
        let root = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let project = root.path().join("Project");
        let recording = project.join("Lab6").join("Trial1").join("run.bmeg");
        let at_root = project.join("run.csv");
        write_file(&recording);
        write_file(&at_root);
        let project_text = project.to_string_lossy().into_owned();

        let accepted = checked_file_in_project_folder(&recording.to_string_lossy(), &project_text)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            accepted,
            fs::canonicalize(&recording).unwrap_or_else(|error| panic!("{error}"))
        );
        assert!(checked_file_in_project_folder(&at_root.to_string_lossy(), &project_text).is_ok());
        // Surrounding whitespace from a copied path is ignored.
        assert!(checked_file_in_project_folder(
            &format!("  {}  ", recording.to_string_lossy()),
            &project_text
        )
        .is_ok());
    }

    #[test]
    fn show_in_folder_refuses_files_outside_the_project_folder() {
        let root = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let project = root.path().join("Project");
        let outside = root.path().join("Elsewhere").join("secret.txt");
        // A sibling folder whose name merely starts with the Project folder's name.
        let sibling = root.path().join("Project2").join("run.bmeg");
        fs::create_dir_all(&project).unwrap_or_else(|error| panic!("{error}"));
        write_file(&outside);
        write_file(&sibling);
        let project_text = project.to_string_lossy().into_owned();

        for refused in [
            outside.to_string_lossy().into_owned(),
            sibling.to_string_lossy().into_owned(),
            // `..` cannot climb out once the path is canonicalized.
            project
                .join("..")
                .join("Elsewhere")
                .join("secret.txt")
                .to_string_lossy()
                .into_owned(),
        ] {
            let error = checked_file_in_project_folder(&refused, &project_text)
                .expect_err("a file outside the Project folder must be refused");
            assert!(
                error.contains("inside the current Project folder"),
                "{error}"
            );
        }
    }

    #[test]
    fn show_in_folder_refuses_missing_files_folders_and_relative_paths() {
        let root = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let project = root.path().join("Project");
        let folder = project.join("Lab6");
        fs::create_dir_all(&folder).unwrap_or_else(|error| panic!("{error}"));
        let project_text = project.to_string_lossy().into_owned();

        let missing = checked_file_in_project_folder(
            &project.join("deleted.bmeg").to_string_lossy(),
            &project_text,
        )
        .expect_err("a missing file must be refused");
        assert!(missing.contains("no longer exists"), "{missing}");
        assert!(checked_file_in_project_folder(&folder.to_string_lossy(), &project_text).is_err());
        assert!(checked_file_in_project_folder("Lab6\\run.bmeg", &project_text).is_err());
        assert!(checked_file_in_project_folder("", &project_text).is_err());
        assert!(checked_file_in_project_folder("C:\\run\0.bmeg", &project_text).is_err());

        // A Project folder that is gone (for example an unplugged USB drive).
        let gone = root.path().join("Unplugged");
        let file = project.join("run.bmeg");
        write_file(&file);
        assert!(
            checked_file_in_project_folder(&file.to_string_lossy(), &gone.to_string_lossy())
                .is_err()
        );
    }

    /// A directory junction needs no special privilege on Windows (unlike a
    /// symbolic link), so this always runs there.
    #[cfg(windows)]
    #[test]
    fn show_in_folder_follows_junctions_and_refuses_one_that_leaves_the_project_folder() {
        let root = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let project = root.path().join("Project");
        let outside = root.path().join("Elsewhere");
        let inside = project.join("Lab6");
        write_file(&outside.join("secret.bmeg"));
        write_file(&inside.join("run.bmeg"));
        let junction = |link: &Path, target: &Path| {
            let status = std::process::Command::new("cmd")
                .arg("/C")
                .arg("mklink")
                .arg("/J")
                .arg(link)
                .arg(target)
                .output()
                .unwrap_or_else(|error| panic!("mklink could not run: {error}"));
            assert!(
                status.status.success(),
                "mklink /J failed: {}",
                String::from_utf8_lossy(&status.stderr)
            );
        };
        let escape = project.join("Shortcut");
        let alias = project.join("Alias");
        junction(&escape, &outside);
        junction(&alias, &inside);
        let project_text = project.to_string_lossy().into_owned();

        let refused = checked_file_in_project_folder(
            &escape.join("secret.bmeg").to_string_lossy(),
            &project_text,
        )
        .expect_err("a junction must not lead outside the Project folder");
        assert!(
            refused.contains("inside the current Project folder"),
            "{refused}"
        );

        let accepted = checked_file_in_project_folder(
            &alias.join("run.bmeg").to_string_lossy(),
            &project_text,
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            accepted,
            fs::canonicalize(inside.join("run.bmeg")).unwrap_or_else(|error| panic!("{error}"))
        );
    }

    /// A file symbolic link needs Developer Mode or an elevated account on
    /// Windows; without that privilege the check is skipped, not failed.
    #[cfg(windows)]
    #[test]
    fn show_in_folder_refuses_a_file_link_that_points_outside_the_project_folder() {
        let root = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let project = root.path().join("Project");
        let target = root.path().join("Elsewhere").join("secret.bmeg");
        fs::create_dir_all(&project).unwrap_or_else(|error| panic!("{error}"));
        write_file(&target);
        let link = project.join("run.bmeg");
        if let Err(error) = std::os::windows::fs::symlink_file(&target, &link) {
            eprintln!(
                "skipped: creating a symbolic link needs a privilege this account lacks ({error})"
            );
            return;
        }
        let refused =
            checked_file_in_project_folder(&link.to_string_lossy(), &project.to_string_lossy())
                .expect_err("a link must not lead outside the Project folder");
        assert!(
            refused.contains("inside the current Project folder"),
            "{refused}"
        );
    }

    #[test]
    fn explorer_receives_ordinary_paths_not_verbatim_ones() {
        assert_eq!(
            explorer_path(Path::new(r"\\?\C:\BMEG 420L\run.bmeg")),
            PathBuf::from(r"C:\BMEG 420L\run.bmeg")
        );
        assert_eq!(
            explorer_path(Path::new(r"\\?\UNC\server\share\BMEG\run.bmeg")),
            PathBuf::from(r"\\server\share\BMEG\run.bmeg")
        );
        assert_eq!(
            explorer_path(Path::new(r"C:\BMEG\run.bmeg")),
            PathBuf::from(r"C:\BMEG\run.bmeg")
        );
    }

    #[test]
    fn output_folder_is_relative_and_can_be_nested() {
        assert_eq!(checked_relative_output_folder("").unwrap(), "");
        assert_eq!(
            checked_relative_output_folder("Participant01\\Trial03").unwrap(),
            "Participant01\\Trial03"
        );
        assert!(checked_relative_output_folder("C:\\recordings").is_err());
        assert!(checked_relative_output_folder("..\\outside").is_err());
    }

    #[test]
    fn output_folder_rejects_names_windows_cannot_create() {
        for invalid in [
            "NUL",
            "con",
            "Trial\\COM1",
            "lpt9.txt",
            "AUX .data",
            "Trial:1",
            "Trial1.",
            "Trial1 \\Run",
            "Run<1>",
            "Run|1",
            "Run?",
            "Run*",
            "Run\"1\"",
            "Run\u{7}",
            "C:foo",
            "a:b",
        ] {
            assert!(
                checked_relative_output_folder(invalid).is_err(),
                "{invalid:?} must be rejected"
            );
        }
        for valid in [
            "Participant 01\\Trial.03",
            "COM10",
            "CONSOLE",
            "nul_run",
            ".\\Trial",
        ] {
            assert!(
                checked_relative_output_folder(valid).is_ok(),
                "{valid:?} must be accepted"
            );
        }
        assert!(checked_relative_output_folder(&"a".repeat(MAX_OUTPUT_FOLDER_CHARS + 1)).is_err());
    }

    #[test]
    fn destination_is_created_under_the_project_folder() {
        let root = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let destination = resolve_recording_destination(
            &root.path().join("Project").to_string_lossy(),
            "Lab6\\Trial1",
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert!(destination.effective_folder.is_dir());
        assert!(destination
            .effective_folder
            .starts_with(&destination.project_folder));
    }

    #[test]
    fn missing_project_folder_on_a_removable_drive_is_not_recreated_at_start() {
        let root = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let missing = root.path().join("USB").join("BMEG");
        assert!(resolve_recording_destination_with(
            &missing.to_string_lossy(),
            "Trial1",
            never_fixed
        )
        .is_err());
        assert!(!missing.exists());
        // On a fixed local disk a missing folder is still created.
        assert!(resolve_recording_destination_with(
            &missing.to_string_lossy(),
            "Trial1",
            always_fixed
        )
        .is_ok());
    }

    #[test]
    fn per_user_project_folder_setting_round_trips_without_machine_state() {
        let root = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let settings = root.path().join("user-a").join("project-folder.json");
        let project = root.path().join("Documents").join("BMEG 420L");
        let saved = save_project_folder_at(&settings, project.clone())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(saved.project_folder, project.to_string_lossy());
        let loaded =
            load_project_folder_at(&settings, &[root.path().join("fallback")], always_fixed)
                .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(loaded, saved);
        assert!(loaded.available);
        // The stored file keeps its original one-field format.
        let stored = fs::read_to_string(&settings).unwrap_or_default();
        assert!(!stored.contains("available"));
    }

    #[test]
    fn saved_folder_on_a_missing_removable_drive_is_reported_not_recreated() {
        let root = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let settings = root.path().join("project-folder.json");
        let usb = root.path().join("E").join("BMEG");
        save_project_folder_at(&settings, usb.clone()).unwrap_or_else(|error| panic!("{error}"));
        let loaded =
            load_project_folder_at(&settings, &[root.path().join("fallback")], never_fixed)
                .unwrap_or_else(|error| panic!("{error}"));
        assert!(!loaded.available);
        assert_eq!(loaded.project_folder, usb.to_string_lossy());
        assert!(loaded
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("not available")));
        assert!(!usb.exists());
        // The saved choice is kept for when the drive is reconnected.
        assert!(fs::read_to_string(&settings)
            .unwrap_or_default()
            .contains("BMEG"));
    }

    #[test]
    fn unreadable_setting_is_reported_kept_and_backed_up_when_replaced() {
        let root = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let settings = root.path().join("project-folder.json");
        fs::write(&settings, b"{ not json").unwrap_or_else(|error| panic!("{error}"));
        let fallback = root.path().join("fallback");
        let loaded =
            load_project_folder_at(&settings, std::slice::from_ref(&fallback), always_fixed)
                .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(loaded.project_folder, fallback.to_string_lossy());
        assert!(loaded.notice.is_some());
        assert_eq!(
            fs::read(&settings).unwrap_or_default(),
            b"{ not json".to_vec(),
            "the unreadable setting must not be silently replaced"
        );
        save_project_folder_at(&settings, root.path().join("Chosen"))
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(fs::read_dir(root.path())
            .unwrap_or_else(|error| panic!("{error}"))
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("project-folder.unreadable-")));
    }

    #[test]
    fn default_project_folders_are_absolute_documents_subfolders() {
        let folders = default_project_folders();
        assert!(!folders.is_empty());
        for folder in &folders {
            assert!(folder.is_absolute(), "{}", folder.display());
            assert!(folder.ends_with("BMEG 420L"));
        }
        let listed = |expected: PathBuf| {
            folders.iter().any(|folder| {
                folder
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&expected.to_string_lossy())
            })
        };
        #[cfg(windows)]
        if let Some(documents) = known_documents_folder() {
            assert!(listed(documents.join("BMEG 420L")));
        }
        assert!(listed(legacy_default_project_folder()));
    }

    #[test]
    fn an_existing_folder_from_earlier_versions_stays_the_default() {
        let root = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let known = root
            .path()
            .join("OneDrive")
            .join("Documents")
            .join("BMEG 420L");
        let legacy = root.path().join("Documents").join("BMEG 420L");
        let settings = root.path().join("project-folder.json");

        // A new user (no folder from an earlier version) gets the Known Folder.
        let fresh = default_project_folder_candidates(Some(known.clone()), legacy.clone());
        assert_eq!(fresh, vec![known.clone(), legacy.clone()]);
        let loaded = load_project_folder_at(&settings, &fresh, always_fixed)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(loaded.project_folder, known.to_string_lossy());
        assert!(!settings.exists(), "a default is never saved as a choice");

        // A user whose earlier recordings are in the old default keeps it.
        fs::remove_dir_all(root.path().join("OneDrive")).unwrap_or_else(|error| panic!("{error}"));
        fs::create_dir_all(&legacy).unwrap_or_else(|error| panic!("{error}"));
        let existing = default_project_folder_candidates(Some(known.clone()), legacy.clone());
        assert_eq!(existing, vec![legacy.clone(), known.clone()]);
        let loaded = load_project_folder_at(&settings, &existing, always_fixed)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(loaded.project_folder, legacy.to_string_lossy());
        assert!(!known.exists());

        // Without redirection both are the same folder.
        assert_eq!(
            default_project_folder_candidates(Some(legacy.clone()), legacy.clone()),
            vec![legacy.clone()]
        );
        assert_eq!(
            default_project_folder_candidates(None, legacy.clone()),
            vec![legacy]
        );
    }

    #[test]
    fn an_unreachable_known_folder_falls_back_to_the_local_default() {
        let root = tempdir().unwrap_or_else(|error| panic!("{error}"));
        // A file where the redirected Documents folder should be: it cannot
        // be created, like an offline network share.
        let blocked = root.path().join("Share");
        fs::write(&blocked, b"not a folder").unwrap_or_else(|error| panic!("{error}"));
        let known = blocked.join("Documents").join("BMEG 420L");
        let legacy = root.path().join("Documents").join("BMEG 420L");
        let candidates = default_project_folder_candidates(Some(known), legacy.clone());
        let loaded = load_project_folder_at(
            &root.path().join("project-folder.json"),
            &candidates,
            always_fixed,
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(loaded.project_folder, legacy.to_string_lossy());
        assert!(loaded.available);
        assert!(legacy.is_dir());
    }
}
