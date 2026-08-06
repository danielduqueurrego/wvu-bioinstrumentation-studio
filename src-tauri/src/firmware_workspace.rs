//! Single-file Arduino project storage for the Phase 2 firmware workspace.
//!
//! Templates live in this module (or, for the controlled reference, in the sole
//! repository sketch).  Student projects are always copies: this module never
//! writes a repository template while saving a project.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io::Write,
    path::{Component, Path, PathBuf},
};

pub const PROJECT_SCHEMA_VERSION: u32 = 1;
pub const UNO_R4_WIFI_FQBN: &str = "arduino:renesas_uno:unor4wifi";
const PROJECT_FILE: &str = "project.json";
const RECENT_FILE: &str = "recent_firmware_projects.json";
const MAX_RECENT_PROJECTS: usize = 10;

/// These bytes are copied exactly when a project is created from the controlled
/// reference. They deliberately originate from one repository source file.
const REFERENCE_TEMPLATE: &[u8] =
    include_bytes!("../../firmware/reference_unor4wifi/reference_unor4wifi.ino");

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateKind {
    BlankUnoR4Wifi,
    A0AcquisitionExample,
    WvuProtocolReference,
    SafeDigitalOutput,
    SerialDiagnostic,
}

impl TemplateKind {
    pub const ALL: [Self; 5] = [
        Self::BlankUnoR4Wifi,
        Self::A0AcquisitionExample,
        Self::WvuProtocolReference,
        Self::SafeDigitalOutput,
        Self::SerialDiagnostic,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::BlankUnoR4Wifi => "Blank UNO R4 WiFi sketch",
            Self::A0AcquisitionExample => "A0 acquisition example",
            Self::WvuProtocolReference => "WVU protocol reference firmware",
            Self::SafeDigitalOutput => "Safe digital-output example",
            Self::SerialDiagnostic => "Serial diagnostic example",
        }
    }

    pub fn verification_kind(self) -> FirmwareVerificationKind {
        match self {
            Self::WvuProtocolReference => FirmwareVerificationKind::WvuProtocolReference,
            Self::BlankUnoR4Wifi
            | Self::A0AcquisitionExample
            | Self::SafeDigitalOutput
            | Self::SerialDiagnostic => FirmwareVerificationKind::NonWvu,
        }
    }

    pub fn source_bytes(self) -> &'static [u8] {
        match self {
            Self::BlankUnoR4Wifi => BLANK_TEMPLATE.as_bytes(),
            Self::A0AcquisitionExample => A0_TEMPLATE.as_bytes(),
            Self::WvuProtocolReference => REFERENCE_TEMPLATE,
            Self::SafeDigitalOutput => SAFE_DIGITAL_TEMPLATE.as_bytes(),
            Self::SerialDiagnostic => SERIAL_DIAGNOSTIC_TEMPLATE.as_bytes(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirmwareVerificationKind {
    WvuProtocolReference,
    NonWvu,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FirmwareIdentity {
    pub protocol_version: String,
    pub firmware_build: u32,
    pub device_id: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProjectMetadata {
    pub schema_version: u32,
    pub project_name: String,
    pub created_utc: DateTime<Utc>,
    pub modified_utc: DateTime<Utc>,
    pub target_board: String,
    pub fqbn: String,
    pub source_filename: String,
    pub selected_com_port: Option<String>,
    pub template_origin: TemplateKind,
    pub verification_kind: FirmwareVerificationKind,
    pub lab_profile: Option<String>,
    pub notes: Option<String>,
    pub last_successful_compile_utc: Option<DateTime<Utc>>,
    pub last_successful_upload_utc: Option<DateTime<Utc>>,
    pub last_verified_firmware_identity: Option<FirmwareIdentity>,
    #[serde(default)]
    pub last_compile_source_hash: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub parent_folder: String,
    pub project_name: String,
    pub template: TemplateKind,
    pub notes: Option<String>,
    pub overwrite_confirmed: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SaveProjectRequest {
    pub project_folder: String,
    pub source: String,
    pub notes: Option<String>,
    pub selected_com_port: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SaveAsProjectRequest {
    pub source_project_folder: String,
    pub destination_parent_folder: String,
    pub destination_project_name: String,
    pub source: String,
    pub overwrite_confirmed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct FirmwareProject {
    pub project_folder: String,
    pub source_path: String,
    pub metadata_path: String,
    pub metadata: ProjectMetadata,
    pub source: String,
    pub source_hash: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct TemplateInfo {
    pub kind: TemplateKind,
    pub name: String,
    pub verification_kind: FirmwareVerificationKind,
    pub description: String,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("ProjectInvalid: {0}")]
    ProjectInvalid(String),
    #[error("UnsavedChanges: save the editor before compiling or uploading")]
    UnsavedChanges,
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("Metadata: {0}")]
    Metadata(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct FirmwareWorkspace {
    data_dir: PathBuf,
}

impl Default for FirmwareWorkspace {
    fn default() -> Self {
        Self::new(default_data_dir())
    }
}

impl FirmwareWorkspace {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    pub fn templates() -> Vec<TemplateInfo> {
        TemplateKind::ALL
            .into_iter()
            .map(|kind| TemplateInfo {
                kind,
                name: kind.display_name().into(),
                verification_kind: kind.verification_kind(),
                description: template_description(kind).into(),
            })
            .collect()
    }

    pub fn create_project(
        &self,
        request: CreateProjectRequest,
    ) -> Result<FirmwareProject, WorkspaceError> {
        validate_project_name(&request.project_name)?;
        let parent = checked_directory(&request.parent_folder)?;
        let project_folder = parent.join(&request.project_name);
        ensure_child_path(&parent, &project_folder)?;
        if project_folder.exists() {
            if !request.overwrite_confirmed {
                return Err(WorkspaceError::ProjectInvalid(format!(
                    "project folder already exists: {}; confirm overwrite to use it",
                    project_folder.display()
                )));
            }
            if project_folder.read_dir()?.next().is_some() {
                return Err(WorkspaceError::ProjectInvalid(
                    "refusing to overwrite a non-empty project folder".into(),
                ));
            }
        } else {
            fs::create_dir_all(&project_folder)?;
        }
        let source_filename = format!("{}.ino", request.project_name);
        let source_path = project_folder.join(&source_filename);
        let metadata_path = project_folder.join(PROJECT_FILE);
        atomic_write(&source_path, request.template.source_bytes())?;
        let now = Utc::now();
        let metadata = ProjectMetadata {
            schema_version: PROJECT_SCHEMA_VERSION,
            project_name: request.project_name,
            created_utc: now,
            modified_utc: now,
            target_board: "Arduino UNO R4 WiFi".into(),
            fqbn: UNO_R4_WIFI_FQBN.into(),
            source_filename,
            selected_com_port: None,
            template_origin: request.template,
            verification_kind: request.template.verification_kind(),
            lab_profile: None,
            notes: normalize_optional(request.notes),
            last_successful_compile_utc: None,
            last_successful_upload_utc: None,
            last_verified_firmware_identity: None,
            last_compile_source_hash: None,
        };
        write_metadata(&metadata_path, &metadata)?;
        self.open_project_path(&project_folder)
    }

    pub fn open_project(&self, project_folder: &str) -> Result<FirmwareProject, WorkspaceError> {
        self.open_project_path(&checked_directory(project_folder)?)
    }

    pub fn save_project(
        &self,
        request: SaveProjectRequest,
    ) -> Result<FirmwareProject, WorkspaceError> {
        let folder = checked_directory(&request.project_folder)?;
        let (mut metadata, source_path, metadata_path) = load_project_paths(&folder)?;
        ensure_utf8_source(&request.source)?;
        atomic_write(&source_path, request.source.as_bytes())?;
        metadata.modified_utc = Utc::now();
        metadata.notes = normalize_optional(request.notes);
        metadata.selected_com_port = normalize_optional(request.selected_com_port);
        write_metadata(&metadata_path, &metadata)?;
        self.open_project_path(&folder)
    }

    pub fn save_as_project(
        &self,
        request: SaveAsProjectRequest,
    ) -> Result<FirmwareProject, WorkspaceError> {
        let source_folder = checked_directory(&request.source_project_folder)?;
        let (source_metadata, _, _) = load_project_paths(&source_folder)?;
        validate_project_name(&request.destination_project_name)?;
        ensure_utf8_source(&request.source)?;
        let parent = checked_directory(&request.destination_parent_folder)?;
        let destination = parent.join(&request.destination_project_name);
        ensure_child_path(&parent, &destination)?;
        if destination.exists() {
            if !request.overwrite_confirmed {
                return Err(WorkspaceError::ProjectInvalid(
                    "destination exists; confirm overwrite before Save As".into(),
                ));
            }
            if destination.read_dir()?.next().is_some() {
                return Err(WorkspaceError::ProjectInvalid(
                    "refusing to overwrite a non-empty destination project".into(),
                ));
            }
        } else {
            fs::create_dir_all(&destination)?;
        }
        let source_filename = format!("{}.ino", request.destination_project_name);
        atomic_write(
            &destination.join(&source_filename),
            request.source.as_bytes(),
        )?;
        let now = Utc::now();
        let metadata = ProjectMetadata {
            schema_version: PROJECT_SCHEMA_VERSION,
            project_name: request.destination_project_name,
            created_utc: now,
            modified_utc: now,
            target_board: source_metadata.target_board,
            fqbn: source_metadata.fqbn,
            source_filename,
            selected_com_port: source_metadata.selected_com_port,
            template_origin: source_metadata.template_origin,
            verification_kind: source_metadata.verification_kind,
            lab_profile: source_metadata.lab_profile,
            notes: source_metadata.notes,
            last_successful_compile_utc: None,
            last_successful_upload_utc: None,
            last_verified_firmware_identity: None,
            last_compile_source_hash: None,
        };
        write_metadata(&destination.join(PROJECT_FILE), &metadata)?;
        self.open_project_path(&destination)
    }

    pub fn recent_projects(&self) -> Result<Vec<String>, WorkspaceError> {
        let path = self.data_dir.join(RECENT_FILE);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let bytes = fs::read(path)?;
        let entries: Vec<String> = serde_json::from_slice(&bytes)?;
        Ok(entries
            .into_iter()
            .filter(|entry| Path::new(entry).join(PROJECT_FILE).is_file())
            .collect())
    }

    pub fn restore_saved_source(&self, project_folder: &str) -> Result<String, WorkspaceError> {
        Ok(self.open_project(project_folder)?.source)
    }

    pub fn source_hash_for_project(&self, project_folder: &str) -> Result<String, WorkspaceError> {
        Ok(self.open_project(project_folder)?.source_hash)
    }

    pub fn update_compile_success(
        &self,
        project_folder: &str,
        source_hash: String,
    ) -> Result<(), WorkspaceError> {
        self.update_metadata(project_folder, |metadata| {
            metadata.last_successful_compile_utc = Some(Utc::now());
            metadata.last_compile_source_hash = Some(source_hash);
        })
    }

    pub fn update_upload_success(
        &self,
        project_folder: &str,
        port: String,
        identity: Option<FirmwareIdentity>,
    ) -> Result<(), WorkspaceError> {
        self.update_metadata(project_folder, |metadata| {
            metadata.last_successful_upload_utc = Some(Utc::now());
            metadata.selected_com_port = Some(port);
            metadata.last_verified_firmware_identity = identity;
        })
    }

    pub fn controlled_reference_source() -> Result<String, WorkspaceError> {
        String::from_utf8(REFERENCE_TEMPLATE.to_vec()).map_err(|_| {
            WorkspaceError::ProjectInvalid("controlled reference template is not UTF-8".into())
        })
    }

    fn open_project_path(&self, project_folder: &Path) -> Result<FirmwareProject, WorkspaceError> {
        let (metadata, source_path, metadata_path) = load_project_paths(project_folder)?;
        let source = fs::read_to_string(&source_path)?;
        ensure_utf8_source(&source)?;
        self.record_recent(project_folder)?;
        Ok(FirmwareProject {
            project_folder: project_folder.display().to_string(),
            source_path: source_path.display().to_string(),
            metadata_path: metadata_path.display().to_string(),
            metadata,
            source_hash: stable_hash(source.as_bytes()),
            source,
        })
    }

    fn record_recent(&self, project_folder: &Path) -> Result<(), WorkspaceError> {
        fs::create_dir_all(&self.data_dir)?;
        let path = self.data_dir.join(RECENT_FILE);
        let current = project_folder.display().to_string();
        let mut entries = self.recent_projects().unwrap_or_default();
        entries.retain(|entry| !entry.eq_ignore_ascii_case(&current));
        entries.insert(0, current);
        entries.truncate(MAX_RECENT_PROJECTS);
        atomic_write(&path, &serde_json::to_vec_pretty(&entries)?)
    }

    fn update_metadata<F>(&self, project_folder: &str, update: F) -> Result<(), WorkspaceError>
    where
        F: FnOnce(&mut ProjectMetadata),
    {
        let folder = checked_directory(project_folder)?;
        let (mut metadata, _, path) = load_project_paths(&folder)?;
        update(&mut metadata);
        metadata.modified_utc = Utc::now();
        write_metadata(&path, &metadata)
    }
}

pub fn validate_project_name(value: &str) -> Result<(), WorkspaceError> {
    if value.is_empty() || value.len() > 63 {
        return Err(WorkspaceError::ProjectInvalid(
            "project name must be 1–63 characters".into(),
        ));
    }
    let mut chars = value.chars();
    if !chars
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
        || !chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(WorkspaceError::ProjectInvalid(
            "Arduino project names must start with a letter and use only letters, digits, or underscores"
                .into(),
        ));
    }
    Ok(())
}

pub fn sanitize_project_name(value: &str) -> String {
    let mut result: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if !result
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
    {
        result.insert(0, 'P');
    }
    result.truncate(63);
    result
}

fn checked_directory(value: &str) -> Result<PathBuf, WorkspaceError> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(WorkspaceError::ProjectInvalid(
            "project folder must be a non-empty path without NUL characters".into(),
        ));
    }
    let path = PathBuf::from(value);
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(WorkspaceError::ProjectInvalid(
            "parent-directory traversal is not allowed in project paths".into(),
        ));
    }
    Ok(path)
}

fn ensure_child_path(parent: &Path, candidate: &Path) -> Result<(), WorkspaceError> {
    if candidate.parent() != Some(parent) {
        return Err(WorkspaceError::ProjectInvalid(
            "project path escaped its selected parent folder".into(),
        ));
    }
    Ok(())
}

fn load_project_paths(
    folder: &Path,
) -> Result<(ProjectMetadata, PathBuf, PathBuf), WorkspaceError> {
    if !folder.is_dir() {
        return Err(WorkspaceError::ProjectInvalid(format!(
            "project folder does not exist: {}",
            folder.display()
        )));
    }
    let metadata_path = folder.join(PROJECT_FILE);
    let metadata: ProjectMetadata = serde_json::from_slice(&fs::read(&metadata_path)?)?;
    if metadata.schema_version != PROJECT_SCHEMA_VERSION || metadata.fqbn != UNO_R4_WIFI_FQBN {
        return Err(WorkspaceError::ProjectInvalid(
            "unsupported project metadata schema or target board".into(),
        ));
    }
    validate_project_name(&metadata.project_name)?;
    let expected_filename = format!("{}.ino", metadata.project_name);
    if metadata.source_filename != expected_filename
        || Path::new(&metadata.source_filename)
            .file_name()
            .and_then(|name| name.to_str())
            != Some(metadata.source_filename.as_str())
    {
        return Err(WorkspaceError::ProjectInvalid(
            "project source filename is not a safe Arduino sketch name".into(),
        ));
    }
    let source_path = folder.join(&metadata.source_filename);
    let ino_count = folder
        .read_dir()?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("ino"))
        })
        .count();
    if ino_count != 1 || !source_path.is_file() {
        return Err(WorkspaceError::ProjectInvalid(
            "a Phase 2 project must contain exactly one matching .ino file".into(),
        ));
    }
    Ok((metadata, source_path, metadata_path))
}

fn ensure_utf8_source(source: &str) -> Result<(), WorkspaceError> {
    if source.contains('\0') {
        return Err(WorkspaceError::ProjectInvalid(
            "Arduino source cannot contain a NUL character".into(),
        ));
    }
    if source.len() > 2 * 1024 * 1024 {
        return Err(WorkspaceError::ProjectInvalid(
            "source exceeds the 2 MiB classroom-project limit".into(),
        ));
    }
    Ok(())
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim().to_owned();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

fn write_metadata(path: &Path, metadata: &ProjectMetadata) -> Result<(), WorkspaceError> {
    atomic_write(path, &serde_json::to_vec_pretty(metadata)?)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), WorkspaceError> {
    let parent = path.parent().ok_or_else(|| {
        WorkspaceError::ProjectInvalid("target path has no parent directory".into())
    })?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project"),
        std::process::id()
    ));
    {
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temp, path)?;
    Ok(())
}

pub fn stable_hash(bytes: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn default_data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("firmware_workspace_data"))
        .join("WVU Bioinstrumentation Studio")
        .join("firmware_workspace")
}

fn template_description(kind: TemplateKind) -> &'static str {
    match kind {
        TemplateKind::BlankUnoR4Wifi => "Minimal editable UNO R4 WiFi sketch; it is not acquisition compatible.",
        TemplateKind::A0AcquisitionExample => "ASCII A0 serial example for bench diagnostics; not WVU protocol compatible.",
        TemplateKind::WvuProtocolReference => "Controlled one-channel WVU binary protocol reference firmware.",
        TemplateKind::SafeDigitalOutput => "D4, D5, and D6 are configured LOW and never automatically driven HIGH.",
        TemplateKind::SerialDiagnostic => "ASCII serial diagnostic sketch. Uploading it disables WVU binary acquisition until the reference is restored.",
    }
}

const BLANK_TEMPLATE: &str = r#"/*
 * Blank Arduino UNO R4 WiFi sketch for WVU Bioinstrumentation Studio.
 * Teaching and engineering use only; this is not a medical device.
 */
#include <Arduino.h>

void setup() {
  Serial.begin(115200);
}

void loop() {
}
"#;

const A0_TEMPLATE: &str = r#"/*
 * A0 ASCII acquisition example for bench diagnostics only.
 * This is not WVU binary protocol firmware; Studio acquisition will be unavailable while installed.
 */
#include <Arduino.h>

void setup() {
  analogReadResolution(12);
  Serial.begin(115200);
}

void loop() {
  Serial.println(analogRead(A0));
  delay(10);
}
"#;

const SAFE_DIGITAL_TEMPLATE: &str = r#"/*
 * Safe digital-output example for the Arduino UNO R4 WiFi.
 * D4, D5, and D6 are held LOW; this example never drives them HIGH.
 * This is not WVU binary protocol firmware.
 */
#include <Arduino.h>

const uint8_t kSafetyPins[] = {4, 5, 6};

void setup() {
  for (uint8_t pin : kSafetyPins) {
    pinMode(pin, OUTPUT);
    digitalWrite(pin, LOW);
  }
  Serial.begin(115200);
  Serial.println("Safe digital outputs are LOW.");
}

void loop() {
  for (uint8_t pin : kSafetyPins) digitalWrite(pin, LOW);
  delay(100);
}
"#;

const SERIAL_DIAGNOSTIC_TEMPLATE: &str = r#"/*
 * ASCII serial diagnostic firmware for UNO R4 WiFi bench use.
 * WVU binary acquisition handshake will NOT work while this sketch is installed.
 * No biomedical accessory or person is required for this diagnostic.
 */
#include <Arduino.h>

void setup() {
  analogReadResolution(12);
  Serial.begin(115200);
  Serial.println("WVU ASCII diagnostic: A0 counts follow.");
}

void loop() {
  Serial.print("A0=");
  Serial.println(analogRead(A0));
  delay(250);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn request(parent: &Path, name: &str, template: TemplateKind) -> CreateProjectRequest {
        CreateProjectRequest {
            parent_folder: parent.display().to_string(),
            project_name: name.into(),
            template,
            notes: Some("test".into()),
            overwrite_confirmed: false,
        }
    }

    #[test]
    fn create_open_save_and_save_as_enforce_one_ino_project() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let workspace = FirmwareWorkspace::new(dir.path().join("appdata"));
        let project = workspace
            .create_project(request(
                dir.path(),
                "StudentSketch",
                TemplateKind::BlankUnoR4Wifi,
            ))
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(Path::new(&project.source_path).is_file());
        let saved = workspace
            .save_project(SaveProjectRequest {
                project_folder: project.project_folder.clone(),
                source: "// UTF-8 comment: café\nvoid setup() {}\nvoid loop() {}\n".into(),
                notes: Some("updated".into()),
                selected_com_port: Some("COM12".into()),
            })
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(saved.source.contains("café"));
        let copied = workspace
            .save_as_project(SaveAsProjectRequest {
                source_project_folder: saved.project_folder,
                destination_parent_folder: dir.path().display().to_string(),
                destination_project_name: "StudentCopy".into(),
                source: saved.source,
                overwrite_confirmed: false,
            })
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(copied.metadata.project_name, "StudentCopy");
        fs::write(Path::new(&copied.project_folder).join("extra.ino"), "")
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(workspace.open_project(&copied.project_folder).is_err());
    }

    #[test]
    fn naming_traversal_and_overwrite_are_rejected() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let workspace = FirmwareWorkspace::new(dir.path().join("appdata"));
        assert!(validate_project_name("9bad").is_err());
        assert!(validate_project_name("bad name").is_err());
        assert_eq!(sanitize_project_name("9 bad-name"), "P9_bad_name");
        assert!(workspace
            .create_project(request(
                dir.path(),
                "../escape",
                TemplateKind::BlankUnoR4Wifi
            ))
            .is_err());
        workspace
            .create_project(request(
                dir.path(),
                "SafeName",
                TemplateKind::BlankUnoR4Wifi,
            ))
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(workspace
            .create_project(request(
                dir.path(),
                "SafeName",
                TemplateKind::BlankUnoR4Wifi
            ))
            .is_err());
    }

    #[test]
    fn controlled_reference_copy_is_byte_identical_and_metadata_round_trips() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let workspace = FirmwareWorkspace::new(dir.path().join("appdata"));
        let project = workspace
            .create_project(request(
                dir.path(),
                "ReferenceCopy",
                TemplateKind::WvuProtocolReference,
            ))
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            fs::read(&project.source_path).unwrap_or_default(),
            REFERENCE_TEMPLATE
        );
        let opened = workspace
            .open_project(&project.project_folder)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            opened.metadata.verification_kind,
            FirmwareVerificationKind::WvuProtocolReference
        );
        assert!(workspace
            .recent_projects()
            .unwrap_or_default()
            .contains(&project.project_folder));
    }
}
