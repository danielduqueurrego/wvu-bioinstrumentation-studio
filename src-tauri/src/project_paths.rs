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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectFolderSettings {
    pub project_folder: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecordingDestination {
    pub project_folder: PathBuf,
    pub output_folder: String,
    pub effective_folder: PathBuf,
}

pub fn default_project_folder() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("Documents")
        .join("BMEG 420L")
}

fn settings_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(default_project_folder)
        .join(APP_DATA_FOLDER)
        .join(SETTINGS_FILE)
}

pub fn load_project_folder() -> Result<ProjectFolderSettings, String> {
    load_project_folder_at(&settings_path(), &default_project_folder())
}

fn load_project_folder_at(
    path: &Path,
    default_folder: &Path,
) -> Result<ProjectFolderSettings, String> {
    let project_folder = match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str::<ProjectFolderSettings>(&contents)
            .map(|settings| PathBuf::from(settings.project_folder))
            .unwrap_or_else(|_| default_folder.to_path_buf()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => default_folder.to_path_buf(),
        Err(error) => {
            return Err(format!(
                "could not read the Project folder setting: {error}"
            ))
        }
    };
    fs::create_dir_all(&project_folder)
        .map_err(|error| format!("could not create the Project folder: {error}"))?;
    Ok(ProjectFolderSettings {
        project_folder: project_folder.to_string_lossy().into_owned(),
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
    let settings = ProjectFolderSettings {
        project_folder: root.to_string_lossy().into_owned(),
    };
    let parent = path
        .parent()
        .ok_or_else(|| "could not determine the settings folder".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create the settings folder: {error}"))?;
    let temporary = path.with_extension("tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(&settings)
            .map_err(|error| format!("could not save the Project folder setting: {error}"))?,
    )
    .map_err(|error| format!("could not save the Project folder setting: {error}"))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("could not finalize the Project folder setting: {error}"))?;
    Ok(settings)
}

pub fn resolve_recording_destination(
    project_folder: &str,
    output_folder: &str,
) -> Result<RecordingDestination, String> {
    let project_folder = checked_project_root(project_folder)?;
    let output_folder = checked_relative_output_folder(output_folder)?;
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

pub fn checked_relative_output_folder(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    if value.contains('\0') {
        return Err("Output folder contains an invalid character".into());
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
    fn per_user_project_folder_setting_round_trips_without_machine_state() {
        let root = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let settings = root.path().join("user-a").join("project-folder.json");
        let project = root.path().join("Documents").join("BMEG 420L");
        let saved = save_project_folder_at(&settings, project.clone())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(saved.project_folder, project.to_string_lossy());
        let loaded = load_project_folder_at(&settings, &root.path().join("fallback"))
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(loaded, saved);
    }
}
