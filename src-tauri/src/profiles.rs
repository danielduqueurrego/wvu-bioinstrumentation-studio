//! Versioned, validated acquisition profiles. SHA-256 here detects accidental
//! modification; it is explicitly not an authorship signature or access-control mechanism.
use crate::protocol::{
    PROTOCOL_MAJOR, PROTOCOL_MINOR, REFERENCE_DEVICE_ID, REFERENCE_FIRMWARE_BUILD,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub const PROFILE_SCHEMA_VERSION: u32 = 1;
pub const UNO_R4_WIFI_FQBN: &str = "arduino:renesas_uno:unor4wifi";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileStatus {
    Locked,
    Draft,
    Retired,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSource {
    BuiltIn,
    Instructor,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileMode {
    Student,
    InstructorAuthoring,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FirmwareRequirement {
    pub protocol_major: u8,
    pub protocol_minor_min: u8,
    /// Hexadecimal, e.g. `0x00010001`, to preserve the controlled firmware's documented identity.
    pub build: String,
    pub device: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AcquisitionSettings {
    pub analog_pin: String,
    pub adc_resolution_bits: u8,
    pub sample_rate_hz: u32,
    pub allowed_duration_modes: Vec<String>,
    pub timed_presets_seconds: Vec<u64>,
    pub minimum_custom_duration_seconds: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplaySettings {
    pub primary_quantity: String,
    pub channel_label: String,
    pub raw_units_label: String,
    pub voltage_units_label: String,
    pub voltage_reference_v: f64,
    pub plot_min_v: f64,
    pub plot_max_v: f64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SafetySettings {
    pub bench_only: bool,
    pub human_connection_authorized: bool,
    pub not_medical_device: bool,
    pub notices: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExportSettings {
    pub signal_name: String,
    pub include_profile_snapshot: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileIntegrity {
    pub canonical_hash_algorithm: String,
    pub canonical_hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AcquisitionProfile {
    pub schema_version: u32,
    pub profile_id: String,
    pub profile_version: String,
    pub display_name: String,
    pub category: String,
    pub status: ProfileStatus,
    pub source: ProfileSource,
    pub description: String,
    pub target_board: String,
    pub fqbn: String,
    pub required_firmware: FirmwareRequirement,
    pub acquisition: AcquisitionSettings,
    pub display: DisplaySettings,
    pub safety: SafetySettings,
    pub export: ExportSettings,
    pub integrity: ProfileIntegrity,
    /// Retain optional future fields in deterministic key order on import/export.
    #[serde(flatten, default)]
    pub additional: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileSnapshot {
    pub captured_utc: DateTime<Utc>,
    pub bench_notice_acknowledged: bool,
    pub profile: AcquisitionProfile,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("cannot read profile {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("cannot write profile {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid profile JSON: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("profile validation failed: {0}")]
    Validation(String),
    #[error("student mode may not perform this profile authoring action")]
    StudentMode,
}

impl AcquisitionProfile {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ProfileError> {
        let mut value = self.clone();
        value.integrity.canonical_hash.clear();
        serde_json::to_vec(&value).map_err(ProfileError::Parse)
    }
    pub fn computed_hash(&self) -> Result<String, ProfileError> {
        Ok(format!("{:x}", Sha256::digest(self.canonical_bytes()?)))
    }
    pub fn refresh_hash(&mut self) -> Result<(), ProfileError> {
        self.integrity.canonical_hash_algorithm = "SHA-256".into();
        self.integrity.canonical_hash = self.computed_hash()?;
        Ok(())
    }
    pub fn verify_integrity(&self) -> Result<(), ProfileError> {
        if !self
            .integrity
            .canonical_hash_algorithm
            .eq_ignore_ascii_case("SHA-256")
        {
            return Err(ProfileError::Validation(
                "locked profiles require SHA-256 integrity".into(),
            ));
        }
        let computed = self.computed_hash()?;
        if self.integrity.canonical_hash.len() != 64 || self.integrity.canonical_hash != computed {
            return Err(ProfileError::Validation(format!(
                "profile {} has a hash mismatch (computed {computed})",
                self.profile_id
            )));
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), ProfileError> {
        if self.schema_version != PROFILE_SCHEMA_VERSION {
            return Err(ProfileError::Validation(format!(
                "unsupported schema version {}",
                self.schema_version
            )));
        }
        if !valid_profile_id(&self.profile_id) {
            return Err(ProfileError::Validation(
                "profile ID must use lowercase letters, numbers, dots, dashes, or underscores"
                    .into(),
            ));
        }
        if !valid_semver(&self.profile_version) {
            return Err(ProfileError::Validation(
                "profile version must be MAJOR.MINOR.PATCH".into(),
            ));
        }
        if self.display_name.trim().is_empty() || self.category.trim().is_empty() {
            return Err(ProfileError::Validation(
                "display name and category are required".into(),
            ));
        }
        if self.target_board != "Arduino UNO R4 WiFi" || self.fqbn != UNO_R4_WIFI_FQBN {
            return Err(ProfileError::Validation(
                "profile targets an unsupported board or FQBN".into(),
            ));
        }
        if !matches!(
            self.acquisition.analog_pin.as_str(),
            "A0" | "A1" | "A2" | "A3" | "A4" | "A5"
        ) {
            return Err(ProfileError::Validation(
                "analog pin must be A0 through A5".into(),
            ));
        }
        if self.acquisition.adc_resolution_bits != 12 || self.acquisition.sample_rate_hz != 1_000 {
            return Err(ProfileError::Validation(
                "the controlled Phase 3A firmware supports one 12-bit channel at 1000 samples/s"
                    .into(),
            ));
        }
        if self.acquisition.allowed_duration_modes.is_empty()
            || !self
                .acquisition
                .allowed_duration_modes
                .iter()
                .all(|mode| matches!(mode.as_str(), "timed" | "until_stopped"))
        {
            return Err(ProfileError::Validation(
                "duration modes must be timed and/or until_stopped".into(),
            ));
        }
        if self.acquisition.minimum_custom_duration_seconds < 10
            || self
                .acquisition
                .timed_presets_seconds
                .iter()
                .any(|seconds| *seconds < 10)
        {
            return Err(ProfileError::Validation(
                "timed duration values must be at least 10 seconds".into(),
            ));
        }
        if self.display.primary_quantity != "arduino_input_volts"
            || self.display.raw_units_label != "ADC counts"
            || self.display.voltage_units_label != "V"
            || self.display.voltage_reference_v <= 0.0
            || self.display.plot_min_v >= self.display.plot_max_v
        {
            return Err(ProfileError::Validation(
                "only raw ADC counts and Arduino input volts with ordered plot limits are valid"
                    .into(),
            ));
        }
        if !self.safety.bench_only
            || self.safety.human_connection_authorized
            || !self.safety.not_medical_device
            || self.safety.notices.is_empty()
        {
            return Err(ProfileError::Validation("Phase 3A profiles must be explicitly bench-only, non-medical, and forbid human connection".into()));
        }
        if self.export.signal_name.trim().is_empty() || !self.export.include_profile_snapshot {
            return Err(ProfileError::Validation(
                "profile export must name the signal and include a snapshot".into(),
            ));
        }
        if self.required_firmware.protocol_major != PROTOCOL_MAJOR
            || self.required_firmware.protocol_minor_min > PROTOCOL_MINOR
            || parse_hex(&self.required_firmware.build)? != REFERENCE_FIRMWARE_BUILD
            || parse_hex(&self.required_firmware.device)? != REFERENCE_DEVICE_ID
        {
            return Err(ProfileError::Validation(
                "profile requires an incompatible controlled firmware identity".into(),
            ));
        }
        if self.status == ProfileStatus::Locked {
            self.verify_integrity()?;
        }
        Ok(())
    }
    pub fn snapshot(&self, acknowledged: bool) -> ProfileSnapshot {
        ProfileSnapshot {
            captured_utc: Utc::now(),
            bench_notice_acknowledged: acknowledged,
            profile: self.clone(),
        }
    }
    pub fn requires_bench_acknowledgement(&self) -> bool {
        matches!(self.category.as_str(), "ecg" | "emg")
    }
}

pub fn load_profile(path: &Path) -> Result<AcquisitionProfile, ProfileError> {
    let text = fs::read_to_string(path).map_err(|source| ProfileError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let profile: AcquisitionProfile = serde_json::from_str(&text)?;
    profile.validate()?;
    Ok(profile)
}

pub fn built_in_profiles() -> Result<Vec<AcquisitionProfile>, ProfileError> {
    [
        include_str!("../../profiles/general_a0_development.profile.json"),
        include_str!("../../profiles/ecg_module_raw.profile.json"),
        include_str!("../../profiles/emg_module_raw.profile.json"),
    ]
    .into_iter()
    .map(|text| {
        let profile: AcquisitionProfile = serde_json::from_str(text)?;
        profile.validate()?;
        Ok(profile)
    })
    .collect()
}

#[derive(Clone)]
pub struct ProfileStore {
    root: PathBuf,
    runtime: Arc<Mutex<ProfileRuntime>>,
}
struct ProfileRuntime {
    mode: ProfileMode,
    profiles: BTreeMap<(String, String), AcquisitionProfile>,
    retired: BTreeSet<(String, String)>,
}

impl Default for ProfileStore {
    fn default() -> Self {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("WVU Bioinstrumentation Studio")
            .join("profiles");
        Self::with_root(root).unwrap_or_else(|_| Self {
            root: std::env::temp_dir(),
            runtime: Arc::new(Mutex::new(ProfileRuntime {
                mode: ProfileMode::Student,
                profiles: BTreeMap::new(),
                retired: BTreeSet::new(),
            })),
        })
    }
}
impl ProfileStore {
    pub fn with_root(root: PathBuf) -> Result<Self, ProfileError> {
        let mut profiles = BTreeMap::new();
        for profile in built_in_profiles()? {
            profiles.insert(
                (profile.profile_id.clone(), profile.profile_version.clone()),
                profile,
            );
        }
        Ok(Self {
            root,
            runtime: Arc::new(Mutex::new(ProfileRuntime {
                mode: ProfileMode::Student,
                profiles,
                retired: BTreeSet::new(),
            })),
        })
    }
    pub fn mode(&self) -> Result<ProfileMode, ProfileError> {
        Ok(self.lock()?.mode.clone())
    }
    pub fn set_mode(
        &self,
        mode: ProfileMode,
        acknowledgement: bool,
    ) -> Result<ProfileMode, ProfileError> {
        if mode == ProfileMode::InstructorAuthoring && !acknowledgement {
            return Err(ProfileError::Validation(
                "confirm that instructor mode can change acquisition settings".into(),
            ));
        }
        let mut runtime = self.lock()?;
        runtime.mode = mode.clone();
        drop(runtime);
        self.append_mode_log(&mode)?;
        Ok(mode)
    }
    pub fn list(&self) -> Result<Vec<AcquisitionProfile>, ProfileError> {
        let runtime = self.lock()?;
        Ok(runtime
            .profiles
            .iter()
            .filter(|(key, profile)| {
                profile.status == ProfileStatus::Locked && !runtime.retired.contains(*key)
            })
            .map(|(_, profile)| profile.clone())
            .collect())
    }
    pub fn get_locked(&self, profile_id: &str) -> Result<AcquisitionProfile, ProfileError> {
        let runtime = self.lock()?;
        runtime
            .profiles
            .iter()
            .find(|((id, _), p)| {
                id == profile_id
                    && p.status == ProfileStatus::Locked
                    && !runtime.retired.contains(&pkey(p))
            })
            .map(|(_, p)| p.clone())
            .ok_or_else(|| ProfileError::Validation("select a valid active locked profile".into()))
    }
    pub fn duplicate_to_draft(
        &self,
        profile_id: &str,
        draft_id: &str,
    ) -> Result<AcquisitionProfile, ProfileError> {
        self.require_instructor()?;
        if !valid_profile_id(draft_id) {
            return Err(ProfileError::Validation("invalid draft profile ID".into()));
        }
        let locked = self.get_locked(profile_id)?;
        let mut draft = locked;
        draft.profile_id = draft_id.into();
        draft.status = ProfileStatus::Draft;
        draft.source = ProfileSource::Instructor;
        draft.integrity.canonical_hash.clear();
        let mut runtime = self.lock()?;
        if runtime.profiles.keys().any(|(id, _)| id == draft_id) {
            return Err(ProfileError::Validation("profile ID already exists".into()));
        }
        runtime.profiles.insert(pkey(&draft), draft.clone());
        Ok(draft)
    }
    pub fn update_draft_description(
        &self,
        profile_id: &str,
        profile_version: &str,
        description: String,
    ) -> Result<AcquisitionProfile, ProfileError> {
        self.require_instructor()?;
        let mut runtime = self.lock()?;
        let draft = runtime
            .profiles
            .get_mut(&(profile_id.into(), profile_version.into()))
            .ok_or_else(|| ProfileError::Validation("draft not found".into()))?;
        if draft.status != ProfileStatus::Draft {
            return Err(ProfileError::Validation(
                "only a draft may be edited".into(),
            ));
        }
        if description.trim().is_empty() {
            return Err(ProfileError::Validation("description is required".into()));
        }
        draft.description = description;
        Ok(draft.clone())
    }
    pub fn finalize_draft(
        &self,
        profile_id: &str,
        profile_version: &str,
        final_version: String,
    ) -> Result<AcquisitionProfile, ProfileError> {
        self.require_instructor()?;
        if !valid_semver(&final_version) {
            return Err(ProfileError::Validation(
                "final version must be MAJOR.MINOR.PATCH".into(),
            ));
        }
        let mut runtime = self.lock()?;
        let mut final_profile = runtime
            .profiles
            .get(&(profile_id.into(), profile_version.into()))
            .cloned()
            .ok_or_else(|| ProfileError::Validation("draft not found".into()))?;
        if final_profile.status != ProfileStatus::Draft {
            return Err(ProfileError::Validation(
                "only a draft may be finalized".into(),
            ));
        }
        final_profile.profile_version = final_version;
        final_profile.status = ProfileStatus::Locked;
        final_profile.source = ProfileSource::Instructor;
        final_profile.refresh_hash()?;
        final_profile.validate()?;
        let key = pkey(&final_profile);
        if runtime.profiles.contains_key(&key) {
            return Err(ProfileError::Validation(
                "profile ID/version already exists".into(),
            ));
        }
        runtime.profiles.insert(key, final_profile.clone());
        drop(runtime);
        self.persist(&final_profile)?;
        Ok(final_profile)
    }
    pub fn retire(&self, profile_id: &str, profile_version: &str) -> Result<(), ProfileError> {
        self.require_instructor()?;
        let mut runtime = self.lock()?;
        let key = (profile_id.into(), profile_version.into());
        let profile = runtime
            .profiles
            .get(&key)
            .ok_or_else(|| ProfileError::Validation("profile not found".into()))?;
        if profile.source == ProfileSource::BuiltIn {
            return Err(ProfileError::Validation(
                "built-in profiles cannot be retired".into(),
            ));
        }
        runtime.retired.insert(key);
        Ok(())
    }
    pub fn import_profile(&self, path: &Path) -> Result<AcquisitionProfile, ProfileError> {
        self.require_instructor()?;
        let profile = load_profile(path)?;
        if profile.status != ProfileStatus::Locked {
            return Err(ProfileError::Validation(
                "only finalized locked profile packages may be imported".into(),
            ));
        }
        let mut runtime = self.lock()?;
        let key = pkey(&profile);
        if runtime.profiles.contains_key(&key) {
            return Err(ProfileError::Validation(
                "profile ID/version already exists".into(),
            ));
        }
        runtime.profiles.insert(key, profile.clone());
        drop(runtime);
        self.persist(&profile)?;
        Ok(profile)
    }
    pub fn export_profile(
        &self,
        profile_id: &str,
        profile_version: &str,
        destination: &Path,
    ) -> Result<(), ProfileError> {
        let profile = self
            .lock()?
            .profiles
            .get(&(profile_id.into(), profile_version.into()))
            .cloned()
            .ok_or_else(|| ProfileError::Validation("profile not found".into()))?;
        if destination.file_name().is_none()
            || destination
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(ProfileError::Validation(
                "invalid profile export destination".into(),
            ));
        }
        fs::write(destination, serde_json::to_vec_pretty(&profile)?).map_err(|source| {
            ProfileError::Write {
                path: destination.display().to_string(),
                source,
            }
        })
    }
    fn require_instructor(&self) -> Result<(), ProfileError> {
        if self.mode()? == ProfileMode::InstructorAuthoring {
            Ok(())
        } else {
            Err(ProfileError::StudentMode)
        }
    }
    fn persist(&self, profile: &AcquisitionProfile) -> Result<(), ProfileError> {
        let dir = self.root.join("locked");
        fs::create_dir_all(&dir).map_err(|source| ProfileError::Write {
            path: dir.display().to_string(),
            source,
        })?;
        let path = dir.join(format!(
            "{}_{}.profile.json",
            safe_filename_component(&profile.profile_id),
            profile.profile_version
        ));
        fs::write(&path, serde_json::to_vec_pretty(profile)?).map_err(|source| {
            ProfileError::Write {
                path: path.display().to_string(),
                source,
            }
        })
    }
    fn append_mode_log(&self, mode: &ProfileMode) -> Result<(), ProfileError> {
        fs::create_dir_all(&self.root).map_err(|source| ProfileError::Write {
            path: self.root.display().to_string(),
            source,
        })?;
        let path = self.root.join("mode_changes.log");
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| ProfileError::Write {
                path: path.display().to_string(),
                source,
            })?;
        writeln!(file, "{}\t{:?}", Utc::now().to_rfc3339(), mode).map_err(|source| {
            ProfileError::Write {
                path: path.display().to_string(),
                source,
            }
        })
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ProfileRuntime>, ProfileError> {
        self.runtime
            .lock()
            .map_err(|_| ProfileError::Validation("profile store lock poisoned".into()))
    }
}

fn pkey(profile: &AcquisitionProfile) -> (String, String) {
    (profile.profile_id.clone(), profile.profile_version.clone())
}
fn parse_hex(value: &str) -> Result<u32, ProfileError> {
    u32::from_str_radix(value.trim_start_matches("0x"), 16)
        .map_err(|_| ProfileError::Validation("firmware identity must be hexadecimal".into()))
}
fn valid_profile_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_'))
}
fn valid_semver(value: &str) -> bool {
    let fields: Vec<_> = value.split('.').collect();
    fields.len() == 3
        && fields
            .iter()
            .all(|field| !field.is_empty() && field.chars().all(|c| c.is_ascii_digit()))
}
pub fn safe_filename_component(value: &str) -> String {
    let clean: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let clean = clean.trim_matches('_').to_string();
    if clean.is_empty() {
        "unnamed".into()
    } else {
        clean.chars().take(64).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    #[test]
    fn builtin_profiles_are_valid_and_hashed() {
        let profiles = built_in_profiles().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(profiles.len(), 3);
        assert!(profiles
            .iter()
            .all(|p| p.status == ProfileStatus::Locked && p.verify_integrity().is_ok()));
    }
    #[test]
    fn hash_detects_profile_change() {
        let mut profile = built_in_profiles().unwrap_or_else(|e| panic!("{e}"))[1].clone();
        profile.description.push('!');
        assert!(profile.verify_integrity().is_err());
    }
    #[test]
    fn instructor_draft_finalization_and_retirement_are_controlled() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let store = ProfileStore::with_root(dir.path().into()).unwrap_or_else(|e| panic!("{e}"));
        assert!(store
            .duplicate_to_draft("wvu.bmeg420l.ecg.raw.v1", "example.ecg.draft")
            .is_err());
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|e| panic!("{e}"));
        let draft = store
            .duplicate_to_draft("wvu.bmeg420l.ecg.raw.v1", "example.ecg.draft")
            .unwrap_or_else(|e| panic!("{e}"));
        let final_profile = store
            .finalize_draft(&draft.profile_id, &draft.profile_version, "1.0.1".into())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(final_profile.verify_integrity().is_ok());
        store
            .retire(&final_profile.profile_id, &final_profile.profile_version)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!store
            .list()
            .unwrap_or_default()
            .iter()
            .any(|p| p.profile_id == final_profile.profile_id));
    }
    #[test]
    fn failed_instructor_entry_neither_changes_mode_nor_logs_a_success() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let root = dir.path().join("profiles");
        let store = ProfileStore::with_root(root.clone()).unwrap_or_else(|e| panic!("{e}"));
        assert!(store
            .set_mode(ProfileMode::InstructorAuthoring, false)
            .is_err());
        assert_eq!(
            store.mode().unwrap_or_else(|e| panic!("{e}")),
            ProfileMode::Student
        );
        assert!(!root.join("mode_changes.log").exists());
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|e| panic!("{e}"));
        let log =
            fs::read_to_string(root.join("mode_changes.log")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(log.lines().count(), 1);
        assert!(log.contains("InstructorAuthoring"));
    }
}
