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
pub const SUPPORTED_ANALOG_PINS: [&str; 6] = ["A0", "A1", "A2", "A3", "A4", "A5"];
pub const SUPPORTED_DIGITAL_OUTPUT_PINS: [&str; 3] = ["D4", "D5", "D6"];
pub const RECOMMENDED_SAMPLE_RATES_HZ: [u32; 5] = [100, 200, 250, 500, 1_000];
const MAX_CATALOG_LOG_BYTES: u64 = 512 * 1024;
const RETAINED_CATALOG_LOG_FILES: usize = 3;

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
    Imported,
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
#[serde(rename_all = "snake_case")]
pub enum AcquisitionMode {
    Simultaneous,
    #[serde(rename = "pulseox_4state")]
    Pulseox4State,
}

fn default_acquisition_mode() -> AcquisitionMode {
    AcquisitionMode::Simultaneous
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileChannel {
    pub pin: String,
    pub id: String,
    pub label: String,
    pub csv_name: String,
    pub units: String,
    /// Display/conversion capabilities are a lab-level allowance. Student calibration
    /// coefficients remain local and are never written into a locked lab definition.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_conversions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_display_unit: Option<String>,
    #[serde(
        default = "default_visible",
        skip_serializing_if = "is_default_visible"
    )]
    pub default_visible: bool,
}

fn default_visible() -> bool {
    true
}
fn is_default_visible(value: &bool) -> bool {
    *value
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DigitalOutputBehavior {
    AlwaysLow,
    HighWhileRecording,
    AcquisitionSequenced,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DigitalOutput {
    pub pin: String,
    pub label: String,
    pub behavior: DigitalOutputBehavior,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlotDefaultGroup {
    pub channel_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlotDefaults {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<PlotDefaultGroup>,
}

/// Historical optional sketch metadata retained only for older lab packages.
/// The distributed application does not expose a sketch editor or association UI.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssociatedSketch {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    #[serde(default)]
    pub is_wvu_reference: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PulseOxInputs {
    pub tx: String,
    pub rx: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LedOutputs {
    #[serde(default)]
    pub green: Option<String>,
    #[serde(default)]
    pub red: Option<String>,
    #[serde(default)]
    pub ir: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AcquisitionSettings {
    /// Retained for legacy profile snapshots. Current labs use `channels`.
    #[serde(default = "default_analog_pin")]
    pub analog_pin: String,
    pub adc_resolution_bits: u8,
    pub sample_rate_hz: u32,
    pub allowed_duration_modes: Vec<String>,
    pub timed_presets_seconds: Vec<u64>,
    pub minimum_custom_duration_seconds: u64,
    #[serde(default = "default_acquisition_mode")]
    pub acquisition_mode: AcquisitionMode,
    #[serde(default)]
    pub channels: Vec<ProfileChannel>,
    #[serde(default)]
    pub analog_inputs: Option<PulseOxInputs>,
    #[serde(default)]
    pub led_outputs: Option<LedOutputs>,
    #[serde(default)]
    pub state_dwell_us: Option<u32>,
    /// Current labs use this explicit description. `led_outputs` remains readable
    /// in historical recordings and is treated as a legacy shorthand.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub digital_outputs: Vec<DigitalOutput>,
}

fn default_analog_pin() -> String {
    "A0".into()
}

impl AcquisitionSettings {
    pub fn resolved_channels(&self) -> Vec<ProfileChannel> {
        if !self.channels.is_empty() {
            return self.channels.clone();
        }
        vec![ProfileChannel {
            pin: self.analog_pin.clone(),
            id: "raw".into(),
            label: "Raw analog input".into(),
            csv_name: "adc_counts".into(),
            units: "ADC counts".into(),
            allowed_conversions: Vec::new(),
            default_display_unit: None,
            default_visible: true,
        }]
    }

    pub fn record_field_names(&self) -> Vec<String> {
        match self.acquisition_mode {
            AcquisitionMode::Simultaneous => self
                .resolved_channels()
                .into_iter()
                .map(|channel| channel.csv_name)
                .collect(),
            AcquisitionMode::Pulseox4State => [
                "red_TX", "dark1_TX", "ir_TX", "dark2_TX", "red_RX", "dark1_RX", "ir_RX",
                "dark2_RX",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    pub fn analog_pins(&self) -> Vec<String> {
        match self.acquisition_mode {
            AcquisitionMode::Simultaneous => self
                .resolved_channels()
                .into_iter()
                .map(|channel| channel.pin)
                .collect(),
            AcquisitionMode::Pulseox4State => self
                .analog_inputs
                .as_ref()
                .map(|inputs| vec![inputs.tx.clone(), inputs.rx.clone()])
                .unwrap_or_default(),
        }
    }

    pub fn resolved_digital_outputs(&self) -> Vec<DigitalOutput> {
        if !self.digital_outputs.is_empty() {
            return self.digital_outputs.clone();
        }
        let Some(legacy) = &self.led_outputs else {
            return Vec::new();
        };
        let mut outputs = Vec::new();
        if let Some(pin) = &legacy.green {
            outputs.push(DigitalOutput {
                pin: pin.clone(),
                label: "Green LED".into(),
                behavior: DigitalOutputBehavior::HighWhileRecording,
            });
        }
        if let Some(pin) = &legacy.red {
            outputs.push(DigitalOutput {
                pin: pin.clone(),
                label: "Red LED".into(),
                behavior: DigitalOutputBehavior::AcquisitionSequenced,
            });
        }
        if let Some(pin) = &legacy.ir {
            outputs.push(DigitalOutput {
                pin: pin.clone(),
                label: "IR LED".into(),
                behavior: DigitalOutputBehavior::AcquisitionSequenced,
            });
        }
        outputs
    }
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
    #[serde(default, skip_serializing_if = "PlotDefaults::is_empty")]
    pub plot_defaults: PlotDefaults,
    /// Retained for backward-reading imported historical lab definitions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub associated_sketch: Option<AssociatedSketch>,
    /// Retain optional future fields in deterministic key order on import/export.
    #[serde(flatten, default)]
    pub additional: BTreeMap<String, serde_json::Value>,
}

impl PlotDefaults {
    fn is_empty(value: &Self) -> bool {
        value.groups.is_empty()
    }
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
    #[error("the local lab catalog is unavailable; factory course labs remain available, but instructor changes are disabled until the catalog is repaired or reset: {0}")]
    CatalogUnavailable(String),
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
        let analog_pins = self.acquisition.analog_pins();
        let supported_pin = |pin: &str| SUPPORTED_ANALOG_PINS.contains(&pin);
        if analog_pins.is_empty()
            || analog_pins.len() > 6
            || !analog_pins.iter().all(|pin| supported_pin(pin))
            || {
                let mut unique = analog_pins.clone();
                unique.sort();
                unique.dedup();
                unique.len() != analog_pins.len()
            }
        {
            return Err(ProfileError::Validation(
                "analog channels must be unique pins A0 through A5".into(),
            ));
        }
        if !matches!(self.acquisition.adc_resolution_bits, 12 | 14) {
            return Err(ProfileError::Validation(
                "the controlled firmware supports 12-bit or 14-bit ADC acquisition".into(),
            ));
        }
        if self.acquisition.sample_rate_hz == 0 || self.acquisition.sample_rate_hz > 1_000 {
            return Err(ProfileError::Validation(
                "sample rate must be a supported positive rate no greater than 1000 frames/s"
                    .into(),
            ));
        }
        match self.acquisition.acquisition_mode {
            AcquisitionMode::Simultaneous => {
                let channels = self.acquisition.resolved_channels();
                let mut ids = BTreeSet::new();
                let mut csv_names = BTreeSet::new();
                if channels.len() != analog_pins.len()
                    || channels.iter().any(|channel| {
                        channel.id.trim().is_empty()
                            || channel.label.trim().is_empty()
                            || channel.csv_name.trim().is_empty()
                            || channel.units != "ADC counts"
                    })
                    || !channels
                        .iter()
                        .all(|channel| ids.insert(channel.id.clone()))
                    || !channels
                        .iter()
                        .all(|channel| csv_names.insert(channel.csv_name.clone()))
                {
                    return Err(ProfileError::Validation(
                        "simultaneous profiles require uniquely named raw-count channels and CSV fields".into(),
                    ));
                }
                validate_plot_defaults(
                    &self.plot_defaults,
                    &channels
                        .iter()
                        .map(|channel| channel.id.clone())
                        .collect::<Vec<_>>(),
                )?;
            }
            AcquisitionMode::Pulseox4State => {
                let outputs = self.acquisition.resolved_digital_outputs();
                let red = outputs
                    .iter()
                    .find(|output| output.label.eq_ignore_ascii_case("red led"));
                let ir = outputs
                    .iter()
                    .find(|output| output.label.eq_ignore_ascii_case("ir led"));
                if !matches!(self.acquisition.adc_resolution_bits, 12 | 14)
                    || !(50..=1_000).contains(&self.acquisition.sample_rate_hz)
                    || !matches!(self.acquisition.state_dwell_us, Some(250..=5_000))
                    || analog_pins.len() != 2
                    || self.acquisition.analog_inputs.is_none()
                    || red.is_none_or(|output| {
                        output.behavior != DigitalOutputBehavior::AcquisitionSequenced
                    })
                    || ir.is_none_or(|output| {
                        output.behavior != DigitalOutputBehavior::AcquisitionSequenced
                    })
                    || red.zip(ir).is_some_and(|(red, ir)| red.pin == ir.pin)
                {
                    return Err(ProfileError::Validation(
                        "pulse-ox profiles require unique TX/RX analog pins, 12- or 14-bit ADC, a supported dwell, and distinct sequenced Red/IR outputs"
                            .into(),
                    ));
                }
                validate_plot_defaults(
                    &self.plot_defaults,
                    &[
                        "red_tx".into(),
                        "dark1_tx".into(),
                        "ir_tx".into(),
                        "dark2_tx".into(),
                        "red_rx".into(),
                        "dark1_rx".into(),
                        "ir_rx".into(),
                        "dark2_rx".into(),
                    ],
                )?;
            }
        }
        let outputs = self.acquisition.resolved_digital_outputs();
        let mut output_pins = BTreeSet::new();
        if outputs.iter().any(|output| {
            !SUPPORTED_DIGITAL_OUTPUT_PINS.contains(&output.pin.as_str())
                || output.label.trim().is_empty()
                || !output_pins.insert(output.pin.clone())
        }) {
            return Err(ProfileError::Validation(
                "digital outputs must use each supported D4, D5, or D6 pin at most once".into(),
            ));
        }
        if self.acquisition.resolved_channels().iter().any(|channel| {
            channel.allowed_conversions.iter().any(|conversion| {
                !matches!(
                    conversion.as_str(),
                    "counts_volts" | "mpxv_pressure" | "linear_calibration"
                )
            }) || channel
                .default_display_unit
                .as_ref()
                .is_some_and(|unit| !matches!(unit.as_str(), "counts" | "volts" | "kpa" | "mmhg"))
        }) {
            return Err(ProfileError::Validation(
                "channel conversion capabilities must be Counts/Volts, MPXV pressure, or generic linear calibration".into(),
            ));
        }
        if outputs.iter().any(|output| {
            output.behavior == DigitalOutputBehavior::HighWhileRecording
                && (output.pin == "D5" || output.pin == "D6")
        }) {
            return Err(ProfileError::Validation(
                "D5 and D6 are reserved for the fixed mutually-exclusive pulse-ox sequence; use Always LOW outside that mode".into(),
            ));
        }
        if self.acquisition.acquisition_mode == AcquisitionMode::Simultaneous
            && outputs
                .iter()
                .any(|output| output.behavior == DigitalOutputBehavior::AcquisitionSequenced)
        {
            return Err(ProfileError::Validation(
                "acquisition-sequenced outputs are available only to the fixed pulse-ox mode"
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
        if !self.safety.not_medical_device || self.safety.notices.is_empty() {
            return Err(ProfileError::Validation(
                "profiles must be non-medical and include visible safety notices".into(),
            ));
        }
        if self.export.signal_name.trim().is_empty() || !self.export.include_profile_snapshot {
            return Err(ProfileError::Validation(
                "profile export must name the signal and include a snapshot".into(),
            ));
        }
        if self.required_firmware.protocol_major != PROTOCOL_MAJOR
            || self.required_firmware.protocol_minor_min > PROTOCOL_MINOR
            || ![REFERENCE_FIRMWARE_BUILD, 0x0001_0002, 0x0001_0001]
                .contains(&parse_hex(&self.required_firmware.build)?)
            || parse_hex(&self.required_firmware.device)? != REFERENCE_DEVICE_ID
        {
            return Err(ProfileError::Validation(
                "profile requires an incompatible controlled firmware identity".into(),
            ));
        }
        if let Some(sketch) = &self.associated_sketch {
            if sketch.name.trim().is_empty()
                || sketch.relative_path.as_ref().is_some_and(|path| {
                    path.trim().is_empty()
                        || Path::new(path).is_absolute()
                        || Path::new(path)
                            .components()
                            .any(|component| matches!(component, std::path::Component::ParentDir))
                        || !path.to_ascii_lowercase().ends_with(".ino")
                })
            {
                return Err(ProfileError::Validation(
                    "associated sketch must have a name and an optional relative .ino path without traversal".into(),
                ));
            }
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
        self.safety.bench_only && matches!(self.category.as_str(), "ecg" | "emg")
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
        include_str!("../../profiles/general_analog_development.profile.json"),
        include_str!("../../profiles/ecg_course_capture.profile.json"),
        include_str!("../../profiles/emg_force_course_capture.profile.json"),
        include_str!("../../profiles/blood_pressure_ppg_course_capture.profile.json"),
        include_str!("../../profiles/pulseox_tx_rx_course_capture.profile.json"),
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
#[derive(Clone)]
struct ProfileRuntime {
    mode: ProfileMode,
    catalog_writable: bool,
    catalog_load_error: Option<String>,
    /// Factory profiles are compiled into the application and never written to
    /// the local catalog. `profiles` holds them alongside local profiles only
    /// to make historical lookup straightforward; `factory_keys` prevents a
    /// local file from overriding or retiring a shipped definition.
    factory_keys: BTreeSet<(String, String)>,
    profiles: BTreeMap<(String, String), AcquisitionProfile>,
    retired: BTreeSet<(String, String)>,
    active_versions: BTreeMap<String, String>,
    completed_save_requests: BTreeMap<String, (String, String)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LabListEntry {
    pub profile: AcquisitionProfile,
    pub active: bool,
    pub retired: bool,
}

#[derive(Serialize, Deserialize)]
struct PersistedLabState {
    #[serde(default)]
    profiles: Vec<AcquisitionProfile>,
    #[serde(default)]
    retired: Vec<(String, String)>,
    #[serde(default)]
    active_versions: BTreeMap<String, String>,
    /// Durable idempotency records prevent a frontend retry of the same
    /// explicit Save action from allocating another version.
    #[serde(default)]
    completed_save_requests: BTreeMap<String, (String, String)>,
}

impl Default for ProfileStore {
    fn default() -> Self {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("WVU Bioinstrumentation Studio")
            .join("profiles");
        Self::with_root(root.clone())
            .unwrap_or_else(|error| Self::factory_only(root, error.to_string()))
    }
}
impl ProfileStore {
    /// Last-resort read-only fallback for an unavailable local catalog. The
    /// class defaults remain available even if a user-data path is malformed
    /// or temporarily inaccessible. Crucially, this retains the intended
    /// catalog location and rejects instructor writes rather than silently
    /// redirecting them into a temporary directory.
    fn factory_only(root: PathBuf, load_error: String) -> Self {
        let mut profiles = BTreeMap::new();
        let mut factory_keys = BTreeSet::new();
        for profile in built_in_profiles().unwrap_or_default() {
            let key = pkey(&profile);
            factory_keys.insert(key.clone());
            profiles.insert(key, profile);
        }
        let active_versions = profiles
            .values()
            .map(|profile| (profile.profile_id.clone(), profile.profile_version.clone()))
            .collect();
        Self {
            root,
            runtime: Arc::new(Mutex::new(ProfileRuntime {
                mode: ProfileMode::Student,
                catalog_writable: false,
                catalog_load_error: Some(load_error),
                factory_keys,
                profiles,
                retired: BTreeSet::new(),
                active_versions,
                completed_save_requests: BTreeMap::new(),
            })),
        }
    }
    pub fn with_root(root: PathBuf) -> Result<Self, ProfileError> {
        let mut profiles = BTreeMap::new();
        let mut factory_keys = BTreeSet::new();
        for profile in built_in_profiles()? {
            let key = pkey(&profile);
            factory_keys.insert(key.clone());
            profiles.insert(key, profile);
        }
        let mut active_versions: BTreeMap<String, String> = profiles
            .values()
            .map(|profile| (profile.profile_id.clone(), profile.profile_version.clone()))
            .collect();
        let mut retired = BTreeSet::new();
        let mut completed_save_requests = BTreeMap::new();
        let state_path = root.join("lab_state.json");
        if state_path.exists() {
            let text = fs::read_to_string(&state_path).map_err(|source| ProfileError::Read {
                path: state_path.display().to_string(),
                source,
            })?;
            let persisted: PersistedLabState = serde_json::from_str(&text)?;
            for profile in persisted.profiles {
                profile.validate()?;
                let key = pkey(&profile);
                // Factory definitions are resolved from bundled resources. A
                // stale local copy may never replace the class default merely
                // because it has the same identity.
                if !factory_keys.contains(&key) {
                    profiles.insert(key, profile);
                }
            }
            retired.extend(
                persisted
                    .retired
                    .into_iter()
                    .filter(|key| !factory_keys.contains(key)),
            );
            for (id, version) in persisted.active_versions {
                let key = (id.clone(), version.clone());
                if profiles.contains_key(&key) && !factory_keys.contains(&key) {
                    active_versions.insert(id, version);
                }
            }
            for (request_id, key) in persisted.completed_save_requests {
                if profiles.contains_key(&key) && !factory_keys.contains(&key) {
                    // Retain only usable idempotency entries. They are reads
                    // on startup and do not trigger any catalog write.
                    // A request ID is opaque but bounded by the saving API.
                    if valid_operation_id(&request_id) {
                        // We intentionally keep the last successful result.
                        // Replaying it returns this exact revision.
                        //
                        // No version is allocated during this reconciliation.
                        //
                        // (The insertion is into runtime memory only.)
                        completed_save_requests.insert(request_id, key);
                    }
                }
            }
        }
        Ok(Self {
            root,
            runtime: Arc::new(Mutex::new(ProfileRuntime {
                mode: ProfileMode::Student,
                catalog_writable: true,
                catalog_load_error: None,
                factory_keys,
                profiles,
                retired,
                active_versions,
                completed_save_requests,
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
        let catalog_writable = runtime.catalog_writable;
        drop(runtime);
        if catalog_writable {
            self.append_mode_log_best_effort(&mode);
        }
        Ok(mode)
    }
    pub fn list(&self) -> Result<Vec<AcquisitionProfile>, ProfileError> {
        let runtime = self.lock()?;
        let mut active = BTreeMap::new();
        for (key, profile) in &runtime.profiles {
            if profile.status != ProfileStatus::Locked || runtime.retired.contains(key) {
                continue;
            }
            if runtime.active_versions.get(&profile.profile_id) == Some(&profile.profile_version) {
                active.insert(profile.profile_id.clone(), profile.clone());
            }
        }
        Ok(active.into_values().collect())
    }

    /// Instructor-only Lab Manager inventory. It includes historical and retired revisions,
    /// while Student mode receives only `list()` active locked labs.
    pub fn list_all(&self) -> Result<Vec<LabListEntry>, ProfileError> {
        self.require_instructor()?;
        let runtime = self.lock()?;
        Ok(runtime
            .profiles
            .iter()
            .map(|(key, profile)| LabListEntry {
                profile: profile.clone(),
                active: runtime.active_versions.get(&profile.profile_id)
                    == Some(&profile.profile_version)
                    && !runtime.retired.contains(key),
                retired: runtime.retired.contains(key),
            })
            .collect())
    }
    pub fn get_locked(&self, profile_id: &str) -> Result<AcquisitionProfile, ProfileError> {
        let runtime = self.lock()?;
        let version = runtime.active_versions.get(profile_id).ok_or_else(|| {
            ProfileError::Validation("select a valid active locked profile".into())
        })?;
        let key = (profile_id.to_string(), version.clone());
        runtime
            .profiles
            .get(&key)
            .filter(|profile| {
                profile.status == ProfileStatus::Locked && !runtime.retired.contains(&key)
            })
            .cloned()
            .ok_or_else(|| ProfileError::Validation("select a valid active locked profile".into()))
    }

    /// Begins an entirely detached editor draft. It does not insert a profile,
    /// allocate a revision, mark an active version, or write `lab_state.json`.
    /// Only `save_lab_draft` creates a local revision.
    pub fn begin_lab_edit(&self, profile_id: &str) -> Result<AcquisitionProfile, ProfileError> {
        self.require_instructor()?;
        let current = self.get_locked(profile_id)?;
        let mut draft = current;
        draft.status = ProfileStatus::Draft;
        draft.source = ProfileSource::Instructor;
        draft.integrity.canonical_hash.clear();
        Ok(draft)
    }

    /// Creates a new lab identity from the selected lab. This is the non-destructive
    /// Duplicate action; the caller supplies a machine-safe ID, not a version string.
    pub fn duplicate_lab(
        &self,
        profile_id: &str,
        lab_id: &str,
    ) -> Result<AcquisitionProfile, ProfileError> {
        self.require_instructor()?;
        if !valid_profile_id(lab_id) {
            return Err(ProfileError::Validation(
                "lab ID must use lowercase letters, numbers, dots, dashes, or underscores".into(),
            ));
        }
        let current = self.get_locked(profile_id)?;
        let mut draft = current;
        draft.profile_id = lab_id.into();
        draft.profile_version = "1.0.0".into();
        draft.status = ProfileStatus::Draft;
        draft.source = ProfileSource::Instructor;
        draft.integrity.canonical_hash.clear();
        let runtime = self.lock()?;
        if runtime.profiles.keys().any(|(id, _)| id == lab_id) {
            return Err(ProfileError::Validation("lab ID already exists".into()));
        }
        Ok(draft)
    }

    /// Creates the blank simultaneous-analog template. It intentionally
    /// starts as an instructor draft; only an explicitly saved/hashed revision
    /// becomes visible to Student mode.
    pub fn create_blank_simultaneous_lab(
        &self,
        lab_id: &str,
    ) -> Result<AcquisitionProfile, ProfileError> {
        self.require_instructor()?;
        if !valid_profile_id(lab_id) {
            return Err(ProfileError::Validation(
                "lab ID must use lowercase letters, numbers, dots, dashes, or underscores".into(),
            ));
        }
        let mut draft = built_in_profiles()?
            .into_iter()
            .find(|profile| profile.category == "development")
            .ok_or_else(|| {
                ProfileError::Validation("General Analog template is unavailable".into())
            })?;
        draft.profile_id = lab_id.into();
        draft.profile_version = "1.0.0".into();
        draft.display_name = "Blank Simultaneous Analog".into();
        draft.description = "Instructor-authored simultaneous analog course lab.".into();
        draft.source = ProfileSource::Instructor;
        draft.status = ProfileStatus::Draft;
        draft.integrity.canonical_hash.clear();
        let runtime = self.lock()?;
        if runtime.profiles.contains_key(&pkey(&draft)) {
            return Err(ProfileError::Validation(
                "lab ID/version already exists".into(),
            ));
        }
        Ok(draft)
    }

    /// Atomically creates exactly one active local revision for a deliberate
    /// instructor Save. A request ID makes retried UI calls idempotent, and a
    /// base-version check rejects a stale editor rather than guessing a new
    /// version.
    pub fn save_lab_draft(
        &self,
        edited: AcquisitionProfile,
        base_version: Option<String>,
        request_id: String,
    ) -> Result<AcquisitionProfile, ProfileError> {
        self.require_instructor()?;
        self.require_catalog_writable()?;
        if !valid_operation_id(&request_id) {
            return Err(ProfileError::Validation(
                "save request ID is invalid".into(),
            ));
        }
        let mut runtime = self.lock()?;
        if let Some(key) = runtime.completed_save_requests.get(&request_id) {
            return runtime.profiles.get(key).cloned().ok_or_else(|| {
                ProfileError::Validation("saved lab revision is unavailable".into())
            });
        }
        let mut next = runtime.clone();
        let requested_id = edited.profile_id.clone();
        let audit_base_version = base_version.clone();
        let allocated_version = match base_version {
            Some(base) => {
                if next.active_versions.get(&requested_id) != Some(&base) {
                    return Err(ProfileError::Validation(
                        "this lab changed since the editor opened; reload the latest version before saving"
                            .into(),
                    ));
                }
                next_patch_version(&base)?
            }
            None => {
                if next.profiles.keys().any(|(id, _)| id == &requested_id) {
                    return Err(ProfileError::Validation(
                        "lab ID already exists; use Edit to create a new revision".into(),
                    ));
                }
                "1.0.0".into()
            }
        };
        let key = (requested_id.clone(), allocated_version.clone());
        if next.profiles.contains_key(&key) {
            return Err(ProfileError::Validation(
                "the requested lab revision already exists; reload before saving".into(),
            ));
        }
        let mut finalized = edited;
        finalized.profile_id = requested_id;
        finalized.profile_version = allocated_version;
        // Validate editable fields before the immutable hash exists, then lock and
        // validate the final canonical document.
        finalized.status = ProfileStatus::Draft;
        finalized.source = ProfileSource::Instructor;
        finalized.integrity.canonical_hash.clear();
        finalized.validate()?;
        finalized.status = ProfileStatus::Locked;
        finalized.refresh_hash()?;
        finalized.validate()?;
        next.profiles.insert(pkey(&finalized), finalized.clone());
        next.active_versions.insert(
            finalized.profile_id.clone(),
            finalized.profile_version.clone(),
        );
        next.retired.remove(&pkey(&finalized));
        next.completed_save_requests.insert(
            request_id.clone(),
            (
                finalized.profile_id.clone(),
                finalized.profile_version.clone(),
            ),
        );
        let persisted = self.persisted_state(&next);
        self.persist_state(&persisted)?;
        *runtime = next;
        drop(runtime);
        self.append_lab_write_log_best_effort(
            "save_new_version",
            &finalized.profile_id,
            audit_base_version.as_deref(),
            Some(&finalized.profile_version),
            &request_id,
        );
        Ok(finalized)
    }
    pub fn retire(&self, profile_id: &str, profile_version: &str) -> Result<(), ProfileError> {
        self.require_instructor()?;
        self.require_catalog_writable()?;
        let mut runtime = self.lock()?;
        let mut next = runtime.clone();
        let key = (profile_id.into(), profile_version.into());
        if next.factory_keys.contains(&key) {
            return Err(ProfileError::Validation(
                "shipped course defaults cannot be retired; use Restore course default to make one active"
                    .into(),
            ));
        }
        next.profiles
            .get(&key)
            .ok_or_else(|| ProfileError::Validation("profile not found".into()))?;
        next.retired.insert(key);
        if next.active_versions.get(profile_id) == Some(&profile_version.to_string()) {
            next.active_versions.remove(profile_id);
        }
        let persisted = self.persisted_state(&next);
        self.persist_state(&persisted)?;
        *runtime = next;
        drop(runtime);
        self.append_lab_write_log_best_effort(
            "retire",
            profile_id,
            Some(profile_version),
            None,
            "explicit",
        );
        Ok(())
    }
    pub fn restore_retired(
        &self,
        profile_id: &str,
        profile_version: &str,
    ) -> Result<AcquisitionProfile, ProfileError> {
        self.require_instructor()?;
        self.require_catalog_writable()?;
        let key = (profile_id.to_string(), profile_version.to_string());
        let mut runtime = self.lock()?;
        let mut next = runtime.clone();
        if next.factory_keys.contains(&key) {
            return Err(ProfileError::Validation(
                "factory definitions are always available and do not need restoring".into(),
            ));
        }
        let profile = next
            .profiles
            .get(&key)
            .filter(|profile| profile.status == ProfileStatus::Locked)
            .cloned()
            .ok_or_else(|| ProfileError::Validation("retired lab revision not found".into()))?;
        if !next.retired.remove(&key) {
            return Err(ProfileError::Validation(
                "lab revision is not retired".into(),
            ));
        }
        next.active_versions
            .insert(profile.profile_id.clone(), profile.profile_version.clone());
        let persisted = self.persisted_state(&next);
        self.persist_state(&persisted)?;
        *runtime = next;
        drop(runtime);
        self.append_lab_write_log_best_effort(
            "restore_retired",
            profile_id,
            Some(profile_version),
            Some(profile_version),
            "explicit",
        );
        Ok(profile)
    }
    pub fn restore_course_default(
        &self,
        profile_id: &str,
    ) -> Result<AcquisitionProfile, ProfileError> {
        self.require_instructor()?;
        self.require_catalog_writable()?;
        let restored = built_in_profiles()?
            .into_iter()
            .find(|profile| profile.profile_id == profile_id)
            .ok_or_else(|| {
                ProfileError::Validation("no shipped course default exists for this lab".into())
            })?;
        let mut runtime = self.lock()?;
        if runtime.active_versions.get(profile_id) == Some(&restored.profile_version) {
            return Ok(restored);
        }
        let mut next = runtime.clone();
        next.active_versions.insert(
            restored.profile_id.clone(),
            restored.profile_version.clone(),
        );
        let persisted = self.persisted_state(&next);
        self.persist_state(&persisted)?;
        *runtime = next;
        drop(runtime);
        self.append_lab_write_log_best_effort(
            "restore_factory_default",
            profile_id,
            None,
            Some(&restored.profile_version),
            "explicit",
        );
        Ok(restored)
    }
    /// Explicitly activates one previously saved local revision. Reading or
    /// selecting a lab never calls this method.
    pub fn set_active_version(
        &self,
        profile_id: &str,
        profile_version: &str,
    ) -> Result<AcquisitionProfile, ProfileError> {
        self.require_instructor()?;
        self.require_catalog_writable()?;
        let key = (profile_id.to_string(), profile_version.to_string());
        let mut runtime = self.lock()?;
        if runtime.factory_keys.contains(&key) {
            drop(runtime);
            return self.restore_course_default(profile_id);
        }
        let profile = runtime
            .profiles
            .get(&key)
            .filter(|profile| profile.status == ProfileStatus::Locked)
            .cloned()
            .ok_or_else(|| ProfileError::Validation("local lab revision not found".into()))?;
        if runtime.retired.contains(&key) {
            return Err(ProfileError::Validation(
                "restore this retired lab revision before making it active".into(),
            ));
        }
        if runtime.active_versions.get(profile_id) == Some(&profile_version.to_string()) {
            return Ok(profile);
        }
        let mut next = runtime.clone();
        next.active_versions
            .insert(profile_id.to_string(), profile_version.to_string());
        let persisted = self.persisted_state(&next);
        self.persist_state(&persisted)?;
        *runtime = next;
        drop(runtime);
        self.append_lab_write_log_best_effort(
            "set_active_version",
            profile_id,
            None,
            Some(profile_version),
            "explicit",
        );
        Ok(profile)
    }
    /// Removes only local instructor/imported overrides. Factory definitions
    /// and recordings remain intact. The action is intentionally explicit and
    /// intended for a classroom catalog reset, never for normal startup.
    pub fn reset_local_customizations(&self) -> Result<(), ProfileError> {
        self.require_instructor()?;
        let mut runtime = self.lock()?;
        let mut next = runtime.clone();
        let factory_keys = next.factory_keys.clone();
        next.profiles.retain(|key, _| factory_keys.contains(key));
        next.retired.clear();
        next.active_versions = next
            .profiles
            .values()
            .map(|profile| (profile.profile_id.clone(), profile.profile_version.clone()))
            .collect();
        next.completed_save_requests.clear();
        next.catalog_writable = true;
        next.catalog_load_error = None;
        let persisted = self.persisted_state(&next);
        let state_path = self.root.join("lab_state.json");
        let quarantined = if !runtime.catalog_writable && state_path.exists() {
            let path = self.root.join(format!(
                "lab_state.corrupt-{}.json",
                Utc::now().format("%Y%m%dT%H%M%S%.3fZ")
            ));
            fs::rename(&state_path, &path).map_err(|source| ProfileError::Write {
                path: path.display().to_string(),
                source,
            })?;
            Some(path)
        } else {
            None
        };
        if let Err(error) = self.persist_state(&persisted) {
            if let Some(path) = &quarantined {
                let _ = fs::rename(path, &state_path);
            }
            return Err(error);
        }
        *runtime = next;
        drop(runtime);
        self.append_lab_write_log_best_effort(
            "reset_local_customizations",
            "all",
            None,
            None,
            "explicit",
        );
        Ok(())
    }
    pub fn import_profile(&self, path: &Path) -> Result<AcquisitionProfile, ProfileError> {
        self.require_instructor()?;
        self.require_catalog_writable()?;
        let mut profile = load_profile(path)?;
        if profile.status != ProfileStatus::Locked {
            return Err(ProfileError::Validation(
                "only finalized locked profile packages may be imported".into(),
            ));
        }
        let mut runtime = self.lock()?;
        let key = pkey(&profile);
        if let Some(existing) = runtime.profiles.get(&key) {
            if existing.canonical_bytes()? == profile.canonical_bytes()? {
                return Err(ProfileError::Validation(
                    "this exact lab version is already installed; no catalog change was made"
                        .into(),
                ));
            }
            return Err(ProfileError::Validation(
                "an installed lab has the same ID and version but different content; import it with a new lab ID"
                    .into(),
            ));
        }
        profile.source = ProfileSource::Imported;
        profile.integrity.canonical_hash.clear();
        profile.refresh_hash()?;
        profile.validate()?;
        let mut next = runtime.clone();
        next.profiles.insert(key, profile.clone());
        next.active_versions
            .insert(profile.profile_id.clone(), profile.profile_version.clone());
        let persisted = self.persisted_state(&next);
        self.persist_state(&persisted)?;
        *runtime = next;
        drop(runtime);
        self.append_lab_write_log_best_effort(
            "import",
            &profile.profile_id,
            None,
            Some(&profile.profile_version),
            "explicit",
        );
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
    fn require_catalog_writable(&self) -> Result<(), ProfileError> {
        let runtime = self.lock()?;
        if runtime.catalog_writable {
            Ok(())
        } else {
            Err(ProfileError::CatalogUnavailable(
                runtime
                    .catalog_load_error
                    .clone()
                    .unwrap_or_else(|| "unknown catalog initialization failure".into()),
            ))
        }
    }
    fn persisted_state(&self, runtime: &ProfileRuntime) -> PersistedLabState {
        PersistedLabState {
            profiles: runtime
                .profiles
                .values()
                .filter(|profile| profile.source != ProfileSource::BuiltIn)
                .cloned()
                .collect(),
            retired: runtime
                .retired
                .iter()
                .filter(|key| !runtime.factory_keys.contains(*key))
                .cloned()
                .collect(),
            active_versions: runtime
                .active_versions
                .iter()
                .filter(|(id, version)| {
                    !runtime
                        .factory_keys
                        .contains(&((*id).clone(), (*version).clone()))
                })
                .map(|(id, version)| (id.clone(), version.clone()))
                .collect(),
            completed_save_requests: runtime.completed_save_requests.clone(),
        }
    }
    fn persist_state(&self, state: &PersistedLabState) -> Result<(), ProfileError> {
        fs::create_dir_all(&self.root).map_err(|source| ProfileError::Write {
            path: self.root.display().to_string(),
            source,
        })?;
        let path = self.root.join("lab_state.json");
        let temporary = self.root.join("lab_state.json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(state)?).map_err(|source| {
            ProfileError::Write {
                path: temporary.display().to_string(),
                source,
            }
        })?;
        let backup = self.root.join("lab_state.json.backup");
        if backup.exists() {
            fs::remove_file(&backup).map_err(|source| ProfileError::Write {
                path: backup.display().to_string(),
                source,
            })?;
        }
        let had_previous = path.exists();
        if had_previous {
            fs::rename(&path, &backup).map_err(|source| ProfileError::Write {
                path: backup.display().to_string(),
                source,
            })?;
        }
        if let Err(source) = fs::rename(&temporary, &path) {
            if had_previous {
                let _ = fs::rename(&backup, &path);
            }
            return Err(ProfileError::Write {
                path: path.display().to_string(),
                source,
            });
        }
        if had_previous {
            if let Err(error) = fs::remove_file(&backup) {
                // The new catalog is already installed at this commit point.
                // Cleanup failure must not make the caller retain the old
                // in-memory state while disk contains the new one.
                crate::app_log::record(
                    "WARN",
                    &format!(
                        "LAB_CATALOG_BACKUP_CLEANUP_FAILED path={} detail={error}",
                        backup.display()
                    ),
                );
            }
        }
        Ok(())
    }
    fn append_mode_log_best_effort(&self, mode: &ProfileMode) {
        if let Err(error) = self.append_mode_log(mode) {
            crate::app_log::record(
                "WARN",
                &format!("LAB_CATALOG_AUDIT_LOG_FAILED operation=set_mode detail={error}"),
            );
        }
    }
    fn append_lab_write_log_best_effort(
        &self,
        operation: &str,
        lab_id: &str,
        base_version: Option<&str>,
        new_version: Option<&str>,
        request_id: &str,
    ) {
        if let Err(error) =
            self.append_lab_write_log(operation, lab_id, base_version, new_version, request_id)
        {
            crate::app_log::record(
                "WARN",
                &format!(
                    "LAB_CATALOG_AUDIT_LOG_FAILED operation={operation} lab_id={lab_id} detail={error}"
                ),
            );
        }
    }
    fn append_mode_log(&self, mode: &ProfileMode) -> Result<(), ProfileError> {
        let path = self.root.join("mode_changes.log");
        use std::io::Write;
        let mut file = self.open_catalog_log(&path)?;
        writeln!(file, "{}\t{:?}", Utc::now().to_rfc3339(), mode).map_err(|source| {
            ProfileError::Write {
                path: path.display().to_string(),
                source,
            }
        })
    }
    fn append_lab_write_log(
        &self,
        operation: &str,
        lab_id: &str,
        base_version: Option<&str>,
        new_version: Option<&str>,
        request_id: &str,
    ) -> Result<(), ProfileError> {
        let path = self.root.join("lab_write_audit.log");
        use std::io::Write;
        let mut file = self.open_catalog_log(&path)?;
        writeln!(
            file,
            "{}\tLAB_WRITE\t{}\t{}\t{}\t{}\trequest={}",
            Utc::now().to_rfc3339(),
            operation,
            lab_id,
            base_version.unwrap_or("-"),
            new_version.unwrap_or("-"),
            request_id
        )
        .map_err(|source| ProfileError::Write {
            path: path.display().to_string(),
            source,
        })
    }
    fn open_catalog_log(&self, path: &Path) -> Result<fs::File, ProfileError> {
        fs::create_dir_all(&self.root).map_err(|source| ProfileError::Write {
            path: self.root.display().to_string(),
            source,
        })?;
        rotate_catalog_log(path)?;
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|source| ProfileError::Write {
                path: path.display().to_string(),
                source,
            })
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ProfileRuntime>, ProfileError> {
        self.runtime
            .lock()
            .map_err(|_| ProfileError::Validation("profile store lock poisoned".into()))
    }
}

fn rotate_catalog_log(path: &Path) -> Result<(), ProfileError> {
    let current_bytes = fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if current_bytes < MAX_CATALOG_LOG_BYTES {
        return Ok(());
    }
    let oldest = path.with_extension(RETAINED_CATALOG_LOG_FILES.to_string());
    if oldest.exists() {
        fs::remove_file(&oldest).map_err(|source| ProfileError::Write {
            path: oldest.display().to_string(),
            source,
        })?;
    }
    for index in (1..RETAINED_CATALOG_LOG_FILES).rev() {
        let from = path.with_extension(index.to_string());
        let to = path.with_extension((index + 1).to_string());
        if from.exists() {
            fs::rename(&from, &to).map_err(|source| ProfileError::Write {
                path: to.display().to_string(),
                source,
            })?;
        }
    }
    if path.exists() {
        fs::rename(path, path.with_extension("1")).map_err(|source| ProfileError::Write {
            path: path.display().to_string(),
            source,
        })?;
    }
    Ok(())
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
fn valid_operation_id(value: &str) -> bool {
    (8..=128).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}
fn valid_semver(value: &str) -> bool {
    let fields: Vec<_> = value.split('.').collect();
    fields.len() == 3
        && fields
            .iter()
            .all(|field| !field.is_empty() && field.chars().all(|c| c.is_ascii_digit()))
}
fn version_key(value: &str) -> (u64, u64, u64) {
    let parts: Vec<u64> = value
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect();
    (
        parts.first().copied().unwrap_or(0),
        parts.get(1).copied().unwrap_or(0),
        parts.get(2).copied().unwrap_or(0),
    )
}
fn next_patch_version(value: &str) -> Result<String, ProfileError> {
    if !valid_semver(value) {
        return Err(ProfileError::Validation(
            "lab version must be MAJOR.MINOR.PATCH".into(),
        ));
    }
    let (major, minor, patch) = version_key(value);
    Ok(format!("{major}.{minor}.{}", patch.saturating_add(1)))
}
fn validate_plot_defaults(
    defaults: &PlotDefaults,
    channel_ids: &[String],
) -> Result<(), ProfileError> {
    if defaults.groups.is_empty() {
        return Ok(());
    }
    let known: BTreeSet<_> = channel_ids.iter().cloned().collect();
    let mut assigned = BTreeSet::new();
    if defaults.groups.iter().any(|group| {
        group.channel_ids.is_empty()
            || group
                .channel_ids
                .iter()
                .any(|id| !known.contains(id) || !assigned.insert(id.clone()))
    }) || assigned != known
    {
        return Err(ProfileError::Validation(
            "plot defaults must assign every display signal to exactly one non-empty plot group"
                .into(),
        ));
    }
    Ok(())
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
    fn builtin_hashes_are_deterministic() {
        let profiles = built_in_profiles().unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(profiles.len(), 5);
        assert!(profiles
            .iter()
            .all(|profile| profile.verify_integrity().is_ok()));
    }

    #[test]
    fn unavailable_catalog_keeps_factory_labs_and_explicit_reset_recovers_writes() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        fs::create_dir_all(&root).unwrap_or_else(|error| panic!("{error}"));
        fs::write(root.join("lab_state.json"), b"{ invalid catalog")
            .unwrap_or_else(|error| panic!("{error}"));
        let store = ProfileStore::factory_only(root.clone(), "invalid local catalog JSON".into());
        assert_eq!(store.list().unwrap_or_default().len(), 5);
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .reset_local_customizations()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(root.join("lab_state.json").is_file());
        assert!(fs::read_dir(&root)
            .unwrap_or_else(|error| panic!("{error}"))
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("lab_state.corrupt-")));
        let reopened = ProfileStore::with_root(root).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(reopened.list().unwrap_or_default().len(), 5);
    }

    #[test]
    fn failed_catalog_persistence_does_not_mutate_runtime_state() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        let mut draft = store
            .begin_lab_edit("wvu.bmeg420l.ecg.course.capture.v1")
            .unwrap_or_else(|error| panic!("{error}"));
        draft
            .description
            .push_str(" must not survive a failed write");

        fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("{error}"));
        fs::write(&root, b"not a directory").unwrap_or_else(|error| panic!("{error}"));
        assert!(store
            .save_lab_draft(draft, Some("1.0.0".into()), "save-fails-0001".into())
            .is_err());

        let entries = store.list_all().unwrap_or_default();
        assert_eq!(entries.len(), 5);
        assert_eq!(
            store
                .get_locked("wvu.bmeg420l.ecg.course.capture.v1")
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "1.0.0"
        );
    }

    #[test]
    fn catalog_audit_logs_rotate_when_they_reach_their_limit() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = dir.path().join("lab_write_audit.log");
        fs::write(&path, vec![b'x'; MAX_CATALOG_LOG_BYTES as usize])
            .unwrap_or_else(|error| panic!("{error}"));
        rotate_catalog_log(&path).unwrap_or_else(|error| panic!("{error}"));
        assert!(path.with_extension("1").is_file());
        assert!(!path.exists());
    }

    #[test]
    fn course_profile_maps_preserve_required_synchronized_variables() {
        let profiles = built_in_profiles().unwrap_or_else(|error| panic!("{error}"));
        let lookup = |category: &str| {
            profiles
                .iter()
                .find(|profile| profile.category == category)
                .unwrap_or_else(|| panic!("missing {category}"))
        };
        let ecg = lookup("course_ecg");
        assert_eq!(ecg.acquisition.record_field_names(), ["ecg_counts"]);
        assert_eq!(ecg.acquisition.adc_resolution_bits, 14);
        let emg = lookup("course_emg_force");
        assert_eq!(emg.acquisition.adc_resolution_bits, 14);
        assert_eq!(emg.acquisition.analog_pins(), ["A0", "A1", "A2", "A3"]);
        assert_eq!(
            emg.acquisition.record_field_names(),
            [
                "raw_emg_counts",
                "rectified_emg_counts",
                "emg_envelope_counts",
                "pressure_counts"
            ]
        );
        let bp = lookup("course_blood_pressure");
        assert_eq!(bp.acquisition.adc_resolution_bits, 14);
        assert_eq!(bp.acquisition.analog_pins(), ["A0", "A1", "A2"]);
        assert_eq!(
            bp.acquisition
                .led_outputs
                .as_ref()
                .and_then(|outputs| outputs.green.as_deref()),
            Some("D4")
        );
        let pulseox = lookup("course_pulseox");
        assert_eq!(
            pulseox.acquisition.acquisition_mode,
            AcquisitionMode::Pulseox4State
        );
        assert_eq!(pulseox.acquisition.analog_pins(), ["A0", "A1"]);
        assert_eq!(pulseox.acquisition.record_field_names().len(), 8);
        assert_eq!(pulseox.acquisition.state_dwell_us, Some(1_000));
        assert_eq!(pulseox.acquisition.adc_resolution_bits, 14);
        let general = lookup("development");
        assert_eq!(general.acquisition.adc_resolution_bits, 14);
    }

    #[test]
    fn profile_validation_rejects_duplicate_pins_and_bad_pulseox_mapping() {
        let mut profile = built_in_profiles().unwrap_or_else(|error| panic!("{error}"))[2].clone();
        profile.acquisition.channels[1].pin = "A0".into();
        profile.integrity.canonical_hash.clear();
        assert!(profile.validate().is_err());
        let mut pulseox = built_in_profiles().unwrap_or_else(|error| panic!("{error}"))[4].clone();
        pulseox.acquisition.state_dwell_us = Some(900);
        pulseox.integrity.canonical_hash.clear();
        assert!(pulseox.validate().is_err());
    }
    #[test]
    fn builtin_profiles_are_valid_and_hashed() {
        let profiles = built_in_profiles().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(profiles.len(), 5);
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
    fn drafts_are_detached_and_one_explicit_save_creates_one_revision() {
        let dir = tempdir().unwrap_or_else(|e| panic!("{e}"));
        let root = dir.path().join("profiles");
        let store = ProfileStore::with_root(root.clone()).unwrap_or_else(|e| panic!("{e}"));
        assert!(store
            .begin_lab_edit("wvu.bmeg420l.ecg.course.capture.v1")
            .is_err());
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|e| panic!("{e}"));
        let mut draft = store
            .begin_lab_edit("wvu.bmeg420l.ecg.course.capture.v1")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(draft.profile_version, "1.0.0");
        assert_eq!(store.list_all().unwrap_or_default().len(), 5);
        assert!(!root.join("lab_state.json").exists());
        draft.description.push_str(" edited");
        let final_profile = store
            .save_lab_draft(draft.clone(), Some("1.0.0".into()), "save-ecg-0001".into())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(final_profile.verify_integrity().is_ok());
        assert_eq!(final_profile.profile_version, "1.0.1");
        assert_eq!(store.list_all().unwrap_or_default().len(), 6);
        let retried = store
            .save_lab_draft(draft, Some("1.0.0".into()), "save-ecg-0001".into())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(retried.profile_version, "1.0.1");
        assert_eq!(store.list_all().unwrap_or_default().len(), 6);
        let audit =
            fs::read_to_string(root.join("lab_write_audit.log")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(audit.lines().count(), 1);
        assert!(audit.contains("save_new_version"));
    }

    #[test]
    fn factory_defaults_are_available_after_repeated_read_only_initialization() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        for _ in 0..100 {
            let store =
                ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
            let names: Vec<_> = store
                .list()
                .unwrap_or_else(|error| panic!("{error}"))
                .into_iter()
                .map(|profile| profile.display_name)
                .collect();
            assert_eq!(names.len(), 5);
            assert!(names.contains(&"ECG — Course Capture".into()));
            assert!(names.contains(&"EMG + Force — Course Capture".into()));
            assert!(names.contains(&"Blood Pressure + PPG — Course Capture".into()));
            assert!(names.contains(&"Pulse Oximetry — TX + RX Raw Capture".into()));
            assert!(names.contains(&"General Analog — Development".into()));
        }
        assert!(!root.join("lab_state.json").exists());
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

    #[test]
    fn lab_edit_creates_next_active_revision_and_persists_history() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("labs");
        let store = ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        let mut draft = store
            .begin_lab_edit("wvu.bmeg420l.ecg.course.capture.v1")
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(draft.profile_version, "1.0.0");
        draft.acquisition.channels[0].pin = "A2".into();
        let saved = store
            .save_lab_draft(draft, Some("1.0.0".into()), "save-ecg-0002".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(saved.profile_version, "1.0.1");
        assert_eq!(saved.acquisition.analog_pins(), ["A2"]);
        assert!(saved.verify_integrity().is_ok());
        assert_eq!(
            store
                .get_locked("wvu.bmeg420l.ecg.course.capture.v1")
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "1.0.1"
        );
        let reopened = ProfileStore::with_root(root).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            reopened
                .get_locked("wvu.bmeg420l.ecg.course.capture.v1")
                .unwrap_or_else(|error| panic!("{error}"))
                .acquisition
                .analog_pins(),
            ["A2"]
        );
    }

    #[test]
    fn lab_editor_rejects_pin_conflicts_and_invalid_pulseox_resources() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let store =
            ProfileStore::with_root(dir.path().into()).unwrap_or_else(|error| panic!("{error}"));
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        let mut emg = store
            .begin_lab_edit("wvu.bmeg420l.emg.force.course.capture.v1")
            .unwrap_or_else(|error| panic!("{error}"));
        emg.acquisition.channels[1].pin = "A0".into();
        assert!(store
            .save_lab_draft(emg, Some("1.0.0".into()), "invalid-emg-0001".into())
            .is_err());
        let mut pulse = store
            .begin_lab_edit("wvu.bmeg420l.pulseox.txrx.raw.course.capture.v1")
            .unwrap_or_else(|error| panic!("{error}"));
        pulse.acquisition.digital_outputs[1].pin = "D5".into();
        assert!(store
            .save_lab_draft(pulse, Some("1.0.0".into()), "invalid-pulse-001".into())
            .is_err());
    }

    #[test]
    fn blank_simultaneous_template_starts_as_a_saveable_instructor_draft() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let store =
            ProfileStore::with_root(dir.path().into()).unwrap_or_else(|error| panic!("{error}"));
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        let draft = store
            .create_blank_simultaneous_lab("team.blank")
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(draft.status, ProfileStatus::Draft);
        assert_eq!(
            draft.acquisition.acquisition_mode,
            AcquisitionMode::Simultaneous
        );
        assert_eq!(draft.acquisition.resolved_channels().len(), 1);
        let saved = store
            .save_lab_draft(draft, None, "save-blank-0001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(saved.status, ProfileStatus::Locked);
        assert!(store.get_locked("team.blank").is_ok());
    }

    #[test]
    fn retired_lab_can_be_restored_and_course_default_creates_new_revision() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let store =
            ProfileStore::with_root(dir.path().into()).unwrap_or_else(|error| panic!("{error}"));
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        let draft = store
            .duplicate_lab("wvu.bmeg420l.ecg.course.capture.v1", "team.ecg")
            .unwrap_or_else(|error| panic!("{error}"));
        let saved = store
            .save_lab_draft(draft, None, "save-team-ecg-01".into())
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .retire(&saved.profile_id, &saved.profile_version)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(store.get_locked("team.ecg").is_err());
        store
            .restore_retired(&saved.profile_id, &saved.profile_version)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(store.get_locked("team.ecg").is_ok());
        let restored = store
            .restore_course_default("wvu.bmeg420l.ecg.course.capture.v1")
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(restored.profile_version, "1.0.0");
        assert_eq!(restored.source, ProfileSource::BuiltIn);
        assert!(store
            .retire("wvu.bmeg420l.blood_pressure.ppg.course.capture.v1", "1.0.0")
            .is_err());
    }

    #[test]
    fn selection_navigation_and_restart_never_create_a_ghost_revision() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        let mut draft = store
            .begin_lab_edit("wvu.bmeg420l.emg.force.course.capture.v1")
            .unwrap_or_else(|error| panic!("{error}"));
        draft.description.push_str(" instructor revision");
        let saved = store
            .save_lab_draft(draft, Some("1.0.0".into()), "save-emg-0001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(saved.profile_version, "1.0.1");

        for _ in 0..20 {
            assert!(store.list().is_ok());
            assert!(store
                .get_locked("wvu.bmeg420l.emg.force.course.capture.v1")
                .is_ok());
            // Preview/edit initialization is a detached draft and must not
            // become a historical catalog row until an explicit Save.
            assert!(store
                .begin_lab_edit("wvu.bmeg420l.emg.force.course.capture.v1")
                .is_ok());
        }
        let versions: Vec<_> = store
            .list_all()
            .unwrap_or_else(|error| panic!("{error}"))
            .into_iter()
            .filter(|entry| entry.profile.profile_id == "wvu.bmeg420l.emg.force.course.capture.v1")
            .map(|entry| entry.profile.profile_version)
            .collect();
        assert_eq!(versions, vec!["1.0.0", "1.0.1"]);

        let reopened = ProfileStore::with_root(root).unwrap_or_else(|error| panic!("{error}"));
        reopened
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        let reopened_versions: Vec<_> = reopened
            .list_all()
            .unwrap_or_else(|error| panic!("{error}"))
            .into_iter()
            .filter(|entry| entry.profile.profile_id == "wvu.bmeg420l.emg.force.course.capture.v1")
            .map(|entry| entry.profile.profile_version)
            .collect();
        assert_eq!(reopened_versions, vec!["1.0.0", "1.0.1"]);
        assert_eq!(
            reopened
                .get_locked("wvu.bmeg420l.emg.force.course.capture.v1")
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "1.0.1"
        );
    }

    #[test]
    fn stale_save_and_import_collisions_are_non_mutating() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        let mut first = store
            .begin_lab_edit("wvu.bmeg420l.ecg.course.capture.v1")
            .unwrap_or_else(|error| panic!("{error}"));
        let mut stale = first.clone();
        first.description.push_str(" first");
        stale.description.push_str(" stale");
        store
            .save_lab_draft(first, Some("1.0.0".into()), "save-stale-001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(store
            .save_lab_draft(stale, Some("1.0.0".into()), "save-stale-002".into())
            .is_err());

        let exact = built_in_profiles()
            .unwrap_or_else(|error| panic!("{error}"))
            .into_iter()
            .find(|profile| profile.category == "course_blood_pressure")
            .unwrap_or_else(|| panic!("missing BP factory lab"));
        let package = root.join("factory-bp.lab.json");
        fs::create_dir_all(&root).unwrap_or_else(|error| panic!("{error}"));
        fs::write(
            &package,
            serde_json::to_vec_pretty(&exact).unwrap_or_default(),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert!(store.import_profile(&package).is_err());
        let entries = store.list_all().unwrap_or_default();
        assert_eq!(entries.len(), 6);
        assert!(!entries
            .iter()
            .any(|entry| entry.profile.profile_version == "1.0.2"));
    }

    #[test]
    fn reset_local_customizations_keeps_factory_labs_and_recording_history_separate() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = ProfileStore::with_root(root).unwrap_or_else(|error| panic!("{error}"));
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        let mut draft = store
            .begin_lab_edit("wvu.bmeg420l.ecg.course.capture.v1")
            .unwrap_or_else(|error| panic!("{error}"));
        draft.description.push_str(" local");
        store
            .save_lab_draft(draft, Some("1.0.0".into()), "save-reset-001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .reset_local_customizations()
            .unwrap_or_else(|error| panic!("{error}"));
        let active = store.list().unwrap_or_default();
        assert_eq!(active.len(), 5);
        assert_eq!(
            store
                .get_locked("wvu.bmeg420l.ecg.course.capture.v1")
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "1.0.0"
        );
    }
}
