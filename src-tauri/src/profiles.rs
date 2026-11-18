//! Versioned, validated acquisition profiles. SHA-256 here detects accidental
//! modification; it is explicitly not an authorship signature or access-control mechanism.
use crate::protocol::{
    PROTOCOL_MAJOR, PROTOCOL_MINOR, REFERENCE_DEVICE_ID, REFERENCE_FIRMWARE_BUILD,
};
use crate::recording::{derived_column_name, voltage_column_name, RecordingDuration};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub const PROFILE_SCHEMA_VERSION: u32 = 1;
pub const UNO_R4_WIFI_FQBN: &str = "arduino:renesas_uno:unor4wifi";
pub const SUPPORTED_ANALOG_PINS: [&str; 6] = ["A0", "A1", "A2", "A3", "A4", "A5"];
pub const SUPPORTED_DIGITAL_OUTPUT_PINS: [&str; 3] = ["D4", "D5", "D6"];
/// The simultaneous frame rates the controlled WVU firmware accepts
/// (`supportedFrameRate` in the reference sketch and its CAPABILITIES reply).
pub const RECOMMENDED_SAMPLE_RATES_HZ: [u32; 5] = [100, 200, 250, 500, 1_000];
/// Pulse-ox state dwell accepted by the controlled firmware, in microseconds.
pub const PULSEOX_DWELL_RANGE_US: std::ops::RangeInclusive<u32> = 250..=5_000;
/// Fixed CSV columns that a channel's CSV field may not reuse.
pub const RESERVED_CSV_COLUMNS: [&str; 3] = ["record_sequence", "t_us", "cycle_index"];
/// Text limits for a revision being saved or imported now. Stored labs are
/// never rejected by these limits; the recording header budget still applies.
pub const MAX_PROFILE_ID_CHARS: usize = 128;
pub const MAX_LAB_NAME_CHARS: usize = 100;
pub const MAX_LAB_CATEGORY_CHARS: usize = 64;
pub const MAX_LAB_DESCRIPTION_CHARS: usize = 2_000;
pub const MAX_LAB_LABEL_CHARS: usize = 80;
pub const MAX_LAB_IDENTIFIER_CHARS: usize = 64;
pub const MAX_LAB_NOTICES: usize = 12;
pub const MAX_LAB_NOTICE_CHARS: usize = 500;
const MAX_CATALOG_LOG_BYTES: u64 = 512 * 1024;
const RETAINED_CATALOG_LOG_FILES: usize = 3;
const CATALOG_FILE: &str = "lab_state.json";
/// Lock file that serializes catalog writes between application windows.
const CATALOG_LOCK_FILE: &str = "lab_state.lock";
/// Written by earlier builds between two renames; a leftover one is a
/// recovery candidate when the catalog itself is missing or unreadable.
const LEGACY_CATALOG_BACKUP: &str = "lab_state.json.backup";
const CATALOG_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
static CATALOG_TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

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
    /// Explicit pulse-ox RED/IR assignment. It is optional and omitted when
    /// absent, so the canonical bytes (and hashes) of existing labs do not
    /// change. Without it the historical label rule applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pulse_ox_role: Option<PulseOxRole>,
}

/// Which pulse-ox LED a sequenced output drives.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PulseOxRole {
    Red,
    Ir,
}

impl PulseOxRole {
    /// Key used in recording metadata `digital_output_mapping`.
    pub fn mapping_key(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Ir => "ir",
        }
    }
}

/// The historical rule: an output labelled "Red LED" or "IR LED" (ASCII case
/// ignored) is the RED or IR output. `lab-authoring.ts` mirrors it exactly.
fn legacy_pulse_ox_role(label: &str) -> Option<PulseOxRole> {
    if label.eq_ignore_ascii_case("red led") {
        Some(PulseOxRole::Red)
    } else if label.eq_ignore_ascii_case("ir led") {
        Some(PulseOxRole::Ir)
    } else {
        None
    }
}

impl DigitalOutput {
    /// The explicit role, or the label rule when no role is stored.
    pub fn effective_pulse_ox_role(&self) -> Option<PulseOxRole> {
        self.pulse_ox_role
            .or_else(|| legacy_pulse_ox_role(&self.label))
    }
}

/// The one shared RED/IR selection rule (also used by acquisition and
/// metadata): an output that names the role explicitly wins; otherwise the
/// first output without an explicit role whose label matches.
pub fn pulse_ox_output(outputs: &[DigitalOutput], role: PulseOxRole) -> Option<&DigitalOutput> {
    outputs
        .iter()
        .find(|output| output.pulse_ox_role == Some(role))
        .or_else(|| {
            outputs.iter().find(|output| {
                output.pulse_ox_role.is_none() && legacy_pulse_ox_role(&output.label) == Some(role)
            })
        })
}

/// Nominal pulse-ox cycle rate for a state dwell: `round(1e6 / (4 × dwell))`,
/// the same value the lab editor derives.
pub fn pulse_ox_cycle_rate_hz(state_dwell_us: u32) -> u32 {
    if state_dwell_us == 0 {
        return 0;
    }
    let dwell = u64::from(state_dwell_us);
    u32::try_from((500_000 + dwell) / (2 * dwell)).unwrap_or(u32::MAX)
}

/// CSV field names for new or imported revisions: 1–64 ASCII letters,
/// digits, or underscores (a leading digit is allowed, e.g. `01_counts`).
pub fn valid_csv_field_name(value: &str) -> bool {
    (1..=MAX_LAB_IDENTIFIER_CHARS).contains(&value.len())
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
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
                pulse_ox_role: None,
            });
        }
        if let Some(pin) = &legacy.red {
            outputs.push(DigitalOutput {
                pin: pin.clone(),
                label: "Red LED".into(),
                behavior: DigitalOutputBehavior::AcquisitionSequenced,
                pulse_ox_role: None,
            });
        }
        if let Some(pin) = &legacy.ir {
            outputs.push(DigitalOutput {
                pin: pin.clone(),
                label: "IR LED".into(),
                behavior: DigitalOutputBehavior::AcquisitionSequenced,
                pulse_ox_role: None,
            });
        }
        outputs
    }

    /// The RED or IR output of a pulse-ox lab, by the shared selection rule.
    pub fn pulse_ox_output(&self, role: PulseOxRole) -> Option<DigitalOutput> {
        pulse_ox_output(&self.resolved_digital_outputs(), role).cloned()
    }

    /// Why this lab refuses the requested duration, as student-facing text,
    /// or `None` when the lab allows it. A preset duration is always allowed;
    /// any other timed duration must reach the lab's custom minimum.
    pub fn duration_refusal(&self, duration: &RecordingDuration) -> Option<String> {
        let mode = duration.label();
        if !self
            .allowed_duration_modes
            .iter()
            .any(|allowed| allowed == mode)
        {
            return Some(match duration {
                RecordingDuration::Timed { .. } => {
                    "This lab does not allow timed recordings. Choose Until stopped, then start again."
                        .into()
                }
                RecordingDuration::UntilStopped => {
                    "This lab does not allow Until-stopped recordings. Choose a timed duration, then start again."
                        .into()
                }
            });
        }
        if let RecordingDuration::Timed { seconds } = duration {
            let minimum = self
                .minimum_custom_duration_seconds
                .max(RecordingDuration::MIN_TIMED_SECONDS);
            if *seconds < minimum && !self.timed_presets_seconds.contains(seconds) {
                return Some(format!(
                    "This lab requires timed recordings of at least {minimum} seconds, or one of its preset durations. Choose a longer duration, then start again."
                ));
            }
        }
        None
    }

    /// A setting the controlled WVU firmware cannot run. It is checked before
    /// a hardware recording so the refusal is not reported as a handshake
    /// failure after the board was contacted.
    pub fn firmware_configuration_issue(&self) -> Option<&'static str> {
        match self.acquisition_mode {
            AcquisitionMode::Simultaneous
                if !RECOMMENDED_SAMPLE_RATES_HZ.contains(&self.sample_rate_hz) =>
            {
                Some("This lab's frame rate is not supported by the WVU firmware (100, 200, 250, 500, or 1000 frames/s). Ask your instructor to update the lab.")
            }
            AcquisitionMode::Pulseox4State
                if !self
                    .state_dwell_us
                    .is_some_and(|dwell| PULSEOX_DWELL_RANGE_US.contains(&dwell)) =>
            {
                Some("This lab's pulse-ox state dwell is not supported by the WVU firmware (250-5000 µs). Ask your instructor to update the lab.")
            }
            _ => None,
        }
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
    #[error("the local lab catalog is being changed by another WVU Bioinstrumentation Studio window; wait a moment or close the other window, then try again")]
    CatalogBusy,
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
                let red = pulse_ox_output(&outputs, PulseOxRole::Red);
                let ir = pulse_ox_output(&outputs, PulseOxRole::Ir);
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
            || ![
                REFERENCE_FIRMWARE_BUILD,
                0x0001_0005,
                0x0001_0004,
                0x0001_0003,
                0x0001_0002,
                0x0001_0001,
            ]
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

    /// Stricter rules for a revision created now by Save or Import. Stored
    /// labs are loaded with `validate` alone, so a rule added here can never
    /// hide a lab that is already installed (an older lab is checked again
    /// only when an instructor saves a new revision of it).
    pub fn validate_for_authoring(&self) -> Result<(), ProfileError> {
        self.validate()?;
        self.check_text_bounds()?;
        let outputs = self.acquisition.resolved_digital_outputs();
        match self.acquisition.acquisition_mode {
            AcquisitionMode::Simultaneous => {
                if !RECOMMENDED_SAMPLE_RATES_HZ.contains(&self.acquisition.sample_rate_hz) {
                    return Err(ProfileError::Validation(
                        "frame rate must be one the WVU firmware supports: 100, 200, 250, 500, or 1000 frames/s"
                            .into(),
                    ));
                }
                self.check_csv_columns()?;
                if outputs.iter().any(|output| output.pulse_ox_role.is_some()) {
                    return Err(ProfileError::Validation(
                        "a pulse-ox RED/IR role can be set only in a pulse-ox lab".into(),
                    ));
                }
            }
            AcquisitionMode::Pulseox4State => {
                if !self.acquisition.channels.is_empty() {
                    return Err(ProfileError::Validation(
                        "pulse-ox labs record the fixed TX/RX phase fields and must not define analog channels"
                            .into(),
                    ));
                }
                let dwell = self.acquisition.state_dwell_us.unwrap_or_default();
                let expected = pulse_ox_cycle_rate_hz(dwell);
                if self.acquisition.sample_rate_hz != expected {
                    return Err(ProfileError::Validation(format!(
                        "the pulse-ox cycle rate must follow the state dwell: {expected} cycles/s for {dwell} µs per state"
                    )));
                }
                for role in [PulseOxRole::Red, PulseOxRole::Ir] {
                    if outputs
                        .iter()
                        .filter(|output| output.pulse_ox_role == Some(role))
                        .count()
                        > 1
                    {
                        return Err(ProfileError::Validation(
                            "each pulse-ox role (RED, IR) may be assigned to only one output"
                                .into(),
                        ));
                    }
                }
                let red = pulse_ox_output(&outputs, PulseOxRole::Red).map(|output| &output.pin);
                let ir = pulse_ox_output(&outputs, PulseOxRole::Ir).map(|output| &output.pin);
                if outputs
                    .iter()
                    .any(|output| Some(&output.pin) != red && Some(&output.pin) != ir)
                {
                    return Err(ProfileError::Validation(
                        "pulse-ox labs may define only their RED and IR outputs; remove the other outputs (the firmware keeps them LOW in this mode)"
                            .into(),
                    ));
                }
            }
        }
        Ok(())
    }

    fn check_text_bounds(&self) -> Result<(), ProfileError> {
        fn single_line(value: &str, limit: usize, what: &str) -> Result<(), ProfileError> {
            if value.chars().count() > limit || value.chars().any(char::is_control) {
                return Err(ProfileError::Validation(format!(
                    "{what} must be one line of {limit} characters or fewer"
                )));
            }
            Ok(())
        }
        fn multi_line(value: &str, limit: usize, what: &str) -> Result<(), ProfileError> {
            if value.chars().count() > limit
                || value.chars().any(|character| {
                    character.is_control() && !matches!(character, '\n' | '\r' | '\t')
                })
            {
                return Err(ProfileError::Validation(format!(
                    "{what} must be {limit} characters or fewer"
                )));
            }
            Ok(())
        }
        single_line(&self.profile_id, MAX_PROFILE_ID_CHARS, "the lab ID")?;
        single_line(&self.display_name, MAX_LAB_NAME_CHARS, "the lab name")?;
        single_line(&self.category, MAX_LAB_CATEGORY_CHARS, "the lab category")?;
        multi_line(
            &self.description,
            MAX_LAB_DESCRIPTION_CHARS,
            "the lab description",
        )?;
        single_line(
            &self.display.channel_label,
            MAX_LAB_NAME_CHARS,
            "the display channel label",
        )?;
        single_line(
            &self.export.signal_name,
            MAX_LAB_IDENTIFIER_CHARS,
            "the export signal name",
        )?;
        if self.safety.notices.len() > MAX_LAB_NOTICES {
            return Err(ProfileError::Validation(format!(
                "a lab may have at most {MAX_LAB_NOTICES} safety notices"
            )));
        }
        for notice in &self.safety.notices {
            multi_line(notice, MAX_LAB_NOTICE_CHARS, "each safety notice")?;
        }
        for channel in &self.acquisition.channels {
            single_line(&channel.id, MAX_LAB_IDENTIFIER_CHARS, "each channel ID")?;
            single_line(&channel.label, MAX_LAB_LABEL_CHARS, "each channel label")?;
        }
        for output in &self.acquisition.digital_outputs {
            single_line(&output.label, MAX_LAB_LABEL_CHARS, "each output label")?;
        }
        Ok(())
    }

    /// Every CSV column the lab can produce (raw, `_V`, and the MPXV `_kPa` /
    /// `_mmHg` columns) must be distinct, ignoring case, and must not reuse a
    /// fixed column. Linear-calibration columns are named by the student's
    /// unit at export time; the CSV writer renames those on a collision.
    fn check_csv_columns(&self) -> Result<(), ProfileError> {
        let channels = self.acquisition.resolved_channels();
        if let Some(channel) = channels
            .iter()
            .find(|channel| !valid_csv_field_name(&channel.csv_name))
        {
            return Err(ProfileError::Validation(format!(
                "CSV field \"{}\" must be 1-64 letters, digits, or underscores (no spaces, commas, or quotes)",
                channel.csv_name
            )));
        }
        let mut used: BTreeSet<String> = RESERVED_CSV_COLUMNS
            .iter()
            .map(|column| column.to_ascii_lowercase())
            .collect();
        for channel in &channels {
            let mut columns = vec![
                channel.csv_name.clone(),
                voltage_column_name(&channel.csv_name),
            ];
            if channel
                .allowed_conversions
                .iter()
                .any(|conversion| conversion == "mpxv_pressure")
            {
                columns.push(derived_column_name(&channel.csv_name, "kPa"));
                columns.push(derived_column_name(&channel.csv_name, "mmHg"));
            }
            for column in columns {
                if !used.insert(column.to_ascii_lowercase()) {
                    return Err(ProfileError::Validation(format!(
                        "CSV column \"{column}\" would appear twice or reuses a fixed column; rename a CSV field so raw, _V, and pressure columns are all distinct"
                    )));
                }
            }
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
    /// Stored revisions that could not be loaded. Each is kept verbatim and
    /// written back unchanged, so one bad entry never removes the others.
    skipped: Vec<SkippedEntry>,
    /// Stored active selections that point at a skipped revision. They are
    /// written back until that lab's active version is explicitly changed.
    preserved_active: BTreeMap<String, String>,
    /// SHA-256 of the catalog file as this process last read or wrote it
    /// (`None`: no file). A different file means another window changed it.
    disk_fingerprint: Option<String>,
    /// The file the catalog was recovered from at startup, if any.
    recovered_from: Option<String>,
    /// Unreadable leftovers of an interrupted save that were renamed aside
    /// at startup because no catalog file existed.
    kept_unreadable_files: Vec<String>,
}

/// A stored lab revision that failed to parse or validate.
#[derive(Clone)]
struct SkippedEntry {
    raw: Box<RawValue>,
    profile_id: Option<String>,
    profile_version: Option<String>,
    display_name: Option<String>,
    reason: String,
}

impl SkippedEntry {
    fn new(raw: Box<RawValue>, reason: String) -> Self {
        let identity = serde_json::from_str::<serde_json::Value>(raw.get()).ok();
        let field = |name: &str| {
            identity
                .as_ref()
                .and_then(|value| value.get(name))
                .and_then(serde_json::Value::as_str)
                .map(|text| text.chars().take(MAX_PROFILE_ID_CHARS).collect::<String>())
        };
        Self {
            profile_id: field("profile_id"),
            profile_version: field("profile_version"),
            display_name: field("display_name"),
            raw,
            reason,
        }
    }
    fn key(&self) -> Option<(String, String)> {
        Some((self.profile_id.clone()?, self.profile_version.clone()?))
    }
}

impl ProfileRuntime {
    /// True when any loaded or skipped revision uses this lab ID.
    fn lab_id_in_use(&self, profile_id: &str) -> bool {
        self.profiles.keys().any(|(id, _)| id == profile_id)
            || self
                .skipped
                .iter()
                .any(|entry| entry.profile_id.as_deref() == Some(profile_id))
    }
    fn revision_in_use(&self, key: &(String, String)) -> bool {
        self.profiles.contains_key(key)
            || self
                .skipped
                .iter()
                .any(|entry| entry.key().as_ref() == Some(key))
    }
    /// The shipped course default for a lab ID, if it has one.
    fn factory_version(&self, profile_id: &str) -> Option<String> {
        self.factory_keys
            .iter()
            .filter(|(id, _)| id == profile_id)
            .max_by_key(|(_, version)| version_key(version))
            .map(|(_, version)| version.clone())
    }
    fn factory_profiles(&self) -> Vec<AcquisitionProfile> {
        self.profiles
            .iter()
            .filter(|(key, _)| self.factory_keys.contains(*key))
            .map(|(_, profile)| profile.clone())
            .collect()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LabListEntry {
    pub profile: AcquisitionProfile,
    pub active: bool,
    pub retired: bool,
}

/// Read-only catalog health, available in Student and Instructor mode.
#[derive(Clone, Debug, Serialize)]
pub struct CatalogStatus {
    /// False when the local catalog could not be loaded; factory labs remain
    /// available and instructor changes are refused until it is reset.
    pub writable: bool,
    pub load_error: Option<String>,
    /// Stored labs that could not be loaded. They stay in the catalog file.
    pub skipped_labs: Vec<SkippedLab>,
    /// The leftover file the catalog was restored from at startup.
    pub recovered_from: Option<String>,
    /// No catalog file existed at startup and the leftovers of an
    /// interrupted save could not be read; they were kept under these names
    /// for manual recovery (additive field).
    pub kept_unreadable_files: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkippedLab {
    pub profile_id: Option<String>,
    pub profile_version: Option<String>,
    pub display_name: Option<String>,
    pub reason: String,
}

/// Result of "Reset local customizations".
#[derive(Clone, Debug, Serialize)]
pub struct LabResetOutcome {
    /// Copy of the catalog taken before the reset (`None`: there was no file).
    pub backup_path: Option<String>,
    /// Local (instructor or imported) revisions removed from the catalog.
    pub removed_local_revisions: usize,
}

/// The catalog file as written. Loaded profiles are serialized in struct
/// field order (as before), so the Op Amp Lab installer's re-hash of an
/// installed entry still matches; skipped entries are written back verbatim.
#[derive(Serialize)]
struct PersistedLabState<'a> {
    profiles: Vec<PersistedProfile<'a>>,
    retired: Vec<(String, String)>,
    active_versions: BTreeMap<String, String>,
    /// Durable idempotency records prevent a frontend retry of the same
    /// explicit Save action from allocating another version.
    completed_save_requests: BTreeMap<String, (String, String)>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum PersistedProfile<'a> {
    Loaded(&'a AcquisitionProfile),
    Skipped(&'a RawValue),
}

/// The catalog file as read. Each profile is kept raw so it can be loaded on
/// its own: one entry that fails never hides the others.
#[derive(Deserialize)]
struct StoredLabState {
    #[serde(default)]
    profiles: Vec<Box<RawValue>>,
    #[serde(default)]
    retired: Vec<(String, String)>,
    #[serde(default)]
    active_versions: BTreeMap<String, String>,
    #[serde(default)]
    completed_save_requests: BTreeMap<String, (String, String)>,
}

/// One parsed catalog file and the SHA-256 of its bytes.
struct CatalogSnapshot {
    state: StoredLabState,
    fingerprint: String,
}

/// Exclusive, cross-process lock around a catalog read-modify-write. It is
/// a separate file, so readers of `lab_state.json` are never blocked.
struct CatalogLock {
    file: fs::File,
}

impl CatalogLock {
    fn acquire(root: &Path) -> Result<Self, ProfileError> {
        Self::acquire_within(root, CATALOG_LOCK_TIMEOUT)
    }
    fn acquire_within(root: &Path, timeout: Duration) -> Result<Self, ProfileError> {
        fs::create_dir_all(root).map_err(|source| ProfileError::Write {
            path: root.display().to_string(),
            source,
        })?;
        let path = root.join(CATALOG_LOCK_FILE);
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| ProfileError::Write {
                path: path.display().to_string(),
                source,
            })?;
        let deadline = Instant::now() + timeout;
        loop {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(Self { file }),
                Err(error) if is_lock_contention(&error) => {
                    if Instant::now() >= deadline {
                        return Err(ProfileError::CatalogBusy);
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(source) => {
                    return Err(ProfileError::Write {
                        path: path.display().to_string(),
                        source,
                    })
                }
            }
        }
    }
}

impl Drop for CatalogLock {
    fn drop(&mut self) {
        // Closing the handle releases the lock as well.
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

fn is_lock_contention(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
        || (error.raw_os_error().is_some()
            && error.raw_os_error() == fs2::lock_contended_error().raw_os_error())
}

fn bytes_fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Reads a catalog file, retrying briefly while another program (antivirus,
/// a backup or sync agent) has it open.
fn read_catalog_bytes(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut attempt = 0u64;
    loop {
        match fs::read(path) {
            // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
            Err(error) if attempt < 4 && matches!(error.raw_os_error(), Some(5 | 32 | 33)) => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(50 * attempt));
            }
            result => return result,
        }
    }
}

/// Parses catalog bytes (a UTF-8 BOM is tolerated). Every failure here is
/// `ProfileError::Parse`: the bytes were read but are not a catalog.
fn parse_catalog_bytes(bytes: &[u8]) -> Result<CatalogSnapshot, ProfileError> {
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let state: StoredLabState = serde_json::from_slice(text)?;
    Ok(CatalogSnapshot {
        state,
        fingerprint: bytes_fingerprint(bytes),
    })
}

/// Reads and parses one catalog file. `Ok(None)`: the file does not exist.
/// An I/O failure is `ProfileError::Read`; content that does not parse is
/// `ProfileError::Parse`.
fn read_catalog_file(path: &Path) -> Result<Option<CatalogSnapshot>, ProfileError> {
    let bytes = match read_catalog_bytes(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ProfileError::Read {
                path: path.display().to_string(),
                source,
            })
        }
    };
    parse_catalog_bytes(&bytes).map(Some)
}

/// Parses and validates one stored lab exactly as the whole catalog used to
/// be parsed, so canonical hashes are computed from the same values.
fn load_stored_profile(raw: &RawValue) -> Result<AcquisitionProfile, String> {
    let profile: AcquisitionProfile =
        serde_json::from_str(raw.get()).map_err(|error| format!("invalid lab JSON: {error}"))?;
    profile.validate().map_err(|error| error.to_string())?;
    Ok(profile)
}

/// Builds the in-memory catalog from the factory labs plus one stored file.
fn runtime_with_catalog(
    factory: &[AcquisitionProfile],
    snapshot: Option<CatalogSnapshot>,
    recovered_from: Option<String>,
) -> ProfileRuntime {
    let mut profiles = BTreeMap::new();
    let mut factory_keys = BTreeSet::new();
    for profile in factory {
        let key = pkey(profile);
        factory_keys.insert(key.clone());
        profiles.insert(key, profile.clone());
    }
    let mut active_versions: BTreeMap<String, String> = profiles
        .values()
        .map(|profile| (profile.profile_id.clone(), profile.profile_version.clone()))
        .collect();
    let mut retired = BTreeSet::new();
    let mut completed_save_requests = BTreeMap::new();
    let mut skipped = Vec::new();
    let mut preserved_active = BTreeMap::new();
    let mut disk_fingerprint = None;
    if let Some(snapshot) = snapshot {
        disk_fingerprint = Some(snapshot.fingerprint);
        let persisted = snapshot.state;
        for raw in persisted.profiles {
            match load_stored_profile(&raw) {
                Ok(profile) => {
                    let key = pkey(&profile);
                    // Factory definitions are resolved from bundled resources. A
                    // stale local copy may never replace the class default merely
                    // because it has the same identity.
                    if !factory_keys.contains(&key) {
                        profiles.insert(key, profile);
                    }
                }
                Err(reason) => {
                    let entry = SkippedEntry::new(raw, reason);
                    crate::app_log::record(
                        "WARN",
                        &format!(
                            "LAB_PROFILE_SKIPPED id={} version={} reason={}",
                            entry.profile_id.as_deref().unwrap_or("-"),
                            entry.profile_version.as_deref().unwrap_or("-"),
                            entry.reason
                        ),
                    );
                    skipped.push(entry);
                }
            }
        }
        let skipped_keys: BTreeSet<_> = skipped.iter().filter_map(SkippedEntry::key).collect();
        retired.extend(
            persisted
                .retired
                .into_iter()
                .filter(|key| !factory_keys.contains(key)),
        );
        for (id, version) in persisted.active_versions {
            let key = (id.clone(), version.clone());
            if factory_keys.contains(&key) {
                continue;
            }
            if profiles.contains_key(&key) {
                active_versions.insert(id, version);
            } else if skipped_keys.contains(&key) {
                preserved_active.insert(id, version);
            }
        }
        for (request_id, key) in persisted.completed_save_requests {
            // Retain only usable idempotency entries. They are reads on
            // startup and never allocate a version. Replaying one returns
            // exactly the revision it created.
            if profiles.contains_key(&key)
                && !factory_keys.contains(&key)
                && valid_operation_id(&request_id)
            {
                completed_save_requests.insert(request_id, key);
            }
        }
    }
    ProfileRuntime {
        mode: ProfileMode::Student,
        catalog_writable: true,
        catalog_load_error: None,
        factory_keys,
        profiles,
        retired,
        active_versions,
        completed_save_requests,
        skipped,
        preserved_active,
        disk_fingerprint,
        recovered_from,
        kept_unreadable_files: Vec::new(),
    }
}

/// Leftover files that may hold a complete catalog after an interrupted
/// write: the legacy `.backup` first (the last committed state), then
/// temporary files, newest first.
fn recovery_candidates(root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let legacy_backup = root.join(LEGACY_CATALOG_BACKUP);
    if legacy_backup.is_file() {
        candidates.push(legacy_backup);
    }
    let mut temporaries: Vec<(std::time::SystemTime, PathBuf)> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| is_catalog_temporary(&entry.file_name().to_string_lossy()))
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .collect();
    temporaries.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    candidates.extend(temporaries.into_iter().map(|(_, path)| path));
    candidates
}

fn is_catalog_temporary(name: &str) -> bool {
    name.starts_with("lab_state.json.") && name.ends_with(".tmp")
}

/// What `load_or_recover_catalog` found.
#[derive(Default)]
struct LoadedCatalog {
    snapshot: Option<CatalogSnapshot>,
    /// The leftover file installed as the catalog, if any.
    recovered_from: Option<String>,
    /// Leftovers that could not be read and were renamed aside (kept).
    kept_unreadable_files: Vec<String>,
}

/// Loads `lab_state.json`. When it is missing, or was read but does not
/// parse, and a leftover copy is valid, that copy is installed as the
/// catalog (the leftover and a quarantined copy of an unreadable catalog are
/// both kept) and reported. Every other outcome that could lose a copy fails
/// closed (the caller falls back to read-only factory labs):
/// - a catalog that cannot be *read* (an I/O error, for example a file held
///   open by another program) is never replaced by an older leftover;
/// - a valid leftover that cannot be installed is left in place for the next
///   launch rather than starting an empty catalog whose first write would
///   delete it.
///
/// Leftovers that do not parse are renamed to `lab_state.corrupt-*.json`, so
/// the cleanup after a later write cannot delete the last trace of a
/// catalog that went missing.
fn load_or_recover_catalog(root: &Path) -> Result<LoadedCatalog, ProfileError> {
    let main = root.join(CATALOG_FILE);
    let main_error = match read_catalog_file(&main) {
        Ok(Some(snapshot)) => {
            return Ok(LoadedCatalog {
                snapshot: Some(snapshot),
                ..LoadedCatalog::default()
            })
        }
        Ok(None) => None,
        Err(error @ ProfileError::Parse(_)) => Some(error),
        Err(error) => {
            crate::app_log::record(
                "WARN",
                &format!(
                    "LAB_CATALOG_RECOVERY_SKIPPED reason=main_not_readable path={} detail={error}",
                    main.display()
                ),
            );
            return Err(error);
        }
    };
    let mut unreadable = Vec::new();
    for candidate in recovery_candidates(root) {
        let bytes = match read_catalog_bytes(&candidate) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => {
                // It may be the only valid copy: never start a catalog that
                // a later write would clean this file out of.
                crate::app_log::record(
                    "WARN",
                    &format!(
                        "LAB_CATALOG_RECOVERY_SKIPPED reason=leftover_not_readable path={} detail={source}",
                        candidate.display()
                    ),
                );
                return Err(main_error.unwrap_or_else(|| ProfileError::Read {
                    path: candidate.display().to_string(),
                    source,
                }));
            }
        };
        let Ok(snapshot) = parse_catalog_bytes(&bytes) else {
            unreadable.push(candidate);
            continue;
        };
        if main_error.is_some() {
            let quarantine = root.join(format!(
                "lab_state.corrupt-{}.json",
                Utc::now().format("%Y%m%dT%H%M%S%.3fZ")
            ));
            if let Err(error) = copy_durably(&main, &quarantine) {
                crate::app_log::record(
                    "WARN",
                    &format!(
                        "LAB_CATALOG_RECOVERY_SKIPPED reason=quarantine_failed path={} detail={error}",
                        quarantine.display()
                    ),
                );
                return Err(main_error.unwrap_or_else(|| ProfileError::Write {
                    path: quarantine.display().to_string(),
                    source: error,
                }));
            }
        }
        if let Err(error) = write_catalog_atomically(root, &bytes) {
            crate::app_log::record(
                "WARN",
                &format!(
                    "LAB_CATALOG_RECOVERY_SKIPPED reason=write_failed from={} detail={error}",
                    candidate.display()
                ),
            );
            return Err(error);
        }
        crate::app_log::record(
            "WARN",
            &format!(
                "LAB_CATALOG_RECOVERED from={} reason={}",
                candidate.display(),
                if main_error.is_some() {
                    "unreadable"
                } else {
                    "missing"
                }
            ),
        );
        return Ok(LoadedCatalog {
            snapshot: Some(snapshot),
            recovered_from: Some(candidate.display().to_string()),
            kept_unreadable_files: Vec::new(),
        });
    }
    let kept_unreadable_files = keep_catalog_leftovers(root, &unreadable, "corrupt")?
        .into_iter()
        .map(|path| path.display().to_string())
        .collect();
    match main_error {
        Some(error) => Err(error),
        None => Ok(LoadedCatalog {
            kept_unreadable_files,
            ..LoadedCatalog::default()
        }),
    }
}

/// Renames catalog leftovers (the legacy `.backup` and temporary files) to
/// `lab_state.<label>-<time>-<n>.json`, so the cleanup after the next
/// successful write cannot delete them. A rename failure stops the caller,
/// which then fails closed.
fn keep_catalog_leftovers(
    root: &Path,
    leftovers: &[PathBuf],
    label: &str,
) -> Result<Vec<PathBuf>, ProfileError> {
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let mut kept = Vec::new();
    for (index, leftover) in leftovers.iter().enumerate() {
        let destination = root.join(format!("lab_state.{label}-{stamp}-{}.json", index + 1));
        fs::rename(leftover, &destination).map_err(|source| {
            crate::app_log::record(
                "WARN",
                &format!(
                    "LAB_CATALOG_LEFTOVER_KEEP_FAILED from={} detail={source}",
                    leftover.display()
                ),
            );
            ProfileError::Write {
                path: destination.display().to_string(),
                source,
            }
        })?;
        crate::app_log::record(
            "WARN",
            &format!(
                "LAB_CATALOG_LEFTOVER_KEPT from={} to={}",
                leftover.display(),
                destination.display()
            ),
        );
        kept.push(destination);
    }
    Ok(kept)
}

/// Writes the complete catalog to a uniquely named temporary file, flushes
/// it to disk, then atomically replaces `lab_state.json`. At every moment a
/// complete catalog file exists.
fn write_catalog_atomically(root: &Path, bytes: &[u8]) -> Result<(), ProfileError> {
    use std::io::Write;
    fs::create_dir_all(root).map_err(|source| ProfileError::Write {
        path: root.display().to_string(),
        source,
    })?;
    let target = root.join(CATALOG_FILE);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let temporary = root.join(format!(
        "{CATALOG_FILE}.{}-{nanos}-{}.tmp",
        std::process::id(),
        CATALOG_TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_file(&temporary, &target)
    })();
    if let Err(source) = result {
        let _ = fs::remove_file(&temporary);
        return Err(ProfileError::Write {
            path: target.display().to_string(),
            source,
        });
    }
    Ok(())
}

/// Atomically replaces `destination` with `source`, retrying briefly while
/// another program (for example antivirus) has the file open.
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    let mut attempt = 0u64;
    loop {
        match replace_file_once(source, destination) {
            Ok(()) => return Ok(()),
            // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
            Err(error) if attempt < 4 && matches!(error.raw_os_error(), Some(5 | 32 | 33)) => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(50 * attempt));
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(windows)]
fn replace_file_once(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let (from, to) = (wide(source), wide(destination));
    // SAFETY: both arguments are NUL-terminated UTF-16 paths that outlive the call.
    let moved = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file_once(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

/// Copies a file and flushes the copy to disk.
fn copy_durably(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::copy(source, destination)?;
    fs::OpenOptions::new()
        .write(true)
        .open(destination)?
        .sync_all()
}

/// Removes leftovers of interrupted writes. Called only right after a
/// successful replace, so `lab_state.json` is a complete catalog and none of
/// these is the only valid copy.
fn remove_catalog_leftovers(root: &Path) {
    let mut leftovers = vec![root.join(LEGACY_CATALOG_BACKUP)];
    leftovers.extend(
        fs::read_dir(root)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| is_catalog_temporary(&entry.file_name().to_string_lossy()))
            .map(|entry| entry.path()),
    );
    for path in leftovers {
        if path.is_file() {
            if let Err(error) = fs::remove_file(&path) {
                crate::app_log::record(
                    "WARN",
                    &format!(
                        "LAB_CATALOG_LEFTOVER_CLEANUP_FAILED path={} detail={error}",
                        path.display()
                    ),
                );
            }
        }
    }
}

/// Makes an editor draft use the explicit output list (an empty list is
/// authoritative) and the pulse-ox shape the editor shows: no analog
/// channels, and the cycle rate that follows from the dwell.
fn normalize_authoring_draft(draft: &mut AcquisitionProfile) {
    drop_inert_pulse_ox_channels(draft);
    let acquisition = &mut draft.acquisition;
    acquisition.digital_outputs = acquisition.resolved_digital_outputs();
    acquisition.led_outputs = None;
    if acquisition.acquisition_mode == AcquisitionMode::Pulseox4State {
        if let Some(dwell) = acquisition
            .state_dwell_us
            .filter(|dwell| PULSEOX_DWELL_RANGE_US.contains(dwell))
        {
            acquisition.sample_rate_hz = pulse_ox_cycle_rate_hz(dwell);
        }
    }
}

/// Pulse-ox acquisition never reads `channels`: its pins, record fields, and
/// CSV columns come from the fixed TX/RX phase fields. The v1.0.2 lab editor
/// nevertheless stored an unused "Channel 1 / A0" in every pulse-ox lab it
/// saved, so new revisions and imported packages drop that list instead of
/// refusing a package this application produced. Stored labs are not
/// rewritten (their canonical hashes stay as they are).
fn drop_inert_pulse_ox_channels(profile: &mut AcquisitionProfile) {
    if profile.acquisition.acquisition_mode == AcquisitionMode::Pulseox4State {
        profile.acquisition.channels.clear();
    }
}

/// Two revisions hold the same lab when they differ at most in `source`, the
/// stored hash (an exported instructor lab is installed as `imported`), and
/// the inert pulse-ox channel list that import drops.
fn same_lab_content(
    left: &AcquisitionProfile,
    right: &AcquisitionProfile,
) -> Result<bool, ProfileError> {
    let normalized = |profile: &AcquisitionProfile| {
        let mut copy = profile.clone();
        copy.source = ProfileSource::Imported;
        copy.integrity.canonical_hash_algorithm = "SHA-256".into();
        drop_inert_pulse_ox_channels(&mut copy);
        copy.canonical_bytes()
    };
    Ok(normalized(left)? == normalized(right)?)
}

/// The next revision of a lab: one patch above the highest existing patch
/// with the same ID and MAJOR.MINOR as the edit base, counting factory,
/// retired, and skipped revisions, so a restored default or an older active
/// revision can always be edited.
fn next_revision_version(
    runtime: &ProfileRuntime,
    profile_id: &str,
    base: &str,
) -> Result<String, ProfileError> {
    if !valid_semver(base) {
        return Err(ProfileError::Validation(
            "lab version must be MAJOR.MINOR.PATCH".into(),
        ));
    }
    let (major, minor, base_patch) = version_key(base);
    let highest = runtime
        .profiles
        .keys()
        .cloned()
        .chain(runtime.retired.iter().cloned())
        .chain(runtime.skipped.iter().filter_map(SkippedEntry::key))
        .filter(|(id, version)| id == profile_id && valid_semver(version))
        .map(|(_, version)| version_key(&version))
        .filter(|(existing_major, existing_minor, _)| {
            *existing_major == major && *existing_minor == minor
        })
        .map(|(_, _, patch)| patch)
        .fold(base_patch, u64::max);
    let patch = highest.checked_add(1).ok_or_else(|| {
        ProfileError::Validation("no further revision number is available for this lab".into())
    })?;
    Ok(format!("{major}.{minor}.{patch}"))
}

impl Default for ProfileStore {
    fn default() -> Self {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("WVU Bioinstrumentation Studio")
            .join("profiles");
        match Self::with_root(root.clone()) {
            Ok(store) => store,
            Err(error) => {
                crate::app_log::record(
                    "WARN",
                    &format!(
                        "LAB_CATALOG_UNAVAILABLE path={} detail={error}",
                        root.join(CATALOG_FILE).display()
                    ),
                );
                Self::factory_only(root, error.to_string())
            }
        }
    }
}
impl ProfileStore {
    /// Last-resort read-only fallback for an unavailable local catalog. The
    /// class defaults remain available even if a user-data path is malformed
    /// or temporarily inaccessible. Crucially, this retains the intended
    /// catalog location and rejects instructor writes rather than silently
    /// redirecting them into a temporary directory.
    fn factory_only(root: PathBuf, load_error: String) -> Self {
        let mut runtime =
            runtime_with_catalog(&built_in_profiles().unwrap_or_default(), None, None);
        runtime.catalog_writable = false;
        runtime.catalog_load_error = Some(load_error);
        Self {
            root,
            runtime: Arc::new(Mutex::new(runtime)),
        }
    }
    pub fn with_root(root: PathBuf) -> Result<Self, ProfileError> {
        let factory = built_in_profiles()?;
        // Loading (and any recovery write) must not overlap another window's
        // catalog write. Nothing is created when the folder does not exist.
        let _catalog_lock = if root.is_dir() {
            match CatalogLock::acquire(&root) {
                Ok(lock) => Some(lock),
                Err(error) => {
                    crate::app_log::record(
                        "WARN",
                        &format!("LAB_CATALOG_LOCK_UNAVAILABLE phase=load detail={error}"),
                    );
                    None
                }
            }
        } else {
            None
        };
        let loaded = load_or_recover_catalog(&root)?;
        let mut runtime = runtime_with_catalog(&factory, loaded.snapshot, loaded.recovered_from);
        runtime.kept_unreadable_files = loaded.kept_unreadable_files;
        Ok(Self {
            root,
            runtime: Arc::new(Mutex::new(runtime)),
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
    /// Catalog health for the Lab panel notice. It never reads or writes the
    /// catalog file and is available in Student mode.
    pub fn catalog_status(&self) -> Result<CatalogStatus, ProfileError> {
        let runtime = self.lock()?;
        Ok(CatalogStatus {
            writable: runtime.catalog_writable,
            load_error: runtime.catalog_load_error.clone(),
            skipped_labs: runtime
                .skipped
                .iter()
                .map(|entry| SkippedLab {
                    profile_id: entry.profile_id.clone(),
                    profile_version: entry.profile_version.clone(),
                    display_name: entry.display_name.clone(),
                    reason: entry.reason.clone(),
                })
                .collect(),
            recovered_from: runtime.recovered_from.clone(),
            kept_unreadable_files: runtime.kept_unreadable_files.clone(),
        })
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
    /// Only `save_lab_draft` creates a local revision. Legacy `led_outputs`
    /// are converted to the explicit `digital_outputs` list here, so the
    /// editor (and the saved revision) shows exactly what will be driven.
    pub fn begin_lab_edit(&self, profile_id: &str) -> Result<AcquisitionProfile, ProfileError> {
        self.require_instructor()?;
        let current = self.get_locked(profile_id)?;
        let mut draft = current;
        draft.status = ProfileStatus::Draft;
        draft.source = ProfileSource::Instructor;
        draft.integrity.canonical_hash.clear();
        normalize_authoring_draft(&mut draft);
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
        if !valid_new_lab_id(lab_id) {
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
        normalize_authoring_draft(&mut draft);
        let runtime = self.lock()?;
        if runtime.lab_id_in_use(lab_id) {
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
        if !valid_new_lab_id(lab_id) {
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
        normalize_authoring_draft(&mut draft);
        let runtime = self.lock()?;
        if runtime.lab_id_in_use(lab_id) {
            return Err(ProfileError::Validation(
                "lab ID/version already exists".into(),
            ));
        }
        Ok(draft)
    }

    /// Atomically creates exactly one active local revision for a deliberate
    /// instructor Save. A request ID makes retried UI calls idempotent, and a
    /// base-version check rejects a stale editor rather than guessing a new
    /// version. The new version is one patch above the highest revision that
    /// already exists for the lab (see `next_revision_version`).
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
        let (_catalog_lock, mut runtime) = self.begin_write()?;
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
                next_revision_version(&next, &requested_id, &base)?
            }
            None => {
                if !valid_new_lab_id(&requested_id) {
                    return Err(ProfileError::Validation(
                        "lab ID must use lowercase letters, numbers, dots, dashes, or underscores"
                            .into(),
                    ));
                }
                if next.lab_id_in_use(&requested_id) {
                    return Err(ProfileError::Validation(
                        "lab ID already exists; use Edit to create a new revision".into(),
                    ));
                }
                "1.0.0".into()
            }
        };
        let key = (requested_id.clone(), allocated_version.clone());
        if next.revision_in_use(&key) {
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
        // The explicit output list is authoritative for every new revision:
        // an empty list means no outputs, never the legacy shorthand.
        finalized.acquisition.led_outputs = None;
        // The pulse-ox editor shows no channels; never store a phantom one.
        drop_inert_pulse_ox_channels(&mut finalized);
        finalized.validate_for_authoring()?;
        finalized.status = ProfileStatus::Locked;
        finalized.refresh_hash()?;
        finalized.validate()?;
        next.profiles.insert(pkey(&finalized), finalized.clone());
        next.active_versions.insert(
            finalized.profile_id.clone(),
            finalized.profile_version.clone(),
        );
        next.preserved_active.remove(&finalized.profile_id);
        next.retired.remove(&pkey(&finalized));
        next.completed_save_requests.insert(
            request_id.clone(),
            (
                finalized.profile_id.clone(),
                finalized.profile_version.clone(),
            ),
        );
        self.commit(&mut next)?;
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
    /// Retires one local revision. Retiring the active revision of a lab that
    /// has a shipped course default makes that default active again (the same
    /// result a restart would give); otherwise the lab has no active revision.
    pub fn retire(&self, profile_id: &str, profile_version: &str) -> Result<(), ProfileError> {
        self.require_instructor()?;
        self.require_catalog_writable()?;
        let (_catalog_lock, mut runtime) = self.begin_write()?;
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
            match next.factory_version(profile_id) {
                Some(factory_version) => {
                    next.active_versions
                        .insert(profile_id.to_string(), factory_version);
                }
                None => {
                    next.active_versions.remove(profile_id);
                }
            }
        }
        self.commit(&mut next)?;
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
        let (_catalog_lock, mut runtime) = self.begin_write()?;
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
        next.preserved_active.remove(&profile.profile_id);
        self.commit(&mut next)?;
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
        let (_catalog_lock, mut runtime) = self.begin_write()?;
        if runtime.active_versions.get(profile_id) == Some(&restored.profile_version)
            && !runtime.preserved_active.contains_key(profile_id)
        {
            return Ok(restored);
        }
        let mut next = runtime.clone();
        next.active_versions.insert(
            restored.profile_id.clone(),
            restored.profile_version.clone(),
        );
        next.preserved_active.remove(profile_id);
        self.commit(&mut next)?;
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
        if self.lock()?.factory_keys.contains(&key) {
            return self.restore_course_default(profile_id);
        }
        let (_catalog_lock, mut runtime) = self.begin_write()?;
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
        next.preserved_active.remove(profile_id);
        self.commit(&mut next)?;
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
    /// intended for a classroom catalog reset, never for normal startup. The
    /// catalog file is always kept first: an unreadable one is moved to
    /// `lab_state.corrupt-<time>.json`, a readable one is copied to
    /// `lab_state.reset-<time>.json`, and that path is returned.
    pub fn reset_local_customizations(&self) -> Result<LabResetOutcome, ProfileError> {
        self.require_instructor()?;
        let _catalog_lock = CatalogLock::acquire(&self.root)?;
        let mut runtime = self.lock()?;
        // Count (and log) what is really removed, including revisions another
        // window saved since this one loaded the catalog.
        let mut catalog_readable = runtime.catalog_writable;
        if catalog_readable {
            if let Err(error) = self.sync_with_disk(&mut runtime) {
                crate::app_log::record(
                    "WARN",
                    &format!("LAB_CATALOG_RESET_UNREADABLE detail={error}"),
                );
                catalog_readable = false;
            }
        }
        // An unavailable catalog may have failed to load because a valid
        // leftover could not be installed; keep every leftover, since the
        // write below cleans them out of the folder.
        let kept_leftovers = if catalog_readable {
            Vec::new()
        } else {
            keep_catalog_leftovers(
                &self.root,
                &recovery_candidates(&self.root),
                "reset-leftover",
            )?
        };
        let mut next = runtime.clone();
        let factory_keys = next.factory_keys.clone();
        let removed_local_revisions = next
            .profiles
            .keys()
            .filter(|key| !factory_keys.contains(*key))
            .count()
            + next.skipped.len();
        next.profiles.retain(|key, _| factory_keys.contains(key));
        next.retired.clear();
        next.active_versions = next
            .profiles
            .values()
            .map(|profile| (profile.profile_id.clone(), profile.profile_version.clone()))
            .collect();
        next.completed_save_requests.clear();
        next.skipped.clear();
        next.preserved_active.clear();
        next.recovered_from = None;
        next.kept_unreadable_files.clear();
        next.catalog_writable = true;
        next.catalog_load_error = None;
        let state_path = self.root.join(CATALOG_FILE);
        let stamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
        let mut quarantined = None;
        let mut backup = None;
        if state_path.exists() {
            if !catalog_readable {
                let path = self.root.join(format!("lab_state.corrupt-{stamp}.json"));
                fs::rename(&state_path, &path).map_err(|source| ProfileError::Write {
                    path: path.display().to_string(),
                    source,
                })?;
                quarantined = Some(path.clone());
                backup = Some(path);
            } else {
                let path = self.root.join(format!("lab_state.reset-{stamp}.json"));
                copy_durably(&state_path, &path).map_err(|source| ProfileError::Write {
                    path: path.display().to_string(),
                    source,
                })?;
                backup = Some(path);
            }
        }
        if let Err(error) = self.commit(&mut next) {
            if let Some(path) = &quarantined {
                let _ = fs::rename(path, &state_path);
            }
            return Err(error);
        }
        *runtime = next;
        drop(runtime);
        let backup_path = backup.map(|path| path.display().to_string());
        self.append_lab_write_log_best_effort(
            "reset_local_customizations",
            "all",
            None,
            None,
            "explicit",
        );
        crate::app_log::record(
            "INFO",
            &format!(
                "LAB_CATALOG_RESET backup={} removed_local_revisions={removed_local_revisions} kept_leftovers={}",
                backup_path.as_deref().unwrap_or("-"),
                kept_leftovers.len()
            ),
        );
        Ok(LabResetOutcome {
            backup_path,
            removed_local_revisions,
        })
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
        // The package's own hash was verified above. A pulse-ox package saved
        // by the v1.0.2 editor carries an unused channel; the installed copy
        // is re-hashed below anyway (its source becomes `imported`).
        drop_inert_pulse_ox_channels(&mut profile);
        let (_catalog_lock, mut runtime) = self.begin_write()?;
        let key = pkey(&profile);
        if let Some(existing) = runtime.profiles.get(&key) {
            // An installed lab is stored as `imported` (or is a factory lab),
            // while its exported file may say `instructor`; compare content.
            if same_lab_content(existing, &profile)? {
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
        if runtime.revision_in_use(&key) {
            return Err(ProfileError::Validation(
                "a stored lab with this ID and version could not be loaded; repair or reset the local catalog before importing it"
                    .into(),
            ));
        }
        profile.source = ProfileSource::Imported;
        profile.integrity.canonical_hash.clear();
        profile.refresh_hash()?;
        profile.validate_for_authoring()?;
        let mut next = runtime.clone();
        next.profiles.insert(key, profile.clone());
        next.active_versions
            .insert(profile.profile_id.clone(), profile.profile_version.clone());
        next.preserved_active.remove(&profile.profile_id);
        self.commit(&mut next)?;
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
    /// Starts a catalog read-modify-write: takes the cross-process lock, then
    /// continues from what is on disk if another window changed the file.
    fn begin_write(
        &self,
    ) -> Result<(CatalogLock, std::sync::MutexGuard<'_, ProfileRuntime>), ProfileError> {
        let catalog_lock = CatalogLock::acquire(&self.root)?;
        let mut runtime = self.lock()?;
        if runtime.catalog_writable {
            self.sync_with_disk(&mut runtime)?;
        }
        Ok((catalog_lock, runtime))
    }
    /// Reloads the stored part of the catalog when the file differs from what
    /// this process last read or wrote, so another window's revisions are
    /// never overwritten by a stale in-memory copy.
    fn sync_with_disk(&self, runtime: &mut ProfileRuntime) -> Result<(), ProfileError> {
        let path = self.root.join(CATALOG_FILE);
        let current = match read_catalog_bytes(&path) {
            Ok(bytes) => Some(bytes_fingerprint(&bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(ProfileError::Read {
                    path: path.display().to_string(),
                    source,
                })
            }
        };
        if current == runtime.disk_fingerprint {
            return Ok(());
        }
        let snapshot = read_catalog_file(&path).map_err(|error| {
            ProfileError::CatalogUnavailable(format!(
                "the lab catalog was changed outside this window and could not be read; restart the application ({error})"
            ))
        })?;
        let mut reloaded = runtime_with_catalog(
            &runtime.factory_profiles(),
            snapshot,
            runtime.recovered_from.clone(),
        );
        reloaded.mode = runtime.mode.clone();
        reloaded.kept_unreadable_files = runtime.kept_unreadable_files.clone();
        crate::app_log::record(
            "INFO",
            "LAB_CATALOG_RELOADED reason=changed_outside_this_window",
        );
        *runtime = reloaded;
        Ok(())
    }
    fn persisted_state<'a>(&self, runtime: &'a ProfileRuntime) -> PersistedLabState<'a> {
        let mut active_versions: BTreeMap<String, String> = runtime
            .active_versions
            .iter()
            .filter(|(id, version)| {
                !runtime
                    .factory_keys
                    .contains(&((*id).clone(), (*version).clone()))
            })
            .map(|(id, version)| (id.clone(), version.clone()))
            .collect();
        for (id, version) in &runtime.preserved_active {
            active_versions
                .entry(id.clone())
                .or_insert_with(|| version.clone());
        }
        PersistedLabState {
            profiles: runtime
                .profiles
                .values()
                .filter(|profile| profile.source != ProfileSource::BuiltIn)
                .map(PersistedProfile::Loaded)
                .chain(
                    runtime
                        .skipped
                        .iter()
                        .map(|entry| PersistedProfile::Skipped(entry.raw.as_ref())),
                )
                .collect(),
            retired: runtime
                .retired
                .iter()
                .filter(|key| !runtime.factory_keys.contains(*key))
                .cloned()
                .collect(),
            active_versions,
            completed_save_requests: runtime.completed_save_requests.clone(),
        }
    }
    /// Writes `next` to disk. The caller replaces its runtime with `next`
    /// only after this succeeds (transactional catalog writes).
    fn commit(&self, next: &mut ProfileRuntime) -> Result<(), ProfileError> {
        let bytes = serde_json::to_vec_pretty(&self.persisted_state(next))?;
        write_catalog_atomically(&self.root, &bytes)?;
        next.disk_fingerprint = Some(bytes_fingerprint(&bytes));
        remove_catalog_leftovers(&self.root);
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
/// A lab ID chosen now (Duplicate, Blank, or a new-lab Save) is also bounded
/// in length; stored IDs are never rejected for length.
fn valid_new_lab_id(value: &str) -> bool {
    valid_profile_id(value) && value.len() <= MAX_PROFILE_ID_CHARS
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
        // Each rule is checked on a correctly rehashed profile, so the
        // assertion cannot pass merely because the hash no longer matches.
        let factory_pulseox =
            built_in_profiles().unwrap_or_else(|error| panic!("{error}"))[4].clone();
        let mut out_of_range = factory_pulseox.clone();
        out_of_range.acquisition.state_dwell_us = Some(200);
        out_of_range
            .refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(out_of_range.verify_integrity().is_ok());
        assert!(out_of_range.validate().is_err());

        // 900 µs is a supported dwell, but the stored 250 cycles/s no longer
        // follows from it: a stored lab still loads, a new revision is refused.
        let mut mismatched = factory_pulseox.clone();
        mismatched.acquisition.state_dwell_us = Some(900);
        mismatched
            .refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(mismatched.validate().is_ok());
        assert!(mismatched.validate_for_authoring().is_err());
        mismatched.acquisition.sample_rate_hz = pulse_ox_cycle_rate_hz(900);
        assert_eq!(mismatched.acquisition.sample_rate_hz, 278);
        mismatched
            .refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(mismatched.validate_for_authoring().is_ok());

        // Simultaneous rates: only the firmware's rates for a new revision.
        let mut ecg = built_in_profiles().unwrap_or_else(|error| panic!("{error}"))[1].clone();
        ecg.acquisition.sample_rate_hz = 300;
        ecg.refresh_hash().unwrap_or_else(|error| panic!("{error}"));
        assert!(ecg.validate().is_ok());
        assert!(ecg.validate_for_authoring().is_err());
        assert!(ecg.acquisition.firmware_configuration_issue().is_some());
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

    const ECG_ID: &str = "wvu.bmeg420l.ecg.course.capture.v1";
    const EMG_ID: &str = "wvu.bmeg420l.emg.force.course.capture.v1";
    const PULSE_ID: &str = "wvu.bmeg420l.pulseox.txrx.raw.course.capture.v1";
    const GENERAL_ID: &str = "wvu.bmeg420l.general.analog.development.v2";
    const OP_AMP_IMPORTED_HASH: &str =
        "c73cc1f2941910696d88f934b7b4c09b8b110438a4886d7d799b13807f61471d";
    const OP_AMP_INSTRUCTOR_HASH: &str =
        "bcc08868305ca55e81920e3a5805b020cfa612a0374e75fd5a290fa11f918ad9";
    const OP_AMP_INSTALLER: &str = include_str!("../../scripts/install_op_amp_lab_profile.ps1");

    fn instructor_store(root: &Path) -> ProfileStore {
        let store =
            ProfileStore::with_root(root.to_path_buf()).unwrap_or_else(|error| panic!("{error}"));
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        store
    }

    /// The profile JSON embedded in the student installer, between
    /// `$profileJson = @'` and `'@`.
    fn op_amp_installer_json() -> String {
        let script = OP_AMP_INSTALLER.replace("\r\n", "\n");
        let marker = "$profileJson = @'\n";
        let start = script
            .find(marker)
            .unwrap_or_else(|| panic!("the installer no longer embeds its profile"))
            + marker.len();
        let end = script[start..]
            .find("\n'@")
            .unwrap_or_else(|| panic!("the embedded profile is not terminated"))
            + start;
        script[start..end].to_string()
    }

    fn write_catalog(root: &Path, profiles: &[String], active: &[(&str, &str)]) {
        fs::create_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
        let active = active
            .iter()
            .map(|(id, version)| format!("\"{id}\": \"{version}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let text = format!(
            "{{\n  \"profiles\": [\n{}\n  ],\n  \"retired\": [],\n  \"active_versions\": {{ {active} }},\n  \"completed_save_requests\": {{}}\n}}",
            profiles.join(",\n")
        );
        fs::write(root.join(CATALOG_FILE), text).unwrap_or_else(|error| panic!("{error}"));
    }

    fn file_names(root: &Path) -> Vec<String> {
        fs::read_dir(root)
            .unwrap_or_else(|error| panic!("{error}"))
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn factory_profiles_keep_their_published_canonical_hashes() {
        let expected = [
            (
                GENERAL_ID,
                "2cfca936f3af638774a6a4239ff9a740ba5272b765cc22c33118498050c2b790",
            ),
            (
                ECG_ID,
                "3c59886674d1b8467003e6900987fa5e2643eb9c15306241eb4fc9603d3cad6d",
            ),
            (
                EMG_ID,
                "cd0af7cf4a1ca25d8a5988f630825a30d7015aa6a40ef282af10f225e1ae60b8",
            ),
            (
                "wvu.bmeg420l.blood_pressure.ppg.course.capture.v1",
                "fbd4a5b6f210271694694feaac54876c44c6b09f8525f2f4e84f7c88f0bb266f",
            ),
            (
                PULSE_ID,
                "019c25a69ef6e5b361774acc668915066c7113a7b58170c906a8049a69198bd0",
            ),
        ];
        let profiles = built_in_profiles().unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(profiles.len(), expected.len());
        for (profile, (id, hash)) in profiles.iter().zip(expected) {
            assert_eq!(profile.profile_id, id);
            assert_eq!(profile.integrity.canonical_hash, hash, "{id}");
            assert_eq!(
                profile
                    .computed_hash()
                    .unwrap_or_else(|error| panic!("{error}")),
                hash,
                "{id}"
            );
            assert!(profile.validate().is_ok());
            // Labs derived from a factory lab can be saved and imported.
            assert!(profile.validate_for_authoring().is_ok(), "{id}");
            // The catalog's stored form round-trips to the same canonical bytes.
            let stored = serde_json::to_vec_pretty(profile).unwrap_or_default();
            let reparsed: AcquisitionProfile =
                serde_json::from_slice(&stored).unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(
                reparsed
                    .computed_hash()
                    .unwrap_or_else(|error| panic!("{error}")),
                hash
            );
            // Labs without explicit roles never serialize the new field.
            assert!(
                !String::from_utf8(profile.canonical_bytes().unwrap_or_default())
                    .unwrap_or_default()
                    .contains("pulse_ox_role")
            );
        }
    }

    #[test]
    fn op_amp_installer_profile_keeps_its_documented_hashes() {
        let imported: AcquisitionProfile = serde_json::from_str(&op_amp_installer_json())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(imported.profile_id, GENERAL_ID);
        assert_eq!(imported.profile_version, "2.0.1");
        assert_eq!(imported.source, ProfileSource::Imported);
        assert_eq!(imported.integrity.canonical_hash, OP_AMP_IMPORTED_HASH);
        assert_eq!(
            imported
                .computed_hash()
                .unwrap_or_else(|error| panic!("{error}")),
            OP_AMP_IMPORTED_HASH
        );
        assert!(imported.validate().is_ok());
        let mut instructor = imported.clone();
        instructor.source = ProfileSource::Instructor;
        assert_eq!(
            instructor
                .computed_hash()
                .unwrap_or_else(|error| panic!("{error}")),
            OP_AMP_INSTRUCTOR_HASH
        );
        // Its "01_counts" field (a leading digit) stays valid for new revisions.
        assert!(imported.validate_for_authoring().is_ok());
        assert!(
            OP_AMP_INSTALLER.contains(&format!("$expectedImportedHash = '{OP_AMP_IMPORTED_HASH}'"))
        );
        assert!(OP_AMP_INSTALLER.contains(&format!(
            "$expectedInstructorHash = '{OP_AMP_INSTRUCTOR_HASH}'"
        )));
    }

    #[test]
    fn op_amp_lab_installed_by_the_student_script_loads_and_survives_catalog_writes() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        write_catalog(&root, &[op_amp_installer_json()], &[(GENERAL_ID, "2.0.1")]);
        let store = instructor_store(&root);
        let op_amp = store
            .get_locked(GENERAL_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(op_amp.display_name, "Op Amp Lab");
        assert_eq!(op_amp.profile_version, "2.0.1");
        assert!(store
            .catalog_status()
            .unwrap_or_else(|error| panic!("{error}"))
            .skipped_labs
            .is_empty());

        // An unrelated catalog write keeps the Op Amp entry and its hash.
        let mut draft = store
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        draft.description.push_str(" edited");
        store
            .save_lab_draft(draft, Some("1.0.0".into()), "opamp-write-0001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        let reopened =
            ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        let again = reopened
            .get_locked(GENERAL_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(again.integrity.canonical_hash, OP_AMP_IMPORTED_HASH);
        assert_eq!(
            again
                .computed_hash()
                .unwrap_or_else(|error| panic!("{error}")),
            OP_AMP_IMPORTED_HASH
        );
        // The entry is still written in struct field order, which the
        // installer's PowerShell re-hash of an installed entry relies on.
        let written = fs::read_to_string(root.join(CATALOG_FILE)).unwrap_or_default();
        let name_at = written
            .find("\"display_name\": \"Op Amp Lab\"")
            .unwrap_or_else(|| panic!("Op Amp entry missing"));
        let entry_start = written[..name_at]
            .rfind("\"schema_version\"")
            .unwrap_or_else(|| panic!("entry start missing"));
        let entry = &written[entry_start..];
        let positions: Vec<usize> = [
            "\"profile_id\"",
            "\"profile_version\"",
            "\"display_name\"",
            "\"acquisition\"",
            "\"analog_inputs\": null",
            "\"led_outputs\": null",
            "\"state_dwell_us\": null",
            "\"integrity\"",
            "\"plot_defaults\"",
        ]
        .iter()
        .map(|key| {
            entry
                .find(key)
                .unwrap_or_else(|| panic!("{key} missing from the written entry"))
        })
        .collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));

        // Editing the Op Amp Lab allocates 2.0.2 (factory 2.0.0, local 2.0.1).
        let edit = store
            .begin_lab_edit(GENERAL_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        let saved = store
            .save_lab_draft(edit, Some("2.0.1".into()), "opamp-edit-00001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(saved.profile_version, "2.0.2");
        assert_eq!(saved.acquisition.channels[1].csv_name, "01_counts");
    }

    #[test]
    fn editing_after_restoring_the_course_default_creates_the_next_unused_revision() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = instructor_store(&root);
        let mut first = store
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        first.description.push_str(" first");
        let first = store
            .save_lab_draft(first, Some("1.0.0".into()), "restore-edit-001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(first.profile_version, "1.0.1");

        store
            .restore_course_default(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        let mut second = store
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(second.profile_version, "1.0.0");
        second.description.push_str(" second");
        let second = store
            .save_lab_draft(second, Some("1.0.0".into()), "restore-edit-002".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(second.profile_version, "1.0.2");

        // Making an older revision active and editing it also works.
        store
            .set_active_version(ECG_ID, "1.0.1")
            .unwrap_or_else(|error| panic!("{error}"));
        let third = store
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        let third = store
            .save_lab_draft(third, Some("1.0.1".into()), "restore-edit-003".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(third.profile_version, "1.0.3");

        // A retired revision still counts, and the audit log keeps the real base.
        store
            .retire(ECG_ID, "1.0.3")
            .unwrap_or_else(|error| panic!("{error}"));
        let fourth = store
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        let base = fourth.profile_version.clone();
        let fourth = store
            .save_lab_draft(fourth, Some(base.clone()), "restore-edit-004".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(fourth.profile_version, "1.0.4");
        let audit =
            fs::read_to_string(root.join("lab_write_audit.log")).unwrap_or_else(|e| panic!("{e}"));
        assert!(audit.contains("\t1.0.0\t1.0.2\t"));
        assert!(audit.contains(&format!("\t{base}\t1.0.4\t")));

        // "Use factory default" (set_active_version on the factory key)
        // delegates to Restore course default without deadlocking.
        let factory = store
            .set_active_version(ECG_ID, "1.0.0")
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(factory.source, ProfileSource::BuiltIn);
        assert_eq!(
            store
                .get_locked(ECG_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "1.0.0"
        );
    }

    #[test]
    fn authoring_drafts_use_explicit_outputs_and_an_empty_list_is_authoritative() {
        let mut legacy = built_in_profiles().unwrap_or_else(|error| panic!("{error}"))[4].clone();
        legacy.acquisition.digital_outputs.clear();
        normalize_authoring_draft(&mut legacy);
        assert!(legacy.acquisition.led_outputs.is_none());
        assert_eq!(
            legacy
                .acquisition
                .digital_outputs
                .iter()
                .map(|output| (output.pin.as_str(), output.label.as_str()))
                .collect::<Vec<_>>(),
            [("D5", "Red LED"), ("D6", "IR LED")]
        );

        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let store = instructor_store(&dir.path().join("profiles"));
        let bp_id = "wvu.bmeg420l.blood_pressure.ppg.course.capture.v1";
        let mut bp = store
            .duplicate_lab(bp_id, "team.bp")
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(bp.acquisition.led_outputs.is_none());
        assert_eq!(bp.acquisition.digital_outputs.len(), 1);
        bp.acquisition.digital_outputs.clear();
        // Even a draft that still carries the legacy shorthand is saved
        // with no outputs when its explicit list is empty.
        bp.acquisition.led_outputs = Some(LedOutputs {
            green: Some("D4".into()),
            red: None,
            ir: None,
        });
        let saved = store
            .save_lab_draft(bp, None, "team-bp-save-01".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(saved.acquisition.led_outputs.is_none());
        assert!(saved.acquisition.resolved_digital_outputs().is_empty());
    }

    #[test]
    fn pulse_ox_drafts_have_no_phantom_channel_and_keep_the_rate_tied_to_dwell() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let store = instructor_store(&dir.path().join("profiles"));
        let mut draft = store
            .begin_lab_edit(PULSE_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(draft.acquisition.channels.is_empty());
        assert!(draft.acquisition.led_outputs.is_none());
        assert_eq!(draft.acquisition.digital_outputs.len(), 2);
        // An editor that still sends the old phantom channel cannot store it.
        draft.acquisition.channels.push(ProfileChannel {
            pin: "A0".into(),
            id: "channel_1".into(),
            label: "Channel 1".into(),
            csv_name: "channel_1_counts".into(),
            units: "ADC counts".into(),
            allowed_conversions: vec!["counts_volts".into()],
            default_display_unit: Some("counts".into()),
            default_visible: true,
        });
        draft.acquisition.state_dwell_us = Some(2_000);
        draft.acquisition.sample_rate_hz = 125;
        let saved = store
            .save_lab_draft(draft, Some("1.0.0".into()), "pulse-save-00001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(saved.acquisition.channels.is_empty());
        assert_eq!(saved.acquisition.sample_rate_hz, 125);

        let mut wrong_rate = store
            .begin_lab_edit(PULSE_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        wrong_rate.acquisition.sample_rate_hz = 1_000;
        let error = store
            .save_lab_draft(wrong_rate, Some("1.0.1".into()), "pulse-save-00002".into())
            .err()
            .unwrap_or_else(|| panic!("a rate that does not follow the dwell must be refused"));
        assert!(error.to_string().contains("cycle rate"), "{error}");

        let mut extra_output = store
            .begin_lab_edit(PULSE_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        extra_output
            .acquisition
            .digital_outputs
            .push(DigitalOutput {
                pin: "D4".into(),
                label: "Green LED".into(),
                behavior: DigitalOutputBehavior::HighWhileRecording,
                pulse_ox_role: None,
            });
        let error = store
            .save_lab_draft(
                extra_output,
                Some("1.0.1".into()),
                "pulse-save-00003".into(),
            )
            .err()
            .unwrap_or_else(|| panic!("an extra pulse-ox output must be refused"));
        assert!(
            error.to_string().contains("only their RED and IR"),
            "{error}"
        );
    }

    #[test]
    fn pulse_ox_roles_come_from_an_explicit_field_with_the_label_rule_as_fallback() {
        let mut pulse = built_in_profiles().unwrap_or_else(|error| panic!("{error}"))[4].clone();
        // The label rule ignores ASCII case.
        pulse.acquisition.digital_outputs[0].label = "RED LED".into();
        assert_eq!(
            pulse
                .acquisition
                .pulse_ox_output(PulseOxRole::Red)
                .map(|output| output.pin),
            Some("D5".into())
        );
        // Explicit roles allow any label and win over a matching label.
        pulse.acquisition.digital_outputs = vec![
            DigitalOutput {
                pin: "D6".into(),
                label: "Emitter A".into(),
                behavior: DigitalOutputBehavior::AcquisitionSequenced,
                pulse_ox_role: Some(PulseOxRole::Red),
            },
            DigitalOutput {
                pin: "D5".into(),
                label: "Red LED".into(),
                behavior: DigitalOutputBehavior::AcquisitionSequenced,
                pulse_ox_role: Some(PulseOxRole::Ir),
            },
        ];
        assert_eq!(
            pulse
                .acquisition
                .pulse_ox_output(PulseOxRole::Red)
                .map(|output| output.pin),
            Some("D6".into())
        );
        assert_eq!(
            pulse
                .acquisition
                .pulse_ox_output(PulseOxRole::Ir)
                .map(|output| output.pin),
            Some("D5".into())
        );
        pulse
            .refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(pulse.validate_for_authoring().is_ok());
        let json = serde_json::to_string(&pulse).unwrap_or_default();
        assert!(json.contains("\"pulse_ox_role\":\"red\""));
        let reparsed: AcquisitionProfile =
            serde_json::from_str(&json).unwrap_or_else(|error| panic!("{error}"));
        assert!(reparsed.verify_integrity().is_ok());

        // Two outputs claiming RED are refused for a new revision.
        pulse.acquisition.digital_outputs.push(DigitalOutput {
            pin: "D4".into(),
            label: "Emitter C".into(),
            behavior: DigitalOutputBehavior::AcquisitionSequenced,
            pulse_ox_role: Some(PulseOxRole::Red),
        });
        pulse
            .refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(pulse.validate_for_authoring().is_err());

        // A simultaneous lab may not carry a pulse-ox role.
        let mut bp = built_in_profiles().unwrap_or_else(|error| panic!("{error}"))[3].clone();
        bp.acquisition.digital_outputs[0].pulse_ox_role = Some(PulseOxRole::Red);
        bp.refresh_hash().unwrap_or_else(|error| panic!("{error}"));
        assert!(bp.validate_for_authoring().is_err());
    }

    #[test]
    fn new_revisions_need_machine_safe_distinct_csv_columns_but_stored_labs_still_load() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = instructor_store(&root);
        let long = "a".repeat(MAX_LAB_IDENTIFIER_CHARS + 1);
        for (index, bad) in [
            "Vout, filtered",
            "x\"y",
            "a b",
            "line\nbreak",
            "=HYPERLINK(1)",
            long.as_str(),
            "t_us",
            "RECORD_SEQUENCE",
            "cycle_index",
        ]
        .into_iter()
        .enumerate()
        {
            let mut draft = store
                .begin_lab_edit(ECG_ID)
                .unwrap_or_else(|error| panic!("{error}"));
            draft.acquisition.channels[0].csv_name = bad.into();
            assert!(
                store
                    .save_lab_draft(draft, Some("1.0.0".into()), format!("bad-csv-{index:04}"))
                    .is_err(),
                "{bad:?} must be refused"
            );
        }

        // A raw name may not equal another channel's derived column.
        let mut general = store
            .begin_lab_edit(GENERAL_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        let mut second = general.acquisition.channels[0].clone();
        general.acquisition.channels[0].id = "first".into();
        general.acquisition.channels[0].csv_name = "x".into();
        second.pin = "A1".into();
        second.id = "second".into();
        second.csv_name = "X_v".into();
        general.acquisition.channels.push(second.clone());
        general.plot_defaults = PlotDefaults {
            groups: vec![
                PlotDefaultGroup {
                    channel_ids: vec!["first".into()],
                },
                PlotDefaultGroup {
                    channel_ids: vec!["second".into()],
                },
            ],
        };
        let error = store
            .save_lab_draft(
                general.clone(),
                Some("2.0.0".into()),
                "csv-collide-0001".into(),
            )
            .err()
            .unwrap_or_else(|| panic!("a raw name equal to a derived _V column must be refused"));
        assert!(error.to_string().contains("would appear twice"), "{error}");
        // MPXV pressure columns count as well.
        general.acquisition.channels[0].csv_name = "p_counts".into();
        general.acquisition.channels[0].allowed_conversions =
            vec!["counts_volts".into(), "mpxv_pressure".into()];
        general.acquisition.channels[1].csv_name = "p_kPa".into();
        assert!(store
            .save_lab_draft(
                general.clone(),
                Some("2.0.0".into()),
                "csv-collide-0002".into()
            )
            .is_err());
        general.acquisition.channels[1].csv_name = "01_counts".into();
        let saved = store
            .save_lab_draft(general, Some("2.0.0".into()), "csv-collide-0003".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(saved.profile_version, "2.0.1");

        // A stored lab whose CSV name predates these rules still loads.
        let mut legacy = built_in_profiles().unwrap_or_else(|error| panic!("{error}"))[1].clone();
        legacy.profile_id = "team.legacy".into();
        legacy.source = ProfileSource::Instructor;
        legacy.acquisition.channels[0].csv_name = "ECG, raw".into();
        legacy
            .refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        let legacy_root = dir.path().join("legacy");
        write_catalog(
            &legacy_root,
            &[serde_json::to_string_pretty(&legacy).unwrap_or_default()],
            &[("team.legacy", "1.0.0")],
        );
        let loaded = ProfileStore::with_root(legacy_root).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            loaded
                .get_locked("team.legacy")
                .unwrap_or_else(|error| panic!("{error}"))
                .acquisition
                .channels[0]
                .csv_name,
            "ECG, raw"
        );
    }

    #[test]
    fn new_revisions_bound_text_lengths() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let store = instructor_store(&dir.path().join("profiles"));
        let edit = || {
            store
                .begin_lab_edit(ECG_ID)
                .unwrap_or_else(|error| panic!("{error}"))
        };
        let mut long_name = edit();
        long_name.display_name = "n".repeat(MAX_LAB_NAME_CHARS + 1);
        let mut long_description = edit();
        long_description.description = "d".repeat(MAX_LAB_DESCRIPTION_CHARS + 1);
        let mut broken_label = edit();
        broken_label.acquisition.channels[0].label = "two\nlines".into();
        let mut many_notices = edit();
        many_notices.safety.notices = vec!["Teaching use only.".into(); MAX_LAB_NOTICES + 1];
        let mut long_notice = edit();
        long_notice.safety.notices[0] = "x".repeat(MAX_LAB_NOTICE_CHARS + 1);
        for (index, draft) in [
            long_name,
            long_description,
            broken_label,
            many_notices,
            long_notice,
        ]
        .into_iter()
        .enumerate()
        {
            assert!(store
                .save_lab_draft(
                    draft,
                    Some("1.0.0".into()),
                    format!("text-bound-{index:04}")
                )
                .is_err());
        }
        let mut multi_line_description = edit();
        multi_line_description.description = "First line.\nSecond line.".into();
        assert!(store
            .save_lab_draft(
                multi_line_description,
                Some("1.0.0".into()),
                "text-bound-ok01".into()
            )
            .is_ok());
    }

    #[test]
    fn one_invalid_stored_lab_is_skipped_reported_and_kept() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let first = instructor_store(&root);
        let mut ecg = first
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        ecg.description.push_str(" local");
        first
            .save_lab_draft(ecg, Some("1.0.0".into()), "skip-test-0001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        let team = first
            .duplicate_lab(EMG_ID, "team.emg")
            .unwrap_or_else(|error| panic!("{error}"));
        first
            .save_lab_draft(team, None, "skip-test-0002".into())
            .unwrap_or_else(|error| panic!("{error}"));

        // Damage one stored lab (its hash no longer matches).
        let path = root.join(CATALOG_FILE);
        let mut catalog: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap_or_default())
                .unwrap_or_else(|error| panic!("{error}"));
        for entry in catalog["profiles"].as_array_mut().into_iter().flatten() {
            if entry["profile_id"] == "team.emg" {
                entry["description"] = "edited by hand".into();
            }
        }
        fs::write(
            &path,
            serde_json::to_vec_pretty(&catalog).unwrap_or_default(),
        )
        .unwrap_or_else(|error| panic!("{error}"));

        let second = instructor_store(&root);
        let status = second
            .catalog_status()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(status.writable);
        assert_eq!(status.skipped_labs.len(), 1);
        assert_eq!(
            status.skipped_labs[0].profile_id.as_deref(),
            Some("team.emg")
        );
        assert!(status.skipped_labs[0].reason.contains("hash mismatch"));
        // The other local lab is still loaded and active.
        assert_eq!(
            second
                .get_locked(ECG_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "1.0.1"
        );
        assert!(second.get_locked("team.emg").is_err());
        // Its ID stays reserved while the entry is kept.
        assert!(second.duplicate_lab(ECG_ID, "team.emg").is_err());

        // A write keeps the skipped entry verbatim and its active selection.
        second
            .restore_course_default(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap_or_default())
                .unwrap_or_else(|error| panic!("{error}"));
        assert!(written["profiles"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|entry| entry["profile_id"] == "team.emg"
                && entry["description"] == "edited by hand"));
        assert_eq!(written["active_versions"]["team.emg"], "1.0.0");
        let third = ProfileStore::with_root(root).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            third
                .catalog_status()
                .unwrap_or_else(|error| panic!("{error}"))
                .skipped_labs
                .len(),
            1
        );
    }

    #[test]
    fn missing_catalog_is_recovered_from_a_leftover_backup() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = instructor_store(&root);
        let draft = store
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .save_lab_draft(draft, Some("1.0.0".into()), "recover-backup-1".into())
            .unwrap_or_else(|error| panic!("{error}"));
        // An older build stopped between its two renames.
        fs::rename(root.join(CATALOG_FILE), root.join(LEGACY_CATALOG_BACKUP))
            .unwrap_or_else(|error| panic!("{error}"));

        let recovered = instructor_store(&root);
        assert_eq!(
            recovered
                .get_locked(ECG_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "1.0.1"
        );
        let status = recovered
            .catalog_status()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(status
            .recovered_from
            .as_deref()
            .is_some_and(|path| path.ends_with(LEGACY_CATALOG_BACKUP)));
        assert!(root.join(CATALOG_FILE).is_file());
        assert!(
            root.join(LEGACY_CATALOG_BACKUP).is_file(),
            "the leftover is kept until a later write succeeds"
        );
        recovered
            .restore_course_default(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!root.join(LEGACY_CATALOG_BACKUP).exists());
    }

    #[test]
    fn unreadable_catalog_is_quarantined_and_recovered_from_a_complete_temporary_file() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = instructor_store(&root);
        let draft = store
            .begin_lab_edit(EMG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .save_lab_draft(draft, Some("1.0.0".into()), "recover-temp-001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        fs::copy(
            root.join(CATALOG_FILE),
            root.join("lab_state.json.1234-5-0.tmp"),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        fs::write(root.join(CATALOG_FILE), b"{ truncated")
            .unwrap_or_else(|error| panic!("{error}"));

        let recovered =
            ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            recovered
                .get_locked(EMG_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "1.0.1"
        );
        let quarantined = file_names(&root)
            .into_iter()
            .find(|name| name.starts_with("lab_state.corrupt-"))
            .unwrap_or_else(|| panic!("the unreadable catalog must be kept"));
        assert_eq!(
            fs::read(root.join(quarantined)).unwrap_or_default(),
            b"{ truncated".to_vec()
        );

        // Without any valid copy the catalog still fails closed (read-only factory labs).
        let broken = dir.path().join("broken");
        fs::create_dir_all(&broken).unwrap_or_else(|error| panic!("{error}"));
        fs::write(broken.join(CATALOG_FILE), b"{ bad").unwrap_or_else(|error| panic!("{error}"));
        fs::write(broken.join("lab_state.json.tmp"), b"{ also bad")
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(ProfileStore::with_root(broken.clone()).is_err());
        // The unreadable leftover is renamed aside, so a later reset (whose
        // write cleans up leftovers) cannot delete it.
        assert!(!broken.join("lab_state.json.tmp").exists());
        assert!(file_names(&broken).into_iter().any(|name| {
            name.starts_with("lab_state.corrupt-")
                && fs::read(broken.join(&name)).unwrap_or_default() == b"{ also bad".to_vec()
        }));
        assert_eq!(
            fs::read(broken.join(CATALOG_FILE)).unwrap_or_default(),
            b"{ bad".to_vec()
        );
    }

    #[test]
    fn catalog_writes_replace_the_file_and_leave_no_temporary_files() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = instructor_store(&root);
        for (index, id) in [ECG_ID, EMG_ID].into_iter().enumerate() {
            let draft = store
                .begin_lab_edit(id)
                .unwrap_or_else(|error| panic!("{error}"));
            store
                .save_lab_draft(
                    draft,
                    Some("1.0.0".into()),
                    format!("atomic-write-{index:03}"),
                )
                .unwrap_or_else(|error| panic!("{error}"));
        }
        let names = file_names(&root);
        assert!(names.contains(&CATALOG_FILE.to_string()));
        assert!(names.contains(&CATALOG_LOCK_FILE.to_string()));
        assert!(
            !names
                .iter()
                .any(|name| name.ends_with(".tmp") || name.ends_with(".backup")),
            "{names:?}"
        );
        assert!(read_catalog_file(&root.join(CATALOG_FILE))
            .unwrap_or_else(|error| panic!("{error}"))
            .is_some());
    }

    #[test]
    fn a_second_window_never_overwrites_revisions_saved_by_the_first() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        fs::create_dir_all(&root).unwrap_or_else(|error| panic!("{error}"));
        let first = instructor_store(&root);
        let second = instructor_store(&root);
        let emg_in_second = second
            .begin_lab_edit(EMG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        let stale_ecg_in_second = second
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));

        let ecg = first
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        first
            .save_lab_draft(ecg, Some("1.0.0".into()), "window-one-0001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        second
            .save_lab_draft(
                emg_in_second,
                Some("1.0.0".into()),
                "window-two-0001".into(),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        // The second window sees the first window's ECG revision as active.
        assert!(second
            .save_lab_draft(
                stale_ecg_in_second,
                Some("1.0.0".into()),
                "window-two-0002".into()
            )
            .is_err());

        let reopened = ProfileStore::with_root(root).unwrap_or_else(|error| panic!("{error}"));
        for id in [ECG_ID, EMG_ID] {
            assert_eq!(
                reopened
                    .get_locked(id)
                    .unwrap_or_else(|error| panic!("{error}"))
                    .profile_version,
                "1.0.1",
                "{id}"
            );
        }
    }

    #[test]
    fn catalog_lock_is_exclusive_between_writers() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let held = CatalogLock::acquire(&root).unwrap_or_else(|error| panic!("{error}"));
        assert!(matches!(
            CatalogLock::acquire_within(&root, Duration::from_millis(100)),
            Err(ProfileError::CatalogBusy)
        ));
        drop(held);
        assert!(CatalogLock::acquire_within(&root, Duration::from_millis(100)).is_ok());
    }

    #[test]
    fn reset_keeps_a_timestamped_copy_of_the_catalog_and_reports_it() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = instructor_store(&root);
        let draft = store
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .save_lab_draft(draft, Some("1.0.0".into()), "reset-backup-001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        let before = fs::read(root.join(CATALOG_FILE)).unwrap_or_default();
        let outcome = store
            .reset_local_customizations()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(outcome.removed_local_revisions, 1);
        let backup = PathBuf::from(
            outcome
                .backup_path
                .unwrap_or_else(|| panic!("a healthy catalog must be backed up")),
        );
        assert!(backup
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("lab_state.reset-")));
        assert_eq!(fs::read(&backup).unwrap_or_default(), before);
        assert_eq!(
            store
                .get_locked(ECG_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "1.0.0"
        );
    }

    #[test]
    fn reimporting_an_exported_lab_reports_it_as_already_installed() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let author = instructor_store(&dir.path().join("author"));
        let draft = author
            .duplicate_lab(ECG_ID, "team.ecg")
            .unwrap_or_else(|error| panic!("{error}"));
        let saved = author
            .save_lab_draft(draft, None, "export-import-01".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(saved.source, ProfileSource::Instructor);
        let package = dir.path().join("team.ecg_1.0.0.lab.json");
        author
            .export_profile("team.ecg", "1.0.0", &package)
            .unwrap_or_else(|error| panic!("{error}"));

        let classroom = instructor_store(&dir.path().join("classroom"));
        let imported = classroom
            .import_profile(&package)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(imported.source, ProfileSource::Imported);
        let again = classroom
            .import_profile(&package)
            .err()
            .unwrap_or_else(|| panic!("a repeated import must not create anything"));
        assert!(again.to_string().contains("already installed"), "{again}");
        // Re-importing into the authoring catalog itself is also recognized.
        let own = author
            .import_profile(&package)
            .err()
            .unwrap_or_else(|| panic!("the lab is already installed"));
        assert!(own.to_string().contains("already installed"), "{own}");
    }

    /// A pulse-ox revision as the v1.0.2 editor saved it: an unused
    /// "Channel 1 / A0" plus the legacy `led_outputs` next to the outputs.
    fn v1_0_2_pulse_ox_revision() -> AcquisitionProfile {
        let mut pulse = built_in_profiles()
            .unwrap_or_else(|error| panic!("{error}"))
            .into_iter()
            .find(|profile| profile.profile_id == PULSE_ID)
            .unwrap_or_else(|| panic!("pulse-ox factory lab missing"));
        pulse.profile_version = "1.0.1".into();
        pulse.source = ProfileSource::Instructor;
        pulse.description.push_str(" Edited in v1.0.2.");
        pulse.acquisition.channels.push(ProfileChannel {
            pin: "A0".into(),
            id: "channel_1".into(),
            label: "Channel 1".into(),
            csv_name: "channel_1_counts".into(),
            units: "ADC counts".into(),
            allowed_conversions: vec!["counts_volts".into()],
            default_display_unit: Some("counts".into()),
            default_visible: true,
        });
        pulse
            .refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(pulse.validate().is_ok());
        pulse
    }

    #[test]
    fn pulse_ox_packages_saved_by_the_v1_0_2_editor_still_import() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let stored = v1_0_2_pulse_ox_revision();
        let author_root = dir.path().join("author");
        write_catalog(
            &author_root,
            &[serde_json::to_string_pretty(&stored).unwrap_or_default()],
            &[(PULSE_ID, "1.0.1")],
        );
        let author = instructor_store(&author_root);
        // The stored revision loads unchanged (it is never rewritten).
        let active = author
            .get_locked(PULSE_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(active.acquisition.channels.len(), 1);
        assert_eq!(
            active.integrity.canonical_hash,
            stored.integrity.canonical_hash
        );
        let package = dir.path().join("pulse_1.0.1.lab.json");
        author
            .export_profile(PULSE_ID, "1.0.1", &package)
            .unwrap_or_else(|error| panic!("{error}"));

        let classroom = instructor_store(&dir.path().join("classroom"));
        let imported = classroom
            .import_profile(&package)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(imported.acquisition.channels.is_empty());
        assert_eq!(imported.source, ProfileSource::Imported);
        assert!(imported.validate().is_ok());
        assert_eq!(
            classroom
                .get_locked(PULSE_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "1.0.1"
        );
        // Importing the same package again, here or into the authoring
        // catalog, is recognized as already installed.
        for store in [&classroom, &author] {
            let again = store
                .import_profile(&package)
                .err()
                .unwrap_or_else(|| panic!("the lab is already installed"));
            assert!(again.to_string().contains("already installed"), "{again}");
        }
        // Different pulse-ox content with the same identity is still a conflict.
        let mut different = stored.clone();
        different.acquisition.state_dwell_us = Some(2_000);
        different.acquisition.sample_rate_hz = 125;
        different
            .refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        let conflicting = dir.path().join("pulse_conflict.lab.json");
        fs::write(
            &conflicting,
            serde_json::to_vec_pretty(&different).unwrap_or_default(),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        let conflict = classroom
            .import_profile(&conflicting)
            .err()
            .unwrap_or_else(|| panic!("different content must be refused"));
        assert!(
            conflict.to_string().contains("different content"),
            "{conflict}"
        );
    }

    #[test]
    fn a_catalog_that_cannot_be_read_is_never_replaced_by_a_leftover() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = instructor_store(&root);
        let draft = store
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .save_lab_draft(draft, Some("1.0.0".into()), "io-error-save-01".into())
            .unwrap_or_else(|error| panic!("{error}"));
        let leftover = root.join("lab_state.json.7-1-0.tmp");
        fs::copy(root.join(CATALOG_FILE), &leftover).unwrap_or_else(|error| panic!("{error}"));
        // An I/O failure, not unparseable content: the catalog path cannot be
        // read as a file.
        fs::remove_file(root.join(CATALOG_FILE)).unwrap_or_else(|error| panic!("{error}"));
        fs::create_dir(root.join(CATALOG_FILE)).unwrap_or_else(|error| panic!("{error}"));
        assert!(matches!(
            ProfileStore::with_root(root.clone()),
            Err(ProfileError::Read { .. })
        ));
        assert!(root.join(CATALOG_FILE).is_dir());
        assert!(
            leftover.is_file(),
            "the leftover is left for a later launch"
        );
        assert!(!file_names(&root)
            .iter()
            .any(|name| name.starts_with("lab_state.corrupt-")));
    }

    #[test]
    fn unreadable_leftovers_of_a_missing_catalog_are_kept_and_reported() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        fs::create_dir_all(&root).unwrap_or_else(|error| panic!("{error}"));
        fs::write(root.join("lab_state.json.99-1-0.tmp"), b"{ partial")
            .unwrap_or_else(|error| panic!("{error}"));
        let store = instructor_store(&root);
        let status = store
            .catalog_status()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(status.writable);
        assert!(status.recovered_from.is_none());
        assert_eq!(status.kept_unreadable_files.len(), 1);
        let kept = PathBuf::from(&status.kept_unreadable_files[0]);
        assert!(kept
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("lab_state.corrupt-")));
        assert_eq!(fs::read(&kept).unwrap_or_default(), b"{ partial".to_vec());

        // A later write cleans up leftovers but never the kept copy.
        let draft = store
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .save_lab_draft(draft, Some("1.0.0".into()), "kept-leftover-01".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(kept.is_file());
        assert!(!file_names(&root).iter().any(|name| name.ends_with(".tmp")));
    }

    #[test]
    fn reset_counts_revisions_another_window_saved() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        fs::create_dir_all(&root).unwrap_or_else(|error| panic!("{error}"));
        let first = instructor_store(&root);
        let second = instructor_store(&root);
        let draft = second
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        second
            .save_lab_draft(draft, Some("1.0.0".into()), "other-window-001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        let outcome = first
            .reset_local_customizations()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(outcome.removed_local_revisions, 1);
        let backup = fs::read_to_string(
            outcome
                .backup_path
                .unwrap_or_else(|| panic!("the catalog must be backed up")),
        )
        .unwrap_or_default();
        assert!(backup.contains(ECG_ID));
    }

    #[test]
    fn reset_of_an_unavailable_catalog_keeps_its_leftovers() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = instructor_store(&root);
        let draft = store
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .save_lab_draft(draft, Some("1.0.0".into()), "reset-leftover-1".into())
            .unwrap_or_else(|error| panic!("{error}"));
        // The catalog is missing and its only copy is a leftover that could
        // not be installed at startup.
        fs::rename(root.join(CATALOG_FILE), root.join(LEGACY_CATALOG_BACKUP))
            .unwrap_or_else(|error| panic!("{error}"));
        let before = fs::read(root.join(LEGACY_CATALOG_BACKUP)).unwrap_or_default();
        let unavailable =
            ProfileStore::factory_only(root.clone(), "the leftover could not be restored".into());
        unavailable
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        unavailable
            .reset_local_customizations()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!root.join(LEGACY_CATALOG_BACKUP).exists());
        let kept = file_names(&root)
            .into_iter()
            .find(|name| name.starts_with("lab_state.reset-leftover-"))
            .unwrap_or_else(|| panic!("the leftover must be kept"));
        assert_eq!(fs::read(root.join(kept)).unwrap_or_default(), before);
        assert!(root.join(CATALOG_FILE).is_file());
    }

    #[test]
    fn retiring_the_active_local_revision_of_a_factory_lab_reactivates_the_course_default() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = instructor_store(&root);
        let draft = store
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .save_lab_draft(draft, Some("1.0.0".into()), "retire-active-01".into())
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .retire(ECG_ID, "1.0.1")
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(store.list().unwrap_or_default().len(), 5);
        assert_eq!(
            store
                .get_locked(ECG_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "1.0.0"
        );
        let reopened = ProfileStore::with_root(root).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            reopened
                .get_locked(ECG_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "1.0.0"
        );
    }

    #[test]
    fn lab_duration_rules_are_enforced() {
        let factory = built_in_profiles().unwrap_or_else(|error| panic!("{error}"));
        for profile in &factory {
            assert!(profile
                .acquisition
                .duration_refusal(&RecordingDuration::UntilStopped)
                .is_none());
            assert!(profile
                .acquisition
                .duration_refusal(&RecordingDuration::Timed { seconds: 10 })
                .is_none());
        }
        let mut acquisition = factory[1].acquisition.clone();
        acquisition.allowed_duration_modes = vec!["timed".into()];
        acquisition.minimum_custom_duration_seconds = 30;
        acquisition.timed_presets_seconds = vec![10, 60];
        assert!(acquisition
            .duration_refusal(&RecordingDuration::UntilStopped)
            .is_some());
        assert!(acquisition
            .duration_refusal(&RecordingDuration::Timed { seconds: 10 })
            .is_none());
        assert!(acquisition
            .duration_refusal(&RecordingDuration::Timed { seconds: 20 })
            .is_some_and(|message| message.contains("30 seconds")));
        assert!(acquisition
            .duration_refusal(&RecordingDuration::Timed { seconds: 45 })
            .is_none());
        acquisition.allowed_duration_modes = vec!["until_stopped".into()];
        assert!(acquisition
            .duration_refusal(&RecordingDuration::Timed { seconds: 60 })
            .is_some());
        assert!(acquisition
            .duration_refusal(&RecordingDuration::UntilStopped)
            .is_none());
    }

    #[test]
    fn pulse_ox_cycle_rate_matches_the_editor_rounding() {
        for (dwell, rate) in [
            (250, 1_000),
            (300, 833),
            (400, 625),
            (900, 278),
            (1_000, 250),
            (2_000, 125),
            (4_000, 63),
            (5_000, 50),
        ] {
            assert_eq!(pulse_ox_cycle_rate_hz(dwell), rate, "{dwell}");
        }
    }
}
