//! Versioned, validated acquisition profiles. SHA-256 here detects accidental
//! modification; it is explicitly not an authorship signature or access-control mechanism.
use crate::protocol::{PROTOCOL_MAJOR, PROTOCOL_MINOR, REFERENCE_DEVICE_ID};
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
        Arc, Mutex, OnceLock,
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
/// LED-state dwell of the LED-sequenced pulse-ox mode (CONFIGURE mode 2), in
/// microseconds. Every state reads every configured pin, so its dwell is
/// longer than the legacy 4-state mode's minimum.
pub const PULSEOX_SEQUENCED_DWELL_RANGE_US: std::ops::RangeInclusive<u32> = 1_000..=5_000;
/// The first firmware build with the LED-sequenced mode. A sequenced lab must
/// name it (or a later accepted build) as its `required_firmware.build`.
pub const SEQUENCED_MIN_FIRMWARE_BUILD: u32 = 0x0001_0007;
/// Field 0 of every LED-sequenced record, and its CSV column.
pub const LED_STATE_FIELD: &str = "led_state";
/// LED-state names by state code (`led_state`, which is `record_sequence % 4`).
pub const LED_STATE_NAMES: [&str; 4] = ["RED", "DARK1", "IR", "DARK2"];
/// The CSV name of an LED-state code outside 0–3.
pub const UNKNOWN_LED_STATE_NAME: &str = "UNKNOWN";
/// `required_firmware.build` values a lab may name. It is the minimum feature
/// build the lab needs; the board itself must run this app's reference build
/// (`REFERENCE_FIRMWARE_BUILD`). Every earlier WVU build stays listed: the
/// factory labs and most local labs name 0x00010006, and a lab whose build is
/// missing here fails validation and disappears from the catalog.
pub const ACCEPTED_LAB_FIRMWARE_BUILDS: [u32; 7] = [
    0x0001_0007,
    0x0001_0006,
    0x0001_0005,
    0x0001_0004,
    0x0001_0003,
    0x0001_0002,
    0x0001_0001,
];
/// Fixed CSV columns that a channel's CSV field may not reuse. `led_state`
/// and `led_state_name` are the LED-sequenced CSV's state columns.
/// `lab-authoring.ts` mirrors this list.
pub const RESERVED_CSV_COLUMNS: [&str; 5] = [
    "record_sequence",
    "t_us",
    "cycle_index",
    "led_state",
    "led_state_name",
];
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

/// Startup migration M1 (`op_amp_builtin_v1`). Before the Op Amp Lab was
/// built in, it was distributed as a local revision of General Analog: the
/// v1.0.2 student installer stored General Analog 2.0.1 as `imported`
/// (hash `c73cc1f2…`), and the maintainer's own Lab Manager copy is the same
/// lab as `instructor` (hash `bcc08868…`). While active it replaced General
/// Analog in the Lab list.
const OP_AMP_BUILTIN_MIGRATION: &str = "op_amp_builtin_v1";
const LEGACY_OP_AMP_LAB_ID: &str = "wvu.bmeg420l.general.analog.development.v2";
const LEGACY_OP_AMP_LAB_VERSION: &str = "2.0.1";
const LEGACY_OP_AMP_LAB_HASHES: [&str; 2] = [
    "c73cc1f2941910696d88f934b7b4c09b8b110438a4886d7d799b13807f61471d",
    "bcc08868305ca55e81920e3a5805b020cfa612a0374e75fd5a290fa11f918ad9",
];

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
    /// Legacy pulse-ox: TX/RX read in RED, DARK 1, IR, DARK 2, one 8-field
    /// record per cycle (CONFIGURE mode 1).
    #[serde(rename = "pulseox_4state")]
    Pulseox4State,
    /// LED-sequenced pulse-ox: the LEDs step RED, DARK 1, IR, DARK 2 and
    /// every state reads all of the lab's channels, one record
    /// `[led_state, ch1..chN]` per state (CONFIGURE mode 2, build 0x00010007).
    #[serde(rename = "pulseox_sequenced")]
    PulseoxSequenced,
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
    /// Display only, and only in an LED-sequenced lab: the live plot shows
    /// this channel as one trace per LED state by default. It never changes
    /// acquisition or the recorded data. Omitted when false, so the canonical
    /// bytes (and hashes) of existing labs do not change.
    #[serde(default, skip_serializing_if = "is_false")]
    pub split_by_led_state: bool,
}

fn default_visible() -> bool {
    true
}
fn is_default_visible(value: &bool) -> bool {
    *value
}
fn is_false(value: &bool) -> bool {
    !*value
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

/// Nominal LED-sequenced record rate for a state dwell: one record per LED
/// state, `round(1e6 / dwell)` records/s (1000 for a 1000 µs dwell).
pub fn pulse_ox_sequenced_rate_hz(state_dwell_us: u32) -> u32 {
    if state_dwell_us == 0 {
        return 0;
    }
    let dwell = u64::from(state_dwell_us);
    u32::try_from((2_000_000 + dwell) / (2 * dwell)).unwrap_or(u32::MAX)
}

/// The CSV name of an LED-state code: RED, DARK1, IR, DARK2, or UNKNOWN.
pub fn led_state_name(state: u16) -> &'static str {
    LED_STATE_NAMES
        .get(usize::from(state))
        .copied()
        .unwrap_or(UNKNOWN_LED_STATE_NAME)
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
            split_by_led_state: false,
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
            AcquisitionMode::PulseoxSequenced => std::iter::once(LED_STATE_FIELD.to_string())
                .chain(
                    self.resolved_channels()
                        .into_iter()
                        .map(|channel| channel.csv_name),
                )
                .collect(),
        }
    }

    /// True for the LED-sequenced pulse-ox mode, whose record field 0 is the
    /// LED state and whose records follow `sequence % 4`.
    pub fn is_led_sequenced(&self) -> bool {
        self.acquisition_mode == AcquisitionMode::PulseoxSequenced
    }

    pub fn analog_pins(&self) -> Vec<String> {
        match self.acquisition_mode {
            AcquisitionMode::Simultaneous | AcquisitionMode::PulseoxSequenced => self
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
            AcquisitionMode::PulseoxSequenced
                if !self
                    .state_dwell_us
                    .is_some_and(|dwell| PULSEOX_SEQUENCED_DWELL_RANGE_US.contains(&dwell)) =>
            {
                Some("This lab's LED-state dwell is not supported by the WVU firmware (1000-5000 µs). Ask your instructor to update the lab.")
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
    #[error("a recording is running; install the lab file after it has finished")]
    RecordingActive,
    /// A refusal worded for the person using the app (Install lab file).
    #[error("{0}")]
    Refused(String),
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
            AcquisitionMode::PulseoxSequenced => self.validate_led_sequenced()?,
        }
        if self.acquisition.acquisition_mode != AcquisitionMode::PulseoxSequenced
            && self
                .acquisition
                .channels
                .iter()
                .any(|channel| channel.split_by_led_state)
        {
            return Err(ProfileError::Validation(
                "Show by LED state is available only in an LED-sequenced pulse-ox lab".into(),
            ));
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
            || !ACCEPTED_LAB_FIRMWARE_BUILDS.contains(&parse_hex(&self.required_firmware.build)?)
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

    /// The LED-sequenced pulse-ox rules. The mode is new in firmware build
    /// 0x00010007, so no stored lab predates them and all of them apply when
    /// a lab is loaded (`validate`), including the CSV-column rules that
    /// other modes check only for a revision saved now. The firmware build is
    /// checked here, before the hash.
    fn validate_led_sequenced(&self) -> Result<(), ProfileError> {
        let acquisition = &self.acquisition;
        let channels = &acquisition.channels;
        let mut ids = BTreeSet::new();
        let mut csv_names = BTreeSet::new();
        if channels.is_empty()
            || channels.len() > SUPPORTED_ANALOG_PINS.len()
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
                "LED-sequenced pulse-ox labs require 1 to 6 uniquely named raw-count channels and CSV fields"
                    .into(),
            ));
        }
        self.check_csv_columns()?;
        let Some(dwell) = acquisition
            .state_dwell_us
            .filter(|dwell| PULSEOX_SEQUENCED_DWELL_RANGE_US.contains(dwell))
        else {
            return Err(ProfileError::Validation(
                "the LED-sequenced state dwell must be 1000 to 5000 µs".into(),
            ));
        };
        let expected = pulse_ox_sequenced_rate_hz(dwell);
        if acquisition.sample_rate_hz != expected {
            return Err(ProfileError::Validation(format!(
                "the LED-sequenced record rate must follow the state dwell: {expected} records/s for {dwell} µs per state"
            )));
        }
        let outputs = acquisition.resolved_digital_outputs();
        for role in [PulseOxRole::Red, PulseOxRole::Ir] {
            if outputs
                .iter()
                .filter(|output| output.pulse_ox_role == Some(role))
                .count()
                > 1
            {
                return Err(ProfileError::Validation(
                    "each pulse-ox role (RED, IR) may be assigned to only one output".into(),
                ));
            }
        }
        let red = pulse_ox_output(&outputs, PulseOxRole::Red);
        let ir = pulse_ox_output(&outputs, PulseOxRole::Ir);
        let Some((red, ir)) = red.zip(ir).filter(|(red, ir)| {
            red.pin != ir.pin
                && [red, ir].iter().all(|output| {
                    output.behavior == DigitalOutputBehavior::AcquisitionSequenced
                        && SUPPORTED_DIGITAL_OUTPUT_PINS.contains(&output.pin.as_str())
                })
        }) else {
            return Err(ProfileError::Validation(
                "LED-sequenced pulse-ox labs require distinct acquisition-sequenced RED and IR outputs on D4, D5, or D6"
                    .into(),
            ));
        };
        if outputs
            .iter()
            .any(|output| output.pin != red.pin && output.pin != ir.pin)
        {
            return Err(ProfileError::Validation(
                "pulse-ox labs may define only their RED and IR outputs; remove the other outputs (the firmware keeps them LOW in this mode)"
                    .into(),
            ));
        }
        if parse_hex(&self.required_firmware.build)? < SEQUENCED_MIN_FIRMWARE_BUILD {
            return Err(ProfileError::Validation(format!(
                "LED-sequenced pulse-ox labs require WVU firmware build 0x{SEQUENCED_MIN_FIRMWARE_BUILD:08X} or later"
            )));
        }
        validate_plot_defaults(
            &self.plot_defaults,
            &channels
                .iter()
                .map(|channel| channel.id.clone())
                .collect::<Vec<_>>(),
        )
    }

    /// Stricter rules for a revision created now by Save or Import. Stored
    /// labs are loaded with `validate` alone, so a rule added here can never
    /// hide a lab that is already installed (an older lab is checked again
    /// only when an instructor saves a new revision of it).
    pub fn validate_for_authoring(&self) -> Result<(), ProfileError> {
        self.validate()?;
        if !canonical_semver(&self.profile_version) {
            return Err(ProfileError::Validation(
                "lab version must be MAJOR.MINOR.PATCH numbers without leading zeros".into(),
            ));
        }
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
            // Every LED-sequenced rule is a load-time rule
            // (`validate_led_sequenced`), already checked above.
            AcquisitionMode::PulseoxSequenced => {}
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
    /// A legacy-category gate: only a bench-only lab whose category is the
    /// pre-course "ecg" or "emg" (an older or imported lab) needs the explicit
    /// bench acknowledgement before Start. Current course labs use
    /// `course_*` categories and carry their notices in `safety.notices`;
    /// General Analog (`development`) and the Op Amp Lab (`course_op_amp`)
    /// are bench-only labs that do not need it. Basing this on `bench_only`
    /// alone would change Start for those labs. The frontend mirrors this
    /// rule (LabCard, +page.svelte).
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

/// One compiled-in factory lab: its file name in the repository's
/// `profiles/` folder and the file's text (`include_str!`).
macro_rules! factory_lab_file {
    ($name:literal) => {
        ($name, include_str!(concat!("../../profiles/", $name)))
    };
}

/// The factory (built-in) course labs, in the order `built_in_profiles()`
/// returns them. Append a new factory lab last. A test requires every
/// `profiles/*.profile.json` file to be listed here, and
/// `scripts/update_factory_labs.ps1 promote` prints the line to add.
const FACTORY_LAB_FILES: &[(&str, &str)] = &[
    factory_lab_file!("general_analog_development.profile.json"),
    factory_lab_file!("ecg_course_capture.profile.json"),
    factory_lab_file!("emg_force_course_capture.profile.json"),
    factory_lab_file!("blood_pressure_ppg_course_capture.profile.json"),
    factory_lab_file!("pulseox_tx_rx_course_capture.profile.json"),
    factory_lab_file!("op_amp_course_capture.profile.json"),
];

/// The compiled-in factory lab files as (file name, text) pairs, for the
/// factory lab file tool (`src-tauri/examples/factory_labs.rs`).
pub fn factory_lab_sources() -> &'static [(&'static str, &'static str)] {
    FACTORY_LAB_FILES
}

pub fn built_in_profiles() -> Result<Vec<AcquisitionProfile>, ProfileError> {
    FACTORY_LAB_FILES
        .iter()
        .map(|(_, text)| {
            let profile: AcquisitionProfile = serde_json::from_str(text)?;
            profile.validate()?;
            Ok(profile)
        })
        .collect()
}

/// A compiled-in factory lab, looked up by ID so tests do not depend on the
/// order or the number of factory labs.
#[cfg(test)]
pub(crate) fn factory_lab(profile_id: &str) -> AcquisitionProfile {
    built_in_profiles()
        .unwrap_or_else(|error| panic!("{error}"))
        .into_iter()
        .find(|profile| profile.profile_id == profile_id)
        .unwrap_or_else(|| panic!("no factory lab {profile_id}"))
}

/// The number of compiled-in factory labs.
#[cfg(test)]
pub(crate) fn factory_lab_count() -> usize {
    FACTORY_LAB_FILES.len()
}

/// The built-in pulse-ox lab's published 1.0.0 file (legacy 4-state mode),
/// kept in `profiles/legacy/` since 1.1.0 moved to the LED-sequenced mode.
/// It is not compiled into the application; tests use it for 4-state
/// coverage.
#[cfg(test)]
pub(crate) const LEGACY_PULSE_OX_LAB_TEXT: &str =
    include_str!("../../profiles/legacy/pulseox_tx_rx_course_capture_1.0.0.profile.json");

/// The built-in pulse-ox lab 1.0.0 as published: legacy 4-state mode, TX A0
/// and RX A1, D5 RED and D6 IR, a 1000 µs dwell (250 cycles/s), build
/// 0x00010006. Locked, `built_in`, with its published hash.
#[cfg(test)]
pub(crate) fn legacy_pulse_ox_lab() -> AcquisitionProfile {
    let lab: AcquisitionProfile =
        serde_json::from_str(LEGACY_PULSE_OX_LAB_TEXT).unwrap_or_else(|error| panic!("{error}"));
    lab.validate().unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        lab.acquisition.acquisition_mode,
        AcquisitionMode::Pulseox4State
    );
    lab
}

/// A locked LED-sequenced pulse-ox lab for tests, built from the legacy
/// 4-state lab 1.0.0 (so it does not follow later edits of the factory
/// file): `tx` (A0) and `rx` (A1) channels, both shown by LED state, D5 RED
/// and D6 IR, a 1000 µs dwell, and build 0x00010007. Its acquisition,
/// outputs and plot defaults are those of the built-in pulse-ox lab 1.1.0
/// (a test checks this).
#[cfg(test)]
pub(crate) fn sequenced_test_lab() -> AcquisitionProfile {
    let mut lab = legacy_pulse_ox_lab();
    lab.required_firmware.build = format!("0x{SEQUENCED_MIN_FIRMWARE_BUILD:08X}");
    let acquisition = &mut lab.acquisition;
    acquisition.acquisition_mode = AcquisitionMode::PulseoxSequenced;
    acquisition.analog_inputs = None;
    acquisition.led_outputs = None;
    acquisition.state_dwell_us = Some(1_000);
    acquisition.sample_rate_hz = 1_000;
    acquisition.channels = [("A0", "tx", "TX"), ("A1", "rx", "RX")]
        .into_iter()
        .map(|(pin, id, label)| ProfileChannel {
            pin: pin.into(),
            id: id.into(),
            label: label.into(),
            csv_name: format!("{id}_counts"),
            units: "ADC counts".into(),
            allowed_conversions: vec!["counts_volts".into()],
            default_display_unit: None,
            default_visible: true,
            split_by_led_state: true,
        })
        .collect();
    lab.plot_defaults = PlotDefaults {
        groups: vec![
            PlotDefaultGroup {
                channel_ids: vec!["tx".into()],
            },
            PlotDefaultGroup {
                channel_ids: vec!["rx".into()],
            },
        ],
    };
    lab.refresh_hash().unwrap_or_else(|error| panic!("{error}"));
    lab.validate().unwrap_or_else(|error| panic!("{error}"));
    lab
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
    /// One-time catalog migrations recorded in the catalog file
    /// (`applied_migrations`). A migration listed here never runs again.
    applied_migrations: BTreeSet<String>,
    /// What a startup migration changed in this launch, for the Lab panel.
    migration_notices: Vec<String>,
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
    /// True when the catalog holds the legacy Op Amp Lab copy (General
    /// Analog 2.0.1 with one of its two published, verified hashes), active
    /// or not.
    fn has_legacy_op_amp_lab(&self) -> bool {
        let key = legacy_op_amp_lab_key();
        !self.factory_keys.contains(&key)
            && self.profiles.get(&key).is_some_and(|profile| {
                LEGACY_OP_AMP_LAB_HASHES.contains(&profile.integrity.canonical_hash.as_str())
                    && profile
                        .computed_hash()
                        .is_ok_and(|hash| hash == profile.integrity.canonical_hash)
            })
    }
    /// True when the legacy Op Amp Lab copy is in the catalog and is the
    /// active, unretired General Analog revision.
    fn legacy_op_amp_lab_selected(&self) -> bool {
        let key = legacy_op_amp_lab_key();
        self.has_legacy_op_amp_lab()
            && self.active_versions.get(LEGACY_OP_AMP_LAB_ID) == Some(&key.1)
            && !self.retired.contains(&key)
    }
    /// True when migration `op_amp_builtin_v1` still has to run: it was
    /// never recorded, and the legacy Op Amp Lab copy is the active,
    /// unretired General Analog revision.
    fn needs_op_amp_builtin_migration(&self) -> bool {
        !self.applied_migrations.contains(OP_AMP_BUILTIN_MIGRATION)
            && self.legacy_op_amp_lab_selected()
            && self.factory_version(LEGACY_OP_AMP_LAB_ID).is_some()
    }
    /// Decides migration `op_amp_builtin_v1` for a catalog just read (at
    /// startup or after another window changed it). A legacy Op Amp Lab
    /// copy that is already inactive or retired needs no migration, so the
    /// migration counts as applied and the next write records it; a later
    /// deliberate reactivation of the copy is then respected. A copy that is
    /// still selected is left for the startup migration, and the marker is
    /// never written for it (by any write) until that migration has run. No
    /// copy, no marker: a copy installed later is still migrated.
    fn settle_op_amp_builtin_migration(&mut self) {
        if self.has_legacy_op_amp_lab() && !self.legacy_op_amp_lab_selected() {
            self.applied_migrations
                .insert(OP_AMP_BUILTIN_MIGRATION.to_string());
        }
    }
}

fn legacy_op_amp_lab_key() -> (String, String) {
    (
        LEGACY_OP_AMP_LAB_ID.to_string(),
        LEGACY_OP_AMP_LAB_VERSION.to_string(),
    )
}

#[derive(Clone, Debug, Serialize)]
pub struct LabListEntry {
    pub profile: AcquisitionProfile,
    pub active: bool,
    pub retired: bool,
    /// The file name "Save as course default file…" suggests: the lab's
    /// compiled-in factory file, or one named after the lab (additive field).
    pub course_default_file_name: String,
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
    /// What a one-time startup migration changed in this launch, as plain
    /// sentences for the Lab panel in both modes (additive field).
    pub migration_notices: Vec<String>,
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

/// Result of "Save as course default file…" (`export_course_default`).
#[derive(Clone, Debug, Serialize)]
pub struct CourseDefaultExport {
    /// The file written.
    pub path: String,
    pub file_name: String,
    pub profile_id: String,
    pub display_name: String,
    /// The factory version the file carries (policy P2: MAJOR.MINOR.0).
    pub profile_version: String,
    pub canonical_hash: String,
    /// The course default this application ships for the lab, which the
    /// file would replace (`None`: the file is a new factory lab).
    pub replaces_version: Option<String>,
    /// The lab's factory file in the repository's `profiles/` folder
    /// (`None`: a new factory lab).
    pub factory_file_name: Option<String>,
    /// The `FACTORY_LAB_FILES` line that compiles the file in; a lab that
    /// already has a factory file is compiled in already.
    pub source_line: String,
}

/// One analog input of a lab file, for the Install lab file summary.
#[derive(Clone, Debug, Serialize)]
pub struct LabFileChannel {
    pub pin: String,
    pub label: String,
}

/// The active lab that installing a lab file would replace (same lab ID).
#[derive(Clone, Debug, Serialize)]
pub struct LabFileReplacement {
    pub profile_id: String,
    pub profile_version: String,
    pub display_name: String,
    /// The replaced lab is the course default this application ships, so
    /// Restore course default undoes the install.
    pub course_default: bool,
    /// The lab ID has a shipped course default. Restore course default
    /// selects that default, not a replaced local revision (which Make
    /// active in Manage Labs brings back).
    pub has_course_default: bool,
}

/// What "Install lab file…" found in a lab file (`inspect_lab_file`), shown
/// before the student confirms. Reading it changes nothing.
#[derive(Clone, Debug, Serialize)]
pub struct LabFileSummary {
    pub profile_id: String,
    pub profile_version: String,
    pub display_name: String,
    pub acquisition_mode: AcquisitionMode,
    pub channels: Vec<LabFileChannel>,
    /// Frames per second, or the nominal cycle rate of a pulse-ox lab.
    pub sample_rate_hz: u32,
    pub adc_resolution_bits: u8,
    /// This exact lab version is already installed (or is a course default)
    /// and is the active version of its lab, so installing changes nothing.
    /// The same version installed but not active is made active again by
    /// the install (no refusal); a retired one is refused.
    pub already_installed: bool,
    /// The active lab the install would replace, if any.
    pub replaces: Option<LabFileReplacement>,
    /// Why installing the file would be refused, if it would be.
    pub refusal: Option<String>,
}

/// How a lab package relates to the revisions already in the catalog
/// (Import lab and Install lab file share these rules).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PackageConflict {
    /// The same ID and version with the same content is installed (or is a
    /// course default) and is the active version of its lab.
    AlreadyInstalled,
    /// The same ID and version with the same content is installed but is
    /// not active (for example after Restore course default). Install lab
    /// file makes it active again; Import lab reports it as installed.
    InstalledInactive,
    /// The same ID and version with the same content is installed but was
    /// retired.
    Retired,
    /// The same ID and version with different content is installed.
    DifferentContent,
    /// A stored revision with this ID and version could not be loaded.
    Unloadable,
}

impl PackageConflict {
    /// The Instructor-mode Import lab wording (unchanged).
    fn import_error(self) -> ProfileError {
        ProfileError::Validation(
            match self {
                Self::AlreadyInstalled | Self::InstalledInactive | Self::Retired => {
                    "this exact lab version is already installed; no catalog change was made"
                }
                Self::DifferentContent => {
                    "an installed lab has the same ID and version but different content; import it with a new lab ID"
                }
                Self::Unloadable => {
                    "a stored lab with this ID and version could not be loaded; repair or reset the local catalog before importing it"
                }
            }
            .into(),
        )
    }
    /// The Install lab file wording, for students. (`InstalledInactive` is
    /// not refused: the install makes that version active again.)
    fn install_message(self, profile: &AcquisitionProfile) -> String {
        let lab = format!("{} {}", profile.display_name, profile.profile_version);
        match self {
            Self::AlreadyInstalled | Self::InstalledInactive => {
                format!("{lab} is already installed on this computer; nothing was changed.")
            }
            Self::Retired => format!(
                "{lab} is installed on this computer but was retired, so it was not installed again. Ask your instructor; Restore in Manage Labs brings it back."
            ),
            Self::DifferentContent => format!(
                "A different lab with the same lab ID and version as {lab} is already installed on this computer, so this file was not installed. Ask your instructor for a lab file with a new version."
            ),
            Self::Unloadable => format!(
                "A stored lab with the same lab ID and version as {lab} could not be loaded on this computer, so this file was not installed. Ask your instructor."
            ),
        }
    }
}

/// The catalog file as written. Loaded profiles are serialized in struct
/// field order (as before); skipped entries are written back verbatim.
#[derive(Serialize)]
struct PersistedLabState<'a> {
    profiles: Vec<PersistedProfile<'a>>,
    retired: Vec<(String, String)>,
    active_versions: BTreeMap<String, String>,
    /// Durable idempotency records prevent a frontend retry of the same
    /// explicit Save action from allocating another version.
    completed_save_requests: BTreeMap<String, (String, String)>,
    /// One-time startup migrations already applied (additive; omitted when
    /// empty, so other catalogs are written exactly as before).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    applied_migrations: Vec<String>,
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
    /// Absent from catalogs written before migrations existed.
    #[serde(default)]
    applied_migrations: BTreeSet<String>,
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
    let mut course_defaults = BTreeMap::new();
    for profile in factory {
        let key = pkey(profile);
        factory_keys.insert(key.clone());
        course_defaults.insert(key.clone(), profile);
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
    let mut applied_migrations = BTreeSet::new();
    if let Some(snapshot) = snapshot {
        disk_fingerprint = Some(snapshot.fingerprint);
        let persisted = snapshot.state;
        applied_migrations = persisted.applied_migrations;
        let mut skip = |entry: SkippedEntry| {
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
        };
        for raw in persisted.profiles {
            match load_stored_profile(&raw) {
                Ok(profile) => {
                    let key = pkey(&profile);
                    // Factory definitions are resolved from bundled resources. A
                    // stored lab may never replace the class default merely
                    // because it has the same identity.
                    match course_defaults.get(&key) {
                        None => {
                            profiles.insert(key, profile);
                        }
                        // The same lab as the course default (for example a
                        // local lab later shipped as a factory lab): it is
                        // not needed, and the next catalog write drops it.
                        Some(course_default)
                            if same_lab_content(&profile, course_default).unwrap_or(false) =>
                        {
                            crate::app_log::record(
                                "INFO",
                                &format!(
                                    "LAB_PROFILE_SUPERSEDED id={} version={} reason=identical_to_course_default",
                                    profile.profile_id, profile.profile_version
                                ),
                            );
                        }
                        // Different content under a course default's ID and
                        // version: never loaded and never erased. It is kept
                        // verbatim (written back like every skipped entry)
                        // and reported in the Lab panel notice.
                        Some(course_default) => skip(SkippedEntry::new(
                            raw,
                            format!(
                                "it has the same lab ID and version as the course default \"{}\" {} shipped with this application, but different content",
                                course_default.display_name, course_default.profile_version
                            ),
                        )),
                    }
                }
                Err(reason) => skip(SkippedEntry::new(raw, reason)),
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
    let mut runtime = ProfileRuntime {
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
        applied_migrations,
        migration_notices: Vec::new(),
    };
    runtime.settle_op_amp_builtin_migration();
    runtime
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

/// Writes `bytes` to a uniquely named temporary sibling of `destination`,
/// flushes it to disk, then atomically replaces `destination`. A failure
/// leaves any previous `destination` unchanged and removes the temporary.
fn write_file_atomically(destination: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let name = destination
        .file_name()
        .ok_or_else(|| std::io::Error::other("destination has no file name"))?
        .to_string_lossy()
        .into_owned();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let temporary = destination.with_file_name(format!(
        ".{name}.{}-{nanos}-{}.tmp",
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
        replace_file(&temporary, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
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

/// Writes `bytes` to a new file (an existing file is never replaced) and
/// flushes it to disk. On failure the file this call created is removed.
fn write_new_file_durably(destination: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    if result.is_err() {
        drop(file);
        let _ = fs::remove_file(destination);
    }
    result
}

/// Copies a file's bytes to a new file and flushes the copy to disk. Only
/// the bytes are copied, not the attributes, so a read-only catalog gives a
/// normal copy (`fs::copy` would copy the read-only attribute, and the
/// flush would then fail on a copy left behind).
fn copy_durably(source: &Path, destination: &Path) -> std::io::Result<()> {
    write_new_file_durably(destination, &read_catalog_bytes(source)?)
}

/// A `lab_state.pre-migration-*.json` copy that already holds exactly these
/// catalog bytes, so a start whose migration could not be saved does not
/// add another copy at every start.
fn existing_pre_migration_backup(root: &Path, catalog: &[u8]) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with("lab_state.pre-migration-") && name.ends_with(".json")
        })
        .map(|entry| entry.path())
        .collect();
    candidates.sort();
    candidates
        .into_iter()
        .find(|path| fs::read(path).is_ok_and(|bytes| bytes == catalog))
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
/// authoritative) and the pulse-ox shape the editor shows: for the legacy
/// 4-state mode no analog channels and the cycle rate that follows from the
/// dwell; for the LED-sequenced mode the record rate that follows from the
/// dwell, no legacy TX/RX inputs, and at least the firmware build that has
/// the mode.
fn normalize_authoring_draft(draft: &mut AcquisitionProfile) {
    drop_inert_pulse_ox_channels(draft);
    let acquisition = &mut draft.acquisition;
    acquisition.digital_outputs = acquisition.resolved_digital_outputs();
    acquisition.led_outputs = None;
    match acquisition.acquisition_mode {
        AcquisitionMode::Pulseox4State => {
            if let Some(dwell) = acquisition
                .state_dwell_us
                .filter(|dwell| PULSEOX_DWELL_RANGE_US.contains(dwell))
            {
                acquisition.sample_rate_hz = pulse_ox_cycle_rate_hz(dwell);
            }
        }
        AcquisitionMode::PulseoxSequenced => {
            if let Some(dwell) = acquisition
                .state_dwell_us
                .filter(|dwell| PULSEOX_SEQUENCED_DWELL_RANGE_US.contains(dwell))
            {
                acquisition.sample_rate_hz = pulse_ox_sequenced_rate_hz(dwell);
            }
            acquisition.analog_inputs = None;
            if !parse_hex(&draft.required_firmware.build)
                .is_ok_and(|build| build >= SEQUENCED_MIN_FIRMWARE_BUILD)
            {
                draft.required_firmware.build = format!("0x{SEQUENCED_MIN_FIRMWARE_BUILD:08X}");
            }
        }
        AcquisitionMode::Simultaneous => {}
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

/// Reads a lab package for Import lab and Install lab file. The file's own
/// hash and the stored-lab rules are checked (`load_profile`), and only a
/// saved (locked) lab is accepted. A pulse-ox package saved by the v1.0.2
/// editor carries an unused channel list, which is dropped; the installed
/// copy is re-hashed anyway (its source becomes `imported`).
fn read_lab_package(path: &Path) -> Result<AcquisitionProfile, ProfileError> {
    let mut profile = load_profile(path)?;
    if profile.status != ProfileStatus::Locked {
        return Err(ProfileError::Validation(
            "only finalized locked profile packages may be imported".into(),
        ));
    }
    drop_inert_pulse_ox_channels(&mut profile);
    Ok(profile)
}

/// Whether a package collides with a revision in the catalog. An installed
/// lab is stored as `imported` (or is a factory lab), while its file may say
/// `instructor` or `built_in`, so content is compared, not bytes.
fn package_conflict(
    runtime: &ProfileRuntime,
    profile: &AcquisitionProfile,
) -> Result<Option<PackageConflict>, ProfileError> {
    let key = pkey(profile);
    if let Some(existing) = runtime.profiles.get(&key) {
        return Ok(Some(if !same_lab_content(existing, profile)? {
            PackageConflict::DifferentContent
        } else if runtime.retired.contains(&key) {
            PackageConflict::Retired
        } else if runtime.active_versions.get(&key.0) == Some(&key.1) {
            PackageConflict::AlreadyInstalled
        } else {
            PackageConflict::InstalledInactive
        }));
    }
    Ok(runtime
        .revision_in_use(&key)
        .then_some(PackageConflict::Unloadable))
}

/// What Install lab file does with a lab file (`inspect_lab_file` reports
/// it, `install_lab_file` does it).
enum InstallPlan {
    /// Store the lab and make it active.
    Install,
    /// The same lab is stored but not active: make it active.
    Activate,
    /// Refused, with the reason worded for the student.
    Refuse(String),
}

/// Install lab file's rules, in the order `install_lab_file` applies them.
fn install_plan(
    runtime: &ProfileRuntime,
    profile: &AcquisitionProfile,
) -> Result<InstallPlan, ProfileError> {
    if !runtime.catalog_writable {
        return Ok(InstallPlan::Refuse(catalog_unavailable_install_message()));
    }
    if let Some(refusal) = package_install_refusal(profile)? {
        return Ok(InstallPlan::Refuse(refusal));
    }
    Ok(match package_conflict(runtime, profile)? {
        Some(PackageConflict::InstalledInactive) => InstallPlan::Activate,
        Some(conflict) => InstallPlan::Refuse(conflict.install_message(profile)),
        None => match install_rules_refusal(profile)? {
            Some(refusal) => InstallPlan::Refuse(refusal),
            None => InstallPlan::Install,
        },
    })
}

/// Install lab file refusals that depend only on the file: the legacy Op Amp
/// Lab copy and a course default file for a later build.
fn package_install_refusal(profile: &AcquisitionProfile) -> Result<Option<String>, ProfileError> {
    Ok(if is_legacy_op_amp_package(profile)? {
        Some(legacy_op_amp_install_message())
    } else if is_unpublished_course_default_file(profile) {
        Some(course_default_file_install_message(profile))
    } else {
        None
    })
}

/// A factory lab file (`built_in`) that is not a factory lab version this
/// application ships with that content: a "Save as course default file…"
/// file for a later build. Every file saved from one build gets the same
/// next version, so drafts with different content can share it, and the
/// version that is finally built in can differ again. Such a file is for
/// the repository's `profiles/` folder, not for students (Export is).
fn is_unpublished_course_default_file(profile: &AcquisitionProfile) -> bool {
    profile.source == ProfileSource::BuiltIn
        && !compiled_factory_lab_lock().contains(&FactoryLabLockEntry::of(profile))
}

fn course_default_file_install_message(profile: &AcquisitionProfile) -> String {
    format!(
        "{} {} is a course default file for a future version of this application, not a lab file to install. Ask your instructor for a lab file exported from Manage Labs.",
        profile.display_name, profile.profile_version
    )
}

/// The rules for a lab installed now (`install_package` stores the lab as
/// `imported`, re-hashed, and checks `validate_for_authoring`), applied to a
/// copy so the student sees a refusal before being asked to confirm.
fn install_rules_refusal(profile: &AcquisitionProfile) -> Result<Option<String>, ProfileError> {
    let mut installed = profile.clone();
    installed.source = ProfileSource::Imported;
    installed.integrity.canonical_hash.clear();
    installed.refresh_hash()?;
    Ok(installed.validate_for_authoring().err().map(|error| {
        let reason = match error {
            ProfileError::Validation(reason) => reason,
            other => other.to_string(),
        };
        format!(
            "{} {} does not meet the rules for a lab installed now, so it was not installed: {reason}. Ask your instructor for a corrected lab file.",
            profile.display_name, profile.profile_version
        )
    }))
}

/// True for the Op Amp Lab 2.0.1 copy of the retired v1.0.2 installer
/// (General Analog's lab ID, either published hash). Install lab file
/// refuses it, because it would replace General Analog again and record
/// migration `op_amp_builtin_v1` as done; Import lab (Instructor mode) can
/// still restore it deliberately.
fn is_legacy_op_amp_package(profile: &AcquisitionProfile) -> Result<bool, ProfileError> {
    if pkey(profile) != legacy_op_amp_lab_key() {
        return Ok(false);
    }
    let mut installed = profile.clone();
    installed.source = ProfileSource::Imported;
    installed.refresh_hash()?;
    Ok(LEGACY_OP_AMP_LAB_HASHES.contains(&installed.integrity.canonical_hash.as_str()))
}

fn catalog_unavailable_install_message() -> String {
    "The lab list on this computer could not be loaded, so lab files cannot be installed now. Restart the application; if this continues, ask your instructor.".into()
}

fn legacy_op_amp_install_message() -> String {
    "This is the older Op Amp Lab file, which would replace General Analog. The Op Amp Lab is now built in: choose Op Amp Lab in the lab list instead.".into()
}

/// `profiles/factory-labs.lock.json` as compiled into the application: every
/// factory lab version ever published. "Save as course default file…" picks
/// the next version from it, as `promote` does from the repository's copy.
const FACTORY_LAB_LOCK_TEXT: &str = include_str!("../../profiles/factory-labs.lock.json");

fn compiled_factory_lab_lock() -> Vec<FactoryLabLockEntry> {
    parse_factory_lab_lock(FACTORY_LAB_LOCK_TEXT).unwrap_or_default()
}

/// The compiled-in factory lab file name of each factory lab ID.
fn factory_lab_file_names() -> &'static BTreeMap<String, &'static str> {
    static NAMES: OnceLock<BTreeMap<String, &'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        FACTORY_LAB_FILES
            .iter()
            .filter_map(|(name, text)| Some((parse_lab_text(text).ok()?.profile_id, *name)))
            .collect()
    })
}

/// The file name to save a lab's course default file as: its compiled-in
/// factory file, or one named after the lab (as `promote` names a new
/// factory lab) that no other factory lab uses.
fn course_default_file_name(profile: &AcquisitionProfile) -> String {
    let names = factory_lab_file_names();
    if let Some(name) = names.get(&profile.profile_id) {
        return (*name).to_string();
    }
    let stem = factory_lab_file_stem(&profile.display_name);
    let taken = |name: &String| names.values().any(|existing| *existing == name.as_str());
    let mut name = format!("{stem}{FACTORY_LAB_FILE_SUFFIX}");
    let mut number = 2;
    while taken(&name) {
        name = format!("{stem}_{number}{FACTORY_LAB_FILE_SUFFIX}");
        number += 1;
    }
    name
}

/// The next revision of a lab: one patch above the highest existing patch
/// with the same ID and MAJOR.MINOR as the edit base, counting factory,
/// retired, and skipped revisions, so a restored default or an older active
/// revision can always be edited. General Analog 2.0.1 always counts as
/// taken, on every computer: it is the Op Amp Lab copy that the retired
/// v1.0.2 installer stored on student computers, where migration
/// `op_amp_builtin_v1` keeps it (retired), so a different General Analog
/// 2.0.1 from an instructor could never be installed there.
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
    let (major, minor, base_patch) = version_key(base).ok_or_else(|| {
        ProfileError::Validation(
            "this lab's version number is too large to create a new revision; duplicate it as a new lab instead".into(),
        )
    })?;
    let highest = runtime
        .profiles
        .keys()
        .cloned()
        .chain(runtime.retired.iter().cloned())
        .chain(runtime.skipped.iter().filter_map(SkippedEntry::key))
        .chain(std::iter::once(legacy_op_amp_lab_key()))
        .filter(|(id, version)| id == profile_id && valid_semver(version))
        .filter_map(|(_, version)| version_key(&version))
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
        // Loading (and any recovery or migration write) must not overlap
        // another window's catalog write. Nothing is created when the folder
        // does not exist.
        let catalog_lock = if root.is_dir() {
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
        let store = Self {
            root,
            runtime: Arc::new(Mutex::new(runtime)),
        };
        store.apply_startup_migrations(catalog_lock.is_some());
        drop(catalog_lock);
        Ok(store)
    }
    /// One-time catalog migrations. Only `with_root` runs them, while it
    /// holds the catalog lock (never `sync_with_disk`, which only reloads
    /// what another window wrote). Without the lock nothing is written and
    /// the migration is tried again at the next start.
    fn apply_startup_migrations(&self, catalog_lock_held: bool) {
        let Ok(mut runtime) = self.lock() else {
            return;
        };
        if !runtime.catalog_writable || !runtime.needs_op_amp_builtin_migration() {
            return;
        }
        if !catalog_lock_held {
            crate::app_log::record(
                "WARN",
                &format!(
                    "LAB_CATALOG_MIGRATION_DEFERRED migration={OP_AMP_BUILTIN_MIGRATION} reason=catalog_lock_unavailable"
                ),
            );
            return;
        }
        self.migrate_legacy_op_amp_lab(&mut runtime);
    }
    /// Migration `op_amp_builtin_v1`: the Op Amp Lab is now a factory lab
    /// with its own ID, so the legacy copy that replaced General Analog is
    /// retired and General Analog's course default is active again. The
    /// catalog is copied to `lab_state.pre-migration-<time>.json` first (an
    /// existing copy with the same bytes, left by a start whose write failed,
    /// is used again); if that copy fails nothing changes. If the write
    /// fails, this launch keeps the migrated labs in memory and the next
    /// start tries again.
    fn migrate_legacy_op_amp_lab(&self, runtime: &mut ProfileRuntime) {
        let key = legacy_op_amp_lab_key();
        let Some(general_version) = runtime.factory_version(LEGACY_OP_AMP_LAB_ID) else {
            return;
        };
        let retired_hash = runtime
            .profiles
            .get(&key)
            .map(|profile| profile.integrity.canonical_hash.clone())
            .unwrap_or_default();
        let catalog = self.root.join(CATALOG_FILE);
        let mut backup = self.root.join(format!(
            "lab_state.pre-migration-{}.json",
            Utc::now().format("%Y%m%dT%H%M%S%.3fZ")
        ));
        let backed_up = read_catalog_bytes(&catalog).and_then(|bytes| {
            match existing_pre_migration_backup(&self.root, &bytes) {
                Some(existing) => {
                    backup = existing;
                    Ok(())
                }
                None => write_new_file_durably(&backup, &bytes),
            }
        });
        if let Err(error) = backed_up {
            crate::app_log::record(
                "WARN",
                &format!(
                    "LAB_CATALOG_MIGRATION_DEFERRED migration={OP_AMP_BUILTIN_MIGRATION} reason=backup_failed path={} detail={error}",
                    backup.display()
                ),
            );
            return;
        }
        let mut next = runtime.clone();
        next.retired.insert(key.clone());
        next.active_versions
            .insert(LEGACY_OP_AMP_LAB_ID.to_string(), general_version.clone());
        next.preserved_active.remove(LEGACY_OP_AMP_LAB_ID);
        // Recorded with the change; after a failed write a later write in
        // this launch saves both.
        next.applied_migrations
            .insert(OP_AMP_BUILTIN_MIGRATION.to_string());
        let details = format!(
            "migration={OP_AMP_BUILTIN_MIGRATION} lab_id={LEGACY_OP_AMP_LAB_ID} retired_version={} retired_hash={retired_hash} active_version={general_version} backup={}",
            key.1,
            backup.display()
        );
        match self.commit(&mut next) {
            Ok(()) => {
                next.migration_notices.push(format!(
                    "The Op Amp Lab is now built in. The older Op Amp Lab copy that replaced General Analog was retired; a backup was saved to {}.",
                    backup.display()
                ));
                *runtime = next;
                crate::app_log::record("INFO", &format!("LAB_CATALOG_MIGRATED {details}"));
                self.append_lab_write_log_best_effort(
                    OP_AMP_BUILTIN_MIGRATION,
                    LEGACY_OP_AMP_LAB_ID,
                    Some(&key.1),
                    Some(&general_version),
                    "startup_migration",
                );
            }
            Err(error) => {
                // The file is unchanged (and `disk_fingerprint` still
                // describes it), so a later write in this launch saves the
                // migrated labs; otherwise the next start migrates again.
                next.migration_notices.push(format!(
                    "The Op Amp Lab is now built in. The older Op Amp Lab copy that replaced General Analog was retired for this session, but the lab catalog could not be saved, so this is tried again at the next start; a backup was saved to {}.",
                    backup.display()
                ));
                *runtime = next;
                crate::app_log::record(
                    "WARN",
                    &format!("LAB_CATALOG_MIGRATION_NOT_SAVED {details} detail={error}"),
                );
            }
        }
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
            migration_notices: runtime.migration_notices.clone(),
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
                course_default_file_name: course_default_file_name(profile),
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
        // A new catalog: a legacy Op Amp Lab copy installed later is
        // migrated again at the next start.
        next.applied_migrations.clear();
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
        let profile = read_lab_package(path)?;
        let (_catalog_lock, mut runtime) = self.begin_write()?;
        if let Some(conflict) = package_conflict(&runtime, &profile)? {
            return Err(conflict.import_error());
        }
        let (profile, _) = self.install_package(&mut runtime, profile)?;
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
    /// Reads a lab file for "Install lab file…" without changing anything:
    /// the lab, whether it is already installed, the active lab it would
    /// replace (a course default included), and why installing it would be
    /// refused. Available in Student and Instructor mode.
    pub fn inspect_lab_file(&self, path: &Path) -> Result<LabFileSummary, ProfileError> {
        let profile = read_lab_package(path)?;
        let runtime = self.lock()?;
        let conflict = package_conflict(&runtime, &profile)?;
        let refusal = match install_plan(&runtime, &profile)? {
            InstallPlan::Refuse(refusal) => Some(refusal),
            InstallPlan::Install | InstallPlan::Activate => None,
        };
        let key = pkey(&profile);
        let replaces = runtime
            .active_versions
            .get(&profile.profile_id)
            .map(|version| (profile.profile_id.clone(), version.clone()))
            .filter(|active| *active != key && !runtime.retired.contains(active))
            .and_then(|active| {
                let replaced = runtime.profiles.get(&active)?;
                Some(LabFileReplacement {
                    profile_id: replaced.profile_id.clone(),
                    profile_version: replaced.profile_version.clone(),
                    display_name: replaced.display_name.clone(),
                    course_default: runtime.factory_keys.contains(&active),
                    has_course_default: runtime.factory_version(&active.0).is_some(),
                })
            });
        let channels = match profile.acquisition.acquisition_mode {
            AcquisitionMode::Simultaneous | AcquisitionMode::PulseoxSequenced => profile
                .acquisition
                .resolved_channels()
                .into_iter()
                .map(|channel| LabFileChannel {
                    pin: channel.pin,
                    label: channel.label,
                })
                .collect(),
            AcquisitionMode::Pulseox4State => profile
                .acquisition
                .analog_inputs
                .iter()
                .flat_map(|inputs| {
                    [
                        LabFileChannel {
                            pin: inputs.tx.clone(),
                            label: "TX".into(),
                        },
                        LabFileChannel {
                            pin: inputs.rx.clone(),
                            label: "RX".into(),
                        },
                    ]
                })
                .collect(),
        };
        Ok(LabFileSummary {
            already_installed: conflict == Some(PackageConflict::AlreadyInstalled),
            replaces,
            refusal,
            channels,
            sample_rate_hz: profile.acquisition.sample_rate_hz,
            adc_resolution_bits: profile.acquisition.adc_resolution_bits,
            acquisition_mode: profile.acquisition.acquisition_mode.clone(),
            display_name: profile.display_name,
            profile_version: profile.profile_version,
            profile_id: profile.profile_id,
        })
    }
    /// "Install lab file…": installs a lab file in Student or Instructor
    /// mode, with Import lab's parsing, validation, and conflict rules. The
    /// lab is stored as `imported` and becomes the active version of its lab
    /// ID, replacing the active one (a course default included). Restore
    /// course default undoes it when it replaced the course default;
    /// otherwise Make active on the replaced revision in Manage Labs does.
    /// The same lab stored but not active (for example after Restore course
    /// default) is made active again. Refused while a recording runs
    /// (`recording_active`, from the session), while the catalog is
    /// unavailable, for the Op Amp Lab copy of the retired v1.0.2 installer,
    /// for a course default file that is not a shipped factory version, for
    /// a lab version that is already active, retired, or installed with
    /// other content, and for a lab that breaks the rules for a lab
    /// installed now (`install_plan`, which `inspect_lab_file` reports).
    pub fn install_lab_file(
        &self,
        path: &Path,
        recording_active: bool,
    ) -> Result<AcquisitionProfile, ProfileError> {
        if recording_active {
            return Err(ProfileError::RecordingActive);
        }
        if !self.lock()?.catalog_writable {
            return Err(ProfileError::Refused(catalog_unavailable_install_message()));
        }
        let profile = read_lab_package(path)?;
        // Refusals that depend only on the file come before the catalog lock;
        // the rest are decided on the catalog as it is on disk.
        if let Some(refusal) = package_install_refusal(&profile)? {
            return Err(ProfileError::Refused(refusal));
        }
        let (_catalog_lock, mut runtime) = self.begin_write()?;
        let (profile, replaced_version) = match install_plan(&runtime, &profile)? {
            InstallPlan::Refuse(refusal) => return Err(ProfileError::Refused(refusal)),
            InstallPlan::Install => self.install_package(&mut runtime, profile)?,
            InstallPlan::Activate => {
                let key = pkey(&profile);
                let stored = runtime.profiles.get(&key).cloned().ok_or_else(|| {
                    ProfileError::Validation("installed lab revision not found".into())
                })?;
                let replaced_version = runtime.active_versions.get(&key.0).cloned();
                let mut next = runtime.clone();
                next.active_versions.insert(key.0.clone(), key.1.clone());
                next.preserved_active.remove(&key.0);
                self.commit(&mut next)?;
                *runtime = next;
                (stored, replaced_version)
            }
        };
        drop(runtime);
        self.append_lab_write_log_best_effort(
            "student_install",
            &profile.profile_id,
            replaced_version.as_deref(),
            Some(&profile.profile_version),
            "explicit",
        );
        crate::app_log::record(
            "INFO",
            &format!(
                "LAB_FILE_INSTALLED lab_id={} version={} replaced_version={} hash={}",
                profile.profile_id,
                profile.profile_version,
                replaced_version.as_deref().unwrap_or("-"),
                profile.integrity.canonical_hash
            ),
        );
        Ok(profile)
    }
    /// Stores a checked lab package as `imported` (re-hashed, with the rules
    /// for a lab saved now) and makes it the active version of its lab ID.
    /// Returns the stored lab and the active version it replaced. The caller
    /// holds the catalog lock and has refused conflicts.
    fn install_package(
        &self,
        runtime: &mut ProfileRuntime,
        mut profile: AcquisitionProfile,
    ) -> Result<(AcquisitionProfile, Option<String>), ProfileError> {
        profile.source = ProfileSource::Imported;
        profile.integrity.canonical_hash.clear();
        profile.refresh_hash()?;
        profile.validate_for_authoring()?;
        let replaced_version = runtime.active_versions.get(&profile.profile_id).cloned();
        let mut next = runtime.clone();
        next.profiles.insert(pkey(&profile), profile.clone());
        next.active_versions
            .insert(profile.profile_id.clone(), profile.profile_version.clone());
        next.preserved_active.remove(&profile.profile_id);
        // Only Import lab gets here with the legacy Op Amp Lab copy (Install
        // lab file refuses it): a deliberate restore, which migration
        // `op_amp_builtin_v1` must not undo at the next start.
        if is_legacy_op_amp_package(&profile)? {
            next.applied_migrations
                .insert(OP_AMP_BUILTIN_MIGRATION.to_string());
        }
        self.commit(&mut next)?;
        *runtime = next;
        Ok((profile, replaced_version))
    }
    /// Writes one stored revision to a file the user chose (the Save dialog
    /// confirmed any overwrite). Export only reads the catalog, so it is
    /// allowed in either mode; Lab Manager is where the UI offers it. The
    /// file is written to a temporary sibling and then moved into place, so
    /// an interrupted export never leaves a truncated `.lab.json`.
    pub fn export_profile(
        &self,
        profile_id: &str,
        profile_version: &str,
        destination: &Path,
    ) -> Result<(), ProfileError> {
        let profile = self.stored_revision(profile_id, profile_version)?;
        if destination.file_name().is_none()
            || destination
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(ProfileError::Validation(
                "invalid profile export destination".into(),
            ));
        }
        let bytes = serde_json::to_vec_pretty(&profile)?;
        write_file_atomically(destination, &bytes).map_err(|source| ProfileError::Write {
            path: destination.display().to_string(),
            source,
        })
    }
    /// "Save as course default file…" (Instructor mode): writes one stored
    /// revision as a factory lab file for the repository's `profiles/`
    /// folder, with the normalization of `scripts/update_factory_labs.ps1
    /// promote`: the editor's Save normalization, source `built_in`, status
    /// `locked`, a fresh hash, the rules for a lab saved now, and the next
    /// factory version under policy P2 (`next_factory_lab_version`: one MINOR
    /// above the compiled-in lock, the course default, and the revision
    /// itself, so a new lab ID saved as 1.0.x becomes 1.1.0). Every file
    /// saved from one build of a lab gets the same version, so the file is
    /// for the repository, not for students. The file name must end in `.profile.json`;
    /// it is written like an export (temporary sibling, then replace). The
    /// catalog is not changed.
    pub fn export_course_default(
        &self,
        profile_id: &str,
        profile_version: &str,
        destination: &Path,
    ) -> Result<CourseDefaultExport, ProfileError> {
        self.require_instructor()?;
        let file_name = destination
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if file_name.is_empty()
            || destination
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(ProfileError::Validation(
                "invalid course default file destination".into(),
            ));
        }
        let mut profile = self.stored_revision(profile_id, profile_version)?;
        if file_name.len() <= FACTORY_LAB_FILE_SUFFIX.len()
            || !file_name
                .to_ascii_lowercase()
                .ends_with(FACTORY_LAB_FILE_SUFFIX)
        {
            return Err(ProfileError::Validation(format!(
                "a course default file name must end in {FACTORY_LAB_FILE_SUFFIX} (the lab-file check reads only those files); choose a name such as {}",
                course_default_file_name(&profile)
            )));
        }
        let course_default = {
            let runtime = self.lock()?;
            runtime.factory_version(profile_id).and_then(|version| {
                runtime
                    .profiles
                    .get(&(profile_id.to_string(), version))
                    .cloned()
            })
        };
        if profile.status != ProfileStatus::Locked {
            return Err(ProfileError::Validation(
                "only a saved (locked) lab revision can become a course default".into(),
            ));
        }
        // The revision's own version counts too, so a new lab ID's first
        // factory version is above its local 1.0.x revisions.
        let mut known = vec![profile.profile_version.clone()];
        known.extend(
            course_default
                .as_ref()
                .map(|lab| lab.profile_version.clone()),
        );
        let known: Vec<&str> = known.iter().map(String::as_str).collect();
        let version = next_factory_lab_version(&compiled_factory_lab_lock(), profile_id, &known);
        make_factory_lab(&mut profile, version)?;
        if let Some(current) = &course_default {
            let mut same_version = profile.clone();
            same_version.profile_version = current.profile_version.clone();
            if same_lab_content(&same_version, current)? {
                return Err(ProfileError::Validation(format!(
                    "{} {profile_version} has the same content as the course default {} {} shipped with this application; there is nothing to save",
                    profile.display_name, current.display_name, current.profile_version
                )));
            }
        }
        let text = factory_lab_file_text(&profile, "\n")?;
        if parse_lab_text(&text)? != profile {
            return Err(ProfileError::Validation(
                "the course default file did not read back unchanged; nothing was written".into(),
            ));
        }
        write_text_file(destination, &text)?;
        let factory_file_name = factory_lab_file_names()
            .get(profile_id)
            .map(|name| name.to_string());
        let outcome = CourseDefaultExport {
            path: destination.display().to_string(),
            source_line: factory_lab_source_line(
                factory_file_name.as_deref().unwrap_or(&file_name),
            ),
            file_name,
            profile_id: profile.profile_id.clone(),
            display_name: profile.display_name.clone(),
            profile_version: profile.profile_version.clone(),
            canonical_hash: profile.integrity.canonical_hash.clone(),
            replaces_version: course_default.map(|lab| lab.profile_version),
            factory_file_name,
        };
        self.append_lab_write_log_best_effort(
            "course_default_export",
            profile_id,
            Some(profile_version),
            Some(&outcome.profile_version),
            "explicit",
        );
        crate::app_log::record(
            "INFO",
            &format!(
                "LAB_COURSE_DEFAULT_EXPORTED lab_id={profile_id} from_version={profile_version} version={} hash={} path={}",
                outcome.profile_version, outcome.canonical_hash, outcome.path
            ),
        );
        Ok(outcome)
    }
    /// One stored revision (a course default included).
    fn stored_revision(
        &self,
        profile_id: &str,
        profile_version: &str,
    ) -> Result<AcquisitionProfile, ProfileError> {
        self.lock()?
            .profiles
            .get(&(profile_id.into(), profile_version.into()))
            .cloned()
            .ok_or_else(|| ProfileError::Validation("profile not found".into()))
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
        // Startup migrations never run here; this launch's notices stay.
        reloaded.migration_notices = runtime.migration_notices.clone();
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
            // Everything except the compiled-in course defaults. Selecting by
            // key, not by `source`, keeps a stored lab that says `built_in`
            // (one that is not a current course default) on the next write.
            profiles: runtime
                .profiles
                .iter()
                .filter(|(key, _)| !runtime.factory_keys.contains(*key))
                .map(|(_, profile)| PersistedProfile::Loaded(profile))
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
            // Decided when the catalog was read, by the startup migration,
            // or by a deliberate import (`settle_op_amp_builtin_migration`).
            applied_migrations: runtime.applied_migrations.iter().cloned().collect(),
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
/// The load-time version rule. Stored labs are never rejected by the
/// stricter `canonical_semver`, which applies only to versions written now.
fn valid_semver(value: &str) -> bool {
    let fields: Vec<_> = value.split('.').collect();
    fields.len() == 3
        && fields
            .iter()
            .all(|field| !field.is_empty() && field.chars().all(|c| c.is_ascii_digit()))
}
/// A version saved or imported now: MAJOR.MINOR.PATCH with no leading zeros
/// and every number within u64, so it orders exactly as it reads ("1.0.01"
/// would otherwise be a second key for 1.0.1).
fn canonical_semver(value: &str) -> bool {
    valid_semver(value)
        && value
            .split('.')
            .all(|field| (field == "0" || !field.starts_with('0')) && field.parse::<u64>().is_ok())
}
/// Numeric ordering key, or `None` when a field does not fit in u64 (only a
/// legacy stored version can be that large). Such a version is never treated
/// as 0.0.0 or used as the base of a new revision number.
fn version_key(value: &str) -> Option<(u64, u64, u64)> {
    let mut fields = value.split('.').map(|field| field.parse::<u64>().ok());
    let key = (fields.next()??, fields.next()??, fields.next()??);
    fields.next().is_none().then_some(key)
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

// Factory lab files: the maintainer tool behind `scripts/update_factory_labs.ps1`
// (`src-tauri/examples/factory_labs.rs`). In the application, only "Save as
// course default file…" (`ProfileStore::export_course_default`) uses these
// helpers, to write the same kind of file as `promote`.
//
// Version policy P2: a factory lab version is always MAJOR.MINOR.0 and a change
// to a factory lab's content publishes the next MINOR version. An edit of a
// lab always gets PATCH >= 1 (`next_revision_version`). A new lab ID starts
// locally at 1.0.0, so its first factory version is one MINOR above the
// revision it is made from (1.0.x gives 1.1.0, `next_factory_lab_version`).
// Either way a factory version never has the ID and version of a local
// revision. `profiles/factory-labs.lock.json` records every factory lab
// version ever shipped; an entry is never changed or removed, and every one of
// them stays available as a file: the current file in `profiles/`, or an
// earlier version in `profiles/legacy/`.

/// The append-only record of published factory lab versions, in `profiles/`.
pub const FACTORY_LAB_LOCK_FILE: &str = "factory-labs.lock.json";
/// File name suffix of a factory lab file.
pub const FACTORY_LAB_FILE_SUFFIX: &str = ".profile.json";
/// Subfolder of `profiles/` for earlier factory lab versions kept as files.
/// They are not compiled in; each must still match its lock entry.
pub const FACTORY_LAB_LEGACY_FOLDER: &str = "legacy";

/// One published factory lab version in `factory-labs.lock.json`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactoryLabLockEntry {
    pub profile_id: String,
    pub profile_version: String,
    pub canonical_hash: String,
}

impl FactoryLabLockEntry {
    fn of(profile: &AcquisitionProfile) -> Self {
        Self {
            profile_id: profile.profile_id.clone(),
            profile_version: profile.profile_version.clone(),
            canonical_hash: profile.integrity.canonical_hash.clone(),
        }
    }
    fn key(&self) -> (String, String) {
        (self.profile_id.clone(), self.profile_version.clone())
    }
}

/// Result of `factory_lab_check`.
#[derive(Clone, Debug, Default)]
pub struct FactoryLabReport {
    /// One line per lab file checked: file, lab ID, version, and hash.
    pub labs: Vec<String>,
    /// Everything that needs fixing. Empty when the folder is consistent.
    pub problems: Vec<String>,
}

impl FactoryLabReport {
    pub fn is_ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// Result of `rehash_factory_file`.
#[derive(Clone, Debug)]
pub struct FactoryFixOutcome {
    pub profile_id: String,
    pub previous_version: String,
    pub profile_version: String,
    pub canonical_hash: String,
    /// The lab file was rewritten (a new hash and/or version).
    pub file_changed: bool,
    /// A new entry was appended to the lock.
    pub lock_appended: bool,
    /// Published versions this change replaced, kept in `profiles/legacy/`.
    pub kept_files: Vec<PathBuf>,
}

/// Result of `promote_lab_file`.
#[derive(Clone, Debug)]
pub struct FactoryPromoteOutcome {
    pub path: PathBuf,
    pub file_name: String,
    pub profile_id: String,
    pub profile_version: String,
    pub canonical_hash: String,
    /// The factory version this file replaced, when the lab already had one.
    pub replaced_version: Option<String>,
    /// Published versions this change replaced, kept in `profiles/legacy/`.
    pub kept_files: Vec<PathBuf>,
}

/// Policy P2: a factory lab version is MAJOR.MINOR.0 in canonical form.
pub fn is_factory_lab_version(version: &str) -> bool {
    canonical_semver(version) && version_key(version).is_some_and(|(_, _, patch)| patch == 0)
}

/// The `FACTORY_LAB_FILES` line that compiles a factory lab file in.
pub fn factory_lab_source_line(file_name: &str) -> String {
    format!("factory_lab_file!(\"{file_name}\"),")
}

/// The name an earlier version of a factory lab is kept under in
/// `profiles/legacy/`: its file name with the version added, for example
/// `ecg_course_capture_1.0.0.profile.json`.
pub fn legacy_factory_lab_file_name(file_name: &str, version: &str) -> String {
    let stem = file_name
        .strip_suffix(FACTORY_LAB_FILE_SUFFIX)
        .unwrap_or(file_name);
    format!("{stem}_{version}{FACTORY_LAB_FILE_SUFFIX}")
}

pub fn parse_factory_lab_lock(text: &str) -> Result<Vec<FactoryLabLockEntry>, ProfileError> {
    Ok(serde_json::from_str(
        text.strip_prefix('\u{feff}').unwrap_or(text),
    )?)
}

pub fn read_factory_lab_lock(path: &Path) -> Result<Vec<FactoryLabLockEntry>, ProfileError> {
    parse_factory_lab_lock(&read_text_file(path)?)
}

/// Problems in the lock itself: malformed entries and an ID and version
/// listed more than once.
pub fn factory_lab_lock_problems(lock: &[FactoryLabLockEntry]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen: BTreeMap<(String, String), &str> = BTreeMap::new();
    for entry in lock {
        let label = format!("{} {}", entry.profile_id, entry.profile_version);
        if !valid_profile_id(&entry.profile_id) {
            problems.push(format!("lock entry {label}: invalid lab ID"));
        }
        if !is_factory_lab_version(&entry.profile_version) {
            problems.push(format!(
                "lock entry {label}: a factory lab version is MAJOR.MINOR.0"
            ));
        }
        if !is_sha256_hex(&entry.canonical_hash) {
            problems.push(format!(
                "lock entry {label}: canonical_hash must be 64 lowercase hexadecimal characters"
            ));
        }
        match seen.insert(entry.key(), &entry.canonical_hash) {
            Some(previous) if previous != entry.canonical_hash => problems.push(format!(
                "lock entry {label} is listed twice with different hashes ({previous} and {})",
                entry.canonical_hash
            )),
            Some(_) => problems.push(format!("lock entry {label} is listed twice")),
            None => {}
        }
    }
    problems
}

/// The lock as written: a JSON array with one entry per line.
pub fn factory_lab_lock_text(
    lock: &[FactoryLabLockEntry],
    line_ending: &'static str,
) -> Result<String, ProfileError> {
    let mut text = String::from("[");
    for (index, entry) in lock.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        text.push_str(line_ending);
        text.push_str("  ");
        text.push_str(&factory_json(
            entry,
            FactoryJsonLayout::Inline,
            line_ending,
        )?);
    }
    if !lock.is_empty() {
        text.push_str(line_ending);
    }
    text.push(']');
    text.push_str(line_ending);
    Ok(text)
}

/// A factory lab file laid out like the hand-written ones: the top level and
/// `acquisition` one key per line, `acquisition.channels` one channel per
/// line, everything else inline. The canonical hash does not depend on it.
pub fn factory_lab_file_text(
    profile: &AcquisitionProfile,
    line_ending: &'static str,
) -> Result<String, ProfileError> {
    let mut text = factory_json(profile, FactoryJsonLayout::LabFile, line_ending)?;
    text.push_str(line_ending);
    Ok(text)
}

/// Checks a factory lab folder (the repository's `profiles/`) against the
/// labs compiled into the application (`factory_lab_sources()`):
/// - every `*.profile.json` file parses, validates (including the rules for
///   a lab saved now) and carries its canonical hash;
/// - version MAJOR.MINOR.0, source `built_in`, status `locked`, one file per
///   lab ID, and no version older than one already published;
/// - every file is compiled in, and every compiled-in file exists;
/// - the lock is well formed and records every file's version and hash;
/// - files in `legacy/` match their lock entries as well.
pub fn factory_lab_check(profiles_dir: &Path, compiled: &[(&str, &str)]) -> FactoryLabReport {
    let mut report = FactoryLabReport::default();
    let lock = match read_factory_lab_lock(&profiles_dir.join(FACTORY_LAB_LOCK_FILE)) {
        Ok(lock) => {
            report.problems.extend(
                factory_lab_lock_problems(&lock)
                    .into_iter()
                    .map(|problem| format!("{FACTORY_LAB_LOCK_FILE}: {problem}")),
            );
            lock
        }
        Err(error) => {
            report
                .problems
                .push(format!("{FACTORY_LAB_LOCK_FILE}: {error}"));
            Vec::new()
        }
    };
    let files = factory_lab_files_in(profiles_dir).unwrap_or_else(|error| {
        report
            .problems
            .push(format!("{}: {error}", profiles_dir.display()));
        Vec::new()
    });
    if files.is_empty() {
        report.problems.push(format!(
            "no *{FACTORY_LAB_FILE_SUFFIX} files in {}",
            profiles_dir.display()
        ));
    }
    let file_names: BTreeSet<String> = files.iter().map(|path| file_name_of(path)).collect();
    let mut lab_files: BTreeMap<String, String> = BTreeMap::new();
    let mut current_keys = BTreeSet::new();
    // (ID, version, content hash) of every lab file, current or legacy.
    let mut held: BTreeSet<(String, String, String)> = BTreeSet::new();
    let mut hold = |profile: &AcquisitionProfile| {
        if let Ok(hash) = profile.computed_hash() {
            held.insert((
                profile.profile_id.clone(),
                profile.profile_version.clone(),
                hash,
            ));
        }
    };
    for path in &files {
        let name = file_name_of(path);
        let text = match read_text_file(path) {
            Ok(text) => text,
            Err(error) => {
                report.problems.push(format!("{name}: {error}"));
                continue;
            }
        };
        match compiled
            .iter()
            .find(|(compiled_name, _)| *compiled_name == name)
        {
            None => report.problems.push(format!(
                "{name}: not compiled into the application; add `{}` to FACTORY_LAB_FILES in src-tauri/src/profiles.rs",
                factory_lab_source_line(&name)
            )),
            Some((_, compiled_text)) if normalized_text(compiled_text) != normalized_text(&text) => {
                report.problems.push(format!(
                    "{name}: the compiled-in copy differs from the file; rebuild and check again"
                ))
            }
            Some(_) => {}
        }
        let (profile, problems) = check_factory_lab_text(&text, &lock, true);
        report.problems.extend(
            problems
                .into_iter()
                .map(|problem| format!("{name}: {problem}")),
        );
        let Some(profile) = profile else { continue };
        hold(&profile);
        if let Some(other) = lab_files.insert(profile.profile_id.clone(), name.clone()) {
            report.problems.push(format!(
                "{name}: lab ID {} is also used by {other}; a factory lab has exactly one current file",
                profile.profile_id
            ));
        }
        current_keys.insert(pkey(&profile));
        report.labs.push(format!(
            "{name}: {} {} {}",
            profile.profile_id, profile.profile_version, profile.integrity.canonical_hash
        ));
    }
    for (name, _) in compiled {
        if !file_names.contains(*name) {
            report.problems.push(format!(
                "{name}: compiled into the application but missing from {}",
                profiles_dir.display()
            ));
        }
    }
    let legacy_dir = profiles_dir.join(FACTORY_LAB_LEGACY_FOLDER);
    if legacy_dir.is_dir() {
        let legacy_files = factory_lab_files_in(&legacy_dir).unwrap_or_else(|error| {
            report
                .problems
                .push(format!("{}: {error}", legacy_dir.display()));
            Vec::new()
        });
        for path in legacy_files {
            let name = format!("{FACTORY_LAB_LEGACY_FOLDER}/{}", file_name_of(&path));
            let (profile, problems) = match read_text_file(&path) {
                Ok(text) => check_factory_lab_text(&text, &lock, false),
                Err(error) => (None, vec![error.to_string()]),
            };
            report.problems.extend(
                problems
                    .into_iter()
                    .map(|problem| format!("{name}: {problem}")),
            );
            let Some(profile) = profile else { continue };
            hold(&profile);
            if current_keys.contains(&pkey(&profile)) {
                report.problems.push(format!(
                    "{name}: {} {} is also a current factory lab file; a legacy file holds an earlier version",
                    profile.profile_id, profile.profile_version
                ));
            }
            report.labs.push(format!(
                "{name}: {} {} {}",
                profile.profile_id, profile.profile_version, profile.integrity.canonical_hash
            ));
        }
    }
    // A published version stays available as a file, so it can always be
    // checked and rebuilt (the Rust tests read the v1.0.2 labs from these
    // files to guard the canonical serialization).
    for entry in &lock {
        let key = (
            entry.profile_id.clone(),
            entry.profile_version.clone(),
            entry.canonical_hash.clone(),
        );
        if !held.contains(&key) {
            report.problems.push(format!(
                "{FACTORY_LAB_LOCK_FILE}: {} {} is published, but no file in profiles/ or profiles/{FACTORY_LAB_LEGACY_FOLDER}/ holds it with hash {}; keep the published file as profiles/{FACTORY_LAB_LEGACY_FOLDER}/{} (`fix` and `promote` do this, from the copy committed in Git when the file has already changed)",
                entry.profile_id,
                entry.profile_version,
                entry.canonical_hash,
                legacy_factory_lab_file_name(
                    lab_files
                        .get(&entry.profile_id)
                        .map(String::as_str)
                        .unwrap_or("<file>.profile.json"),
                    &entry.profile_version
                )
            ));
        }
    }
    report
}

/// Before the current factory lab file of `profile_id` (`file_name` in
/// `profiles_dir`) is rewritten to hold `new_version`: every other published
/// version of the lab (a lock entry) must stay available as a file in
/// `profiles/legacy/`. A version no legacy file holds yet is taken from the
/// current file as it is now (`current_text`) or, when that has already
/// changed, from the copy committed in Git (`committed`, called with the
/// file name only when needed), if it holds the version with its published
/// hash. Returns the legacy files to write; refuses (before anything is
/// written) when a published version would be lost, or when a different
/// file already has the legacy name.
fn plan_kept_factory_labs(
    profiles_dir: &Path,
    lock: &[FactoryLabLockEntry],
    profile_id: &str,
    new_version: &str,
    file_name: &str,
    current_text: Option<&str>,
    committed: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<(PathBuf, String)>, ProfileError> {
    let holds = |text: &str, entry: &FactoryLabLockEntry| {
        parse_lab_text(text).is_ok_and(|profile| {
            pkey(&profile) == entry.key()
                && profile
                    .computed_hash()
                    .is_ok_and(|hash| hash == entry.canonical_hash)
        })
    };
    let legacy_dir = profiles_dir.join(FACTORY_LAB_LEGACY_FOLDER);
    let legacy_texts: Vec<String> = if legacy_dir.is_dir() {
        factory_lab_files_in(&legacy_dir)
            .map_err(|source| ProfileError::Read {
                path: legacy_dir.display().to_string(),
                source,
            })?
            .iter()
            .filter_map(|path| read_text_file(path).ok())
            .collect()
    } else {
        Vec::new()
    };
    let mut committed_text: Option<Option<String>> = None;
    let mut kept = Vec::new();
    let mut missing = Vec::new();
    for entry in lock
        .iter()
        .filter(|entry| entry.profile_id == profile_id && entry.profile_version != new_version)
    {
        if legacy_texts.iter().any(|text| holds(text, entry)) {
            continue;
        }
        let source = match current_text.filter(|text| holds(text, entry)) {
            Some(text) => Some(text.to_string()),
            None => committed_text
                .get_or_insert_with(|| committed(file_name))
                .clone()
                .filter(|text| holds(text, entry)),
        };
        let Some(text) = source else {
            missing.push(entry.profile_version.clone());
            continue;
        };
        let destination = legacy_dir.join(legacy_factory_lab_file_name(
            file_name,
            &entry.profile_version,
        ));
        if destination.exists() {
            return Err(ProfileError::Validation(format!(
                "{} already exists but does not hold the published {profile_id} {}; move it aside and run again",
                destination.display(),
                entry.profile_version
            )));
        }
        kept.push((destination, text));
    }
    if !missing.is_empty() {
        return Err(ProfileError::Validation(format!(
            "{profile_id} {} is published (recorded in {FACTORY_LAB_LOCK_FILE}), but neither profiles/{FACTORY_LAB_LEGACY_FOLDER}/ nor {file_name} nor its copy committed in Git holds it, so replacing {file_name} would lose it. Put the published file in profiles/{FACTORY_LAB_LEGACY_FOLDER}/ (for example from an earlier commit) and run again; nothing was changed",
            missing.join(", ")
        )));
    }
    Ok(kept)
}

/// Writes the legacy files `plan_kept_factory_labs` planned.
fn write_kept_factory_labs(kept: &[(PathBuf, String)]) -> Result<Vec<PathBuf>, ProfileError> {
    for (path, text) in kept {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| ProfileError::Write {
                path: parent.display().to_string(),
                source,
            })?;
        }
        write_text_file(path, text)?;
    }
    Ok(kept.iter().map(|(path, _)| path.clone()).collect())
}

/// Re-hashes a factory lab file after a hand edit and records the version
/// in the lock. A version already in the lock never changes: different
/// content there is refused unless `bump_minor` is set, which publishes it
/// as the next MINOR version (MAJOR.MINOR+1.0) and appends that to the lock.
/// The file keeps its layout; only the version and hash values change.
///
/// When the file no longer holds a published version of its lab (it was
/// edited, or a "Save as course default file…" file was copied over it),
/// that version is kept in `profiles/legacy/` first: from the copy committed
/// in Git (`committed`, called with the file name), since the file itself has
/// already changed. See `plan_kept_factory_labs`.
pub fn rehash_factory_file(
    path: &Path,
    lock_path: &Path,
    bump_minor: bool,
    committed: &dyn Fn(&str) -> Option<String>,
) -> Result<FactoryFixOutcome, ProfileError> {
    let text = read_text_file(path)?;
    let mut profile = parse_lab_text(&text)?;
    if profile.source != ProfileSource::BuiltIn || profile.status != ProfileStatus::Locked {
        return Err(ProfileError::Validation(format!(
            "{} is not a factory lab file (it needs \"source\": \"built_in\" and \"status\": \"locked\"); use promote to turn a Lab Manager export into one",
            path.display()
        )));
    }
    let lock_text = read_text_file(lock_path)?;
    let lock = parse_factory_lab_lock(&lock_text)?;
    refuse_lock_problems(&lock)?;
    let previous_version = profile.profile_version.clone();
    let content_hash = profile.computed_hash()?;
    let newest = newest_locked_version(&lock, &profile.profile_id);
    let new_version_needed = match lock.iter().find(|entry| entry.key() == pkey(&profile)) {
        Some(published) if published.canonical_hash != content_hash => Some(format!(
            "{} {previous_version} is already published with hash {}, and a published factory lab never changes",
            profile.profile_id, published.canonical_hash
        )),
        Some(_) => None,
        None if !is_factory_lab_version(&previous_version) => Some(format!(
            "a factory lab version is MAJOR.MINOR.0, not {previous_version}"
        )),
        None if newest.is_some_and(|newest| version_key(newest) > version_key(&previous_version)) => {
            Some(format!(
                "{} {} is already published, and a factory lab never goes back to an older version",
                profile.profile_id,
                newest.unwrap_or_default()
            ))
        }
        None => None,
    };
    if let Some(reason) = new_version_needed {
        let next = next_factory_lab_version(&lock, &profile.profile_id, &[&previous_version]);
        if !bump_minor {
            return Err(ProfileError::Validation(format!(
                "{reason}. Re-run with --bump-minor to publish the change as {next}"
            )));
        }
        profile.profile_version = next;
    }
    profile.refresh_hash()?;
    profile.validate_for_authoring()?;
    let entry = FactoryLabLockEntry::of(&profile);
    let lock_appended = match lock.iter().find(|existing| existing.key() == entry.key()) {
        Some(existing) if existing.canonical_hash != entry.canonical_hash => {
            return Err(ProfileError::Validation(format!(
                "{} {} is already published with a different hash",
                entry.profile_id, entry.profile_version
            )))
        }
        Some(_) => false,
        None => true,
    };
    let new_text = rewrite_factory_lab_text(&text, &profile)?;
    let file_changed = new_text != text;
    let profiles_dir = path
        .parent()
        .map(|parent| {
            if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            }
        })
        .unwrap_or(Path::new("."));
    let kept = plan_kept_factory_labs(
        profiles_dir,
        &lock,
        &profile.profile_id,
        &profile.profile_version,
        &file_name_of(path),
        Some(&text),
        committed,
    )?;
    let kept_files = write_kept_factory_labs(&kept)?;
    if file_changed {
        write_text_file(path, &new_text)?;
    }
    if lock_appended {
        append_factory_lab_lock(lock_path, &lock_text, &lock, entry)?;
    }
    Ok(FactoryFixOutcome {
        profile_id: profile.profile_id.clone(),
        previous_version,
        profile_version: profile.profile_version.clone(),
        canonical_hash: profile.integrity.canonical_hash.clone(),
        file_changed,
        lock_appended,
        kept_files,
    })
}

/// Turns a lab exported from Lab Manager into a factory lab file. The lab
/// gets source `built_in`, status `locked`, the editor's Save normalization,
/// and the next factory version (`next_factory_lab_version`): one MINOR above
/// the lab's published versions and the export's own version, so a lab that
/// already has a factory version goes from X.Y.0 to X.(Y+1).0 and a new lab
/// ID exported as 1.0.x becomes 1.1.0. The file is
/// `profiles/<file_stem>.profile.json`: the lab's current factory file, or
/// one named after the display name. The published version the file held is
/// kept in `profiles/legacy/` first: from the file itself, or from the copy
/// committed in Git (`committed`, called with the file name) when the file
/// has changed since (`plan_kept_factory_labs`). The new version is appended
/// to the lock. Compiling a new file in (`FACTORY_LAB_FILES`) is left to the
/// maintainer.
pub fn promote_lab_file(
    exported_path: &Path,
    profiles_dir: &Path,
    lock_path: &Path,
    file_stem: Option<&str>,
    committed: &dyn Fn(&str) -> Option<String>,
) -> Result<FactoryPromoteOutcome, ProfileError> {
    let mut profile = parse_lab_text(&read_text_file(exported_path)?)?;
    if profile.status != ProfileStatus::Locked {
        return Err(ProfileError::Validation(
            "only a saved (locked) lab exported from Lab Manager can be promoted".into(),
        ));
    }
    // The export's own hash detects a damaged or hand-edited file.
    profile.validate()?;
    let lock_text = read_text_file(lock_path)?;
    let lock = parse_factory_lab_lock(&lock_text)?;
    refuse_lock_problems(&lock)?;
    // The lab's current factory file: name, text, and lab.
    let mut current: Option<(String, String, AcquisitionProfile)> = None;
    for path in factory_lab_files_in(profiles_dir).map_err(|source| ProfileError::Read {
        path: profiles_dir.display().to_string(),
        source,
    })? {
        let text = read_text_file(&path)?;
        let existing = parse_lab_text(&text)?;
        if existing.profile_id == profile.profile_id {
            current = Some((file_name_of(&path), text, existing));
            break;
        }
    }
    let file_name = match (file_stem.map(factory_lab_file_name).transpose()?, &current) {
        (Some(requested), Some((existing_name, _, _))) if requested != *existing_name => {
            return Err(ProfileError::Validation(format!(
                "{} already has the factory lab file {existing_name}; promote without a file name to replace it",
                profile.profile_id
            )))
        }
        (Some(requested), _) => requested,
        (None, Some((existing_name, _, _))) => existing_name.clone(),
        (None, None) => format!(
            "{}{FACTORY_LAB_FILE_SUFFIX}",
            factory_lab_file_stem(&profile.display_name)
        ),
    };
    let destination = profiles_dir.join(&file_name);
    if current.is_none() && destination.exists() {
        return Err(ProfileError::Validation(format!(
            "{file_name} already exists for another lab; choose another file name"
        )));
    }
    let replaced_version = current
        .as_ref()
        .map(|(_, _, existing)| existing.profile_version.clone());
    // The export's own version counts too, so a new lab ID's first factory
    // version is above its local 1.0.x revisions.
    let mut known = vec![profile.profile_version.as_str()];
    known.extend(replaced_version.as_deref());
    let version = next_factory_lab_version(&lock, &profile.profile_id, &known);
    make_factory_lab(&mut profile, version)?;
    if let Some((_, _, existing)) = &current {
        let mut same_version = profile.clone();
        same_version.profile_version = existing.profile_version.clone();
        if same_lab_content(&same_version, existing)? {
            return Err(ProfileError::Validation(format!(
                "the export has the same content as the current course default {} {}; nothing to promote",
                existing.display_name, existing.profile_version
            )));
        }
    }
    let entry = FactoryLabLockEntry::of(&profile);
    if lock.iter().any(|existing| existing.key() == entry.key()) {
        return Err(ProfileError::Validation(format!(
            "{} {} is already published",
            entry.profile_id, entry.profile_version
        )));
    }
    let line_ending = match &current {
        Some((_, existing_text, _)) => line_ending_of(existing_text),
        None => line_ending_of(&lock_text),
    };
    let text = factory_lab_file_text(&profile, line_ending)?;
    if parse_lab_text(&text)? != profile {
        return Err(ProfileError::Validation(
            "the factory lab file did not read back unchanged; nothing was written".into(),
        ));
    }
    let kept = plan_kept_factory_labs(
        profiles_dir,
        &lock,
        &profile.profile_id,
        &profile.profile_version,
        &file_name,
        current
            .as_ref()
            .map(|(_, existing_text, _)| existing_text.as_str()),
        committed,
    )?;
    let kept_files = write_kept_factory_labs(&kept)?;
    write_text_file(&destination, &text)?;
    append_factory_lab_lock(lock_path, &lock_text, &lock, entry)?;
    Ok(FactoryPromoteOutcome {
        path: destination,
        file_name,
        profile_id: profile.profile_id.clone(),
        profile_version: profile.profile_version.clone(),
        canonical_hash: profile.integrity.canonical_hash.clone(),
        replaced_version,
        kept_files,
    })
}

/// Turns a saved lab into a factory lab at `version`: the editor's Save
/// normalization (explicit outputs, no inert pulse-ox channels), source
/// `built_in`, status `locked`, a fresh hash, and the rules for a lab saved
/// now.
fn make_factory_lab(profile: &mut AcquisitionProfile, version: String) -> Result<(), ProfileError> {
    normalize_authoring_draft(profile);
    profile.profile_version = version;
    profile.source = ProfileSource::BuiltIn;
    profile.status = ProfileStatus::Locked;
    profile.refresh_hash()?;
    profile.validate_for_authoring()
}

fn read_text_file(path: &Path) -> Result<String, ProfileError> {
    fs::read_to_string(path).map_err(|source| ProfileError::Read {
        path: path.display().to_string(),
        source,
    })
}

fn write_text_file(path: &Path, text: &str) -> Result<(), ProfileError> {
    write_file_atomically(path, text.as_bytes()).map_err(|source| ProfileError::Write {
        path: path.display().to_string(),
        source,
    })
}

fn parse_lab_text(text: &str) -> Result<AcquisitionProfile, ProfileError> {
    Ok(serde_json::from_str(
        text.strip_prefix('\u{feff}').unwrap_or(text),
    )?)
}

/// Text compared without a byte-order mark or Windows line endings (a
/// checkout may convert them).
fn normalized_text(text: &str) -> String {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n")
}

fn line_ending_of(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
}

/// The `*.profile.json` files directly in `dir`, sorted by name.
fn factory_lab_files_in(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && file_name_of(path).ends_with(FACTORY_LAB_FILE_SUFFIX))
        .collect();
    files.sort();
    Ok(files)
}

fn refuse_lock_problems(lock: &[FactoryLabLockEntry]) -> Result<(), ProfileError> {
    let problems = factory_lab_lock_problems(lock);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(ProfileError::Validation(format!(
            "{FACTORY_LAB_LOCK_FILE} needs repair first: {}",
            problems.join("; ")
        )))
    }
}

/// One factory lab file's own problems: the factory rules, its hash, the
/// validation rules (for a current file, also those for a lab saved now),
/// and its lock entry.
fn check_factory_lab_text(
    text: &str,
    lock: &[FactoryLabLockEntry],
    current: bool,
) -> (Option<AcquisitionProfile>, Vec<String>) {
    let profile = match parse_lab_text(text) {
        Ok(profile) => profile,
        Err(error) => return (None, vec![error.to_string()]),
    };
    let mut problems = Vec::new();
    if profile.status != ProfileStatus::Locked {
        problems.push("status must be \"locked\"".to_string());
    }
    if profile.source != ProfileSource::BuiltIn {
        problems.push("source must be \"built_in\"".to_string());
    }
    if !is_factory_lab_version(&profile.profile_version) {
        problems.push(format!(
            "version {} is not MAJOR.MINOR.0 (factory lab version policy)",
            profile.profile_version
        ));
    }
    match profile.verify_integrity() {
        Err(error) => problems.push(format!(
            "{error}; after a hand edit run `scripts/update_factory_labs.ps1 fix <file>`"
        )),
        Ok(()) => {
            let validation = if current {
                profile.validate_for_authoring()
            } else {
                profile.validate()
            };
            if let Err(error) = validation {
                problems.push(error.to_string());
            }
        }
    }
    let content_hash = profile.computed_hash().unwrap_or_default();
    match lock.iter().find(|entry| entry.key() == pkey(&profile)) {
        None => problems.push(format!(
            "{} {} is not recorded in {FACTORY_LAB_LOCK_FILE}; run `scripts/update_factory_labs.ps1 fix <file>`",
            profile.profile_id, profile.profile_version
        )),
        Some(entry) if entry.canonical_hash != content_hash => problems.push(format!(
            "{} {} was published with hash {}, but the file's content hashes to {content_hash}; a published factory lab never changes, so run `scripts/update_factory_labs.ps1 fix <file> --bump-minor`",
            profile.profile_id, profile.profile_version, entry.canonical_hash
        )),
        Some(_) => {}
    }
    if current {
        if let Some(newest) = newest_locked_version(lock, &profile.profile_id) {
            if version_key(newest) > version_key(&profile.profile_version) {
                problems.push(format!(
                    "{} {newest} is already published; a factory lab never goes back to an older version (publish earlier content again as a new MINOR version)",
                    profile.profile_id
                ));
            }
        }
    }
    (Some(profile), problems)
}

fn newest_locked_version<'a>(lock: &'a [FactoryLabLockEntry], profile_id: &str) -> Option<&'a str> {
    lock.iter()
        .filter(|entry| entry.profile_id == profile_id)
        .max_by_key(|entry| version_key(&entry.profile_version))
        .map(|entry| entry.profile_version.as_str())
}

/// The next factory version of a lab: one MINOR above the highest MINOR of
/// the highest MAJOR among its published versions and the `known` versions
/// (its current factory version, and the local revision a new factory
/// version is made from), with PATCH 0; 1.0.0 for a lab with no version.
///
/// Counting the revision keeps policy P2's promise for a new lab ID too:
/// its local revisions are 1.0.0 (the first Save) and 1.0.x, so its first
/// factory version is 1.1.0 and never has the ID and version of one of them.
fn next_factory_lab_version(
    lock: &[FactoryLabLockEntry],
    profile_id: &str,
    known: &[&str],
) -> String {
    let known: Vec<(u64, u64, u64)> = lock
        .iter()
        .filter(|entry| entry.profile_id == profile_id)
        .filter_map(|entry| version_key(&entry.profile_version))
        .chain(known.iter().filter_map(|version| version_key(version)))
        .collect();
    let Some(major) = known.iter().map(|(major, _, _)| *major).max() else {
        return "1.0.0".into();
    };
    let minor = known
        .iter()
        .filter(|(known_major, _, _)| *known_major == major)
        .map(|(_, minor, _)| *minor)
        .max()
        .unwrap_or_default();
    format!("{major}.{}.0", minor.saturating_add(1))
}

/// A file name for a new factory lab: its display name in lowercase ASCII
/// words joined by underscores ("Op Amp — Course Capture" gives
/// `op_amp_course_capture`).
fn factory_lab_file_stem(display_name: &str) -> String {
    let mut stem = String::new();
    for character in display_name.chars() {
        if character.is_ascii_alphanumeric() {
            stem.push(character.to_ascii_lowercase());
        } else if !stem.is_empty() && !stem.ends_with('_') {
            stem.push('_');
        }
    }
    let stem: String = stem.trim_end_matches('_').chars().take(64).collect();
    let stem = stem.trim_end_matches('_');
    if stem.is_empty() {
        "lab".into()
    } else {
        stem.into()
    }
}

/// A file name the maintainer chose: 1-64 lowercase letters, digits, `_`, or
/// `-`, with or without the `.profile.json` suffix.
fn factory_lab_file_name(stem: &str) -> Result<String, ProfileError> {
    let stem = stem.strip_suffix(FACTORY_LAB_FILE_SUFFIX).unwrap_or(stem);
    if (1..=64).contains(&stem.len())
        && stem.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '-')
        })
    {
        Ok(format!("{stem}{FACTORY_LAB_FILE_SUFFIX}"))
    } else {
        Err(ProfileError::Validation(format!(
            "file name \"{stem}\" must be 1-64 lowercase letters, digits, underscores, or dashes"
        )))
    }
}

/// The re-hashed file text: only the version and hash values are replaced,
/// so a hand-written layout stays as it is. When those values cannot be
/// found exactly once, the whole file is written in the factory layout.
fn rewrite_factory_lab_text(
    original: &str,
    profile: &AcquisitionProfile,
) -> Result<String, ProfileError> {
    let replacements = [
        ("profile_version", profile.profile_version.as_str()),
        (
            "canonical_hash_algorithm",
            profile.integrity.canonical_hash_algorithm.as_str(),
        ),
        ("canonical_hash", profile.integrity.canonical_hash.as_str()),
    ];
    let mut text = Some(original.to_string());
    for (key, value) in replacements {
        text = text.and_then(|current| replace_json_string_value(&current, key, value));
    }
    if let Some(text) = text {
        if parse_lab_text(&text).is_ok_and(|parsed| parsed == *profile) {
            return Ok(text);
        }
    }
    factory_lab_file_text(profile, line_ending_of(original))
}

/// Replaces the string value of `"key": "..."`, which must occur exactly
/// once and hold no escapes.
fn replace_json_string_value(text: &str, key: &str, value: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut found = None;
    for (start, _) in text.match_indices(&needle) {
        let Some(after_colon) = text[start + needle.len()..].trim_start().strip_prefix(':') else {
            continue;
        };
        let Some(inner) = after_colon.trim_start().strip_prefix('"') else {
            continue;
        };
        let end = inner.find('"')?;
        if inner[..end].contains('\\') {
            return None;
        }
        let value_start = text.len() - inner.len();
        if found.replace((value_start, value_start + end)).is_some() {
            return None;
        }
    }
    let (from, to) = found?;
    Some(format!("{}{value}{}", &text[..from], &text[to..]))
}

/// Appends one entry to the lock, keeping every existing line byte for byte.
fn append_factory_lab_lock(
    lock_path: &Path,
    lock_text: &str,
    lock: &[FactoryLabLockEntry],
    entry: FactoryLabLockEntry,
) -> Result<(), ProfileError> {
    let line_ending = line_ending_of(lock_text);
    let mut expected = lock.to_vec();
    expected.push(entry.clone());
    let line = factory_json(&entry, FactoryJsonLayout::Inline, line_ending)?;
    let appended = lock_text.rfind(']').map(|close| {
        let head = lock_text[..close].trim_end();
        let separator = if head.ends_with('[') { "" } else { "," };
        format!("{head}{separator}{line_ending}  {line}{line_ending}]{line_ending}")
    });
    let text = match appended {
        Some(text) if parse_factory_lab_lock(&text).is_ok_and(|parsed| parsed == expected) => text,
        _ => factory_lab_lock_text(&expected, line_ending)?,
    };
    write_text_file(lock_path, &text)
}

#[derive(Clone, Copy, PartialEq)]
enum FactoryJsonLayout {
    /// The factory lab file layout (see `factory_lab_file_text`).
    LabFile,
    /// One line: `{ "key": value, ... }` and `[a, b]`.
    Inline,
}

fn factory_json<T: Serialize>(
    value: &T,
    layout: FactoryJsonLayout,
    line_ending: &'static str,
) -> Result<String, ProfileError> {
    let mut bytes = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(
        &mut bytes,
        FactoryJsonFormatter {
            layout,
            line_ending,
            frames: Vec::new(),
            capturing_key: false,
        },
    );
    value.serialize(&mut serializer)?;
    String::from_utf8(bytes)
        .map_err(|_| ProfileError::Validation("lab JSON is not valid UTF-8".into()))
}

/// A serde_json formatter for the factory file layout. Each open object or
/// array decides once whether it is written one entry per line.
struct FactoryJsonFormatter {
    layout: FactoryJsonLayout,
    line_ending: &'static str,
    frames: Vec<FactoryJsonFrame>,
    capturing_key: bool,
}

struct FactoryJsonFrame {
    multiline: bool,
    has_values: bool,
    /// The object's current key (empty for an array).
    key: String,
}

impl FactoryJsonFormatter {
    fn opens_multiline(&self, is_array: bool) -> bool {
        if self.layout == FactoryJsonLayout::Inline {
            return false;
        }
        let key = |depth: usize| self.frames.get(depth).map(|frame| frame.key.as_str());
        match (self.frames.len(), is_array) {
            (0, false) => true,
            (1, false) => key(0) == Some("acquisition"),
            (2, true) => key(0) == Some("acquisition") && key(1) == Some("channels"),
            _ => false,
        }
    }
    fn new_line<W: ?Sized + std::io::Write>(
        &self,
        writer: &mut W,
        depth: usize,
    ) -> std::io::Result<()> {
        writer.write_all(self.line_ending.as_bytes())?;
        writer.write_all("  ".repeat(depth).as_bytes())
    }
    fn open<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        is_array: bool,
    ) -> std::io::Result<()> {
        let multiline = self.opens_multiline(is_array);
        self.frames.push(FactoryJsonFrame {
            multiline,
            has_values: false,
            key: String::new(),
        });
        writer.write_all(if is_array { b"[" } else { b"{" })
    }
    fn close<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        is_array: bool,
    ) -> std::io::Result<()> {
        if let Some(frame) = self.frames.pop() {
            if frame.has_values && frame.multiline {
                self.new_line(writer, self.frames.len())?;
            } else if frame.has_values && !is_array {
                writer.write_all(b" ")?;
            }
        }
        writer.write_all(if is_array { b"]" } else { b"}" })
    }
    fn begin_entry<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
        is_object: bool,
    ) -> std::io::Result<()> {
        let depth = self.frames.len();
        let Some(frame) = self.frames.last_mut() else {
            return Ok(());
        };
        frame.has_values = true;
        frame.key.clear();
        let multiline = frame.multiline;
        if !first {
            writer.write_all(b",")?;
        }
        if multiline {
            self.new_line(writer, depth)
        } else if is_object || !first {
            writer.write_all(b" ")
        } else {
            Ok(())
        }
    }
}

impl serde_json::ser::Formatter for FactoryJsonFormatter {
    fn begin_array<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.open(writer, true)
    }
    fn end_array<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.close(writer, true)
    }
    fn begin_array_value<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        self.begin_entry(writer, first, false)
    }
    fn end_array_value<W: ?Sized + std::io::Write>(
        &mut self,
        _writer: &mut W,
    ) -> std::io::Result<()> {
        Ok(())
    }
    fn begin_object<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.open(writer, false)
    }
    fn end_object<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.close(writer, false)
    }
    fn begin_object_key<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        self.begin_entry(writer, first, true)?;
        self.capturing_key = true;
        Ok(())
    }
    fn end_object_key<W: ?Sized + std::io::Write>(
        &mut self,
        _writer: &mut W,
    ) -> std::io::Result<()> {
        self.capturing_key = false;
        Ok(())
    }
    fn begin_object_value<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
    ) -> std::io::Result<()> {
        writer.write_all(b": ")
    }
    fn end_object_value<W: ?Sized + std::io::Write>(
        &mut self,
        _writer: &mut W,
    ) -> std::io::Result<()> {
        Ok(())
    }
    fn write_string_fragment<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        fragment: &str,
    ) -> std::io::Result<()> {
        if self.capturing_key {
            if let Some(frame) = self.frames.last_mut() {
                frame.key.push_str(fragment);
            }
        }
        writer.write_all(fragment.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn labs_keep_every_earlier_firmware_build_after_the_reference_build_moves_on() {
        let reference = crate::protocol::REFERENCE_FIRMWARE_BUILD;
        assert!(ACCEPTED_LAB_FIRMWARE_BUILDS.contains(&reference));
        assert_eq!(reference, 0x0001_0007);
        // Without these, every factory lab (0x00010006) and every older local
        // lab would fail validation and disappear from the catalog.
        for build in 0x0001_0001..reference {
            assert!(
                ACCEPTED_LAB_FIRMWARE_BUILDS.contains(&build),
                "{build:#010x}"
            );
        }
        let base = built_in_profiles()
            .unwrap_or_else(|error| panic!("{error}"))
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("no factory lab"));
        for build in ACCEPTED_LAB_FIRMWARE_BUILDS {
            let mut lab = base.clone();
            lab.required_firmware.build = format!("0x{build:08X}");
            lab.refresh_hash()
                .unwrap_or_else(|error| panic!("{build:#010x}: {error}"));
            lab.validate()
                .unwrap_or_else(|error| panic!("{build:#010x}: {error}"));
        }
        for build in [0x0001_0000u32, 0x0001_0008, 0x0002_0001] {
            let mut lab = base.clone();
            lab.required_firmware.build = format!("0x{build:08X}");
            lab.refresh_hash()
                .unwrap_or_else(|error| panic!("{build:#010x}: {error}"));
            let error = lab
                .validate()
                .err()
                .unwrap_or_else(|| panic!("{build:#010x} was accepted"));
            assert!(
                error
                    .to_string()
                    .contains("incompatible controlled firmware identity"),
                "{build:#010x}: {error}"
            );
        }
        // The factory labs still name 0x00010006, so their hashes are
        // unchanged; only the LED-sequenced pulse-ox lab 1.1.0 needs the
        // build that added its mode.
        for profile in built_in_profiles().unwrap_or_else(|error| panic!("{error}")) {
            let expected = if profile.acquisition.is_led_sequenced() {
                format!("0x{SEQUENCED_MIN_FIRMWARE_BUILD:08X}")
            } else {
                "0x00010006".to_string()
            };
            assert_eq!(
                profile.required_firmware.build, expected,
                "{}",
                profile.profile_id
            );
        }
        assert_eq!(factory_lab(PULSE_ID).required_firmware.build, "0x00010007");
    }

    #[test]
    fn builtin_hashes_are_deterministic() {
        let profiles = built_in_profiles().unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(profiles.len(), factory_lab_count());
        assert!(!profiles.is_empty());
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
        assert_eq!(store.list().unwrap_or_default().len(), factory_lab_count());
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
        assert_eq!(
            reopened.list().unwrap_or_default().len(),
            factory_lab_count()
        );
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
        assert_eq!(entries.len(), factory_lab_count());
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
        // Pulse ox 1.1.0: one record per LED state with every channel.
        let pulseox = lookup("course_pulseox");
        assert_eq!(
            pulseox.acquisition.acquisition_mode,
            AcquisitionMode::PulseoxSequenced
        );
        assert_eq!(pulseox.acquisition.analog_pins(), ["A0", "A1"]);
        assert_eq!(
            pulseox.acquisition.record_field_names(),
            ["led_state", "tx_counts", "rx_counts"]
        );
        assert_eq!(pulseox.acquisition.state_dwell_us, Some(1_000));
        assert_eq!(pulseox.acquisition.sample_rate_hz, 1_000);
        assert_eq!(pulseox.acquisition.adc_resolution_bits, 14);
        // Its published 1.0.0 (legacy 4-state): one 8-field record per cycle.
        let four_state = legacy_pulse_ox_lab();
        assert_eq!(four_state.acquisition.analog_pins(), ["A0", "A1"]);
        assert_eq!(four_state.acquisition.record_field_names().len(), 8);
        assert_eq!(four_state.acquisition.state_dwell_us, Some(1_000));
        assert_eq!(four_state.acquisition.sample_rate_hz, 250);
        assert_eq!(four_state.acquisition.adc_resolution_bits, 14);
        let general = lookup("development");
        assert_eq!(general.acquisition.adc_resolution_bits, 14);
        let op_amp = lookup("course_op_amp");
        assert_eq!(op_amp.acquisition.adc_resolution_bits, 14);
        assert_eq!(op_amp.acquisition.analog_pins(), ["A0", "A1"]);
        assert_eq!(
            op_amp.acquisition.record_field_names(),
            ["vmid_counts", "vout_counts"]
        );
    }

    #[test]
    fn profile_validation_rejects_duplicate_pins_and_bad_pulseox_mapping() {
        let mut profile = factory_lab(EMG_ID);
        profile.acquisition.channels[1].pin = "A0".into();
        profile.integrity.canonical_hash.clear();
        assert!(profile.validate().is_err());
        // Each rule is checked on a correctly rehashed profile, so the
        // assertion cannot pass merely because the hash no longer matches.
        let factory_pulseox = legacy_pulse_ox_lab();
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
        let mut ecg = factory_lab(ECG_ID);
        ecg.acquisition.sample_rate_hz = 300;
        ecg.refresh_hash().unwrap_or_else(|error| panic!("{error}"));
        assert!(ecg.validate().is_ok());
        assert!(ecg.validate_for_authoring().is_err());
        assert!(ecg.acquisition.firmware_configuration_issue().is_some());
    }
    /// `lab` after `change`, rehashed, so a refusal cannot come from a stale hash.
    fn rehashed(
        lab: &AcquisitionProfile,
        change: impl FnOnce(&mut AcquisitionProfile),
    ) -> AcquisitionProfile {
        let mut copy = lab.clone();
        change(&mut copy);
        copy.refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        copy
    }

    #[test]
    fn led_sequenced_labs_record_the_led_state_and_every_channel() {
        let lab = sequenced_test_lab();
        assert!(lab.acquisition.is_led_sequenced());
        assert!(lab.validate_for_authoring().is_ok());
        assert_eq!(
            lab.acquisition.record_field_names(),
            ["led_state", "tx_counts", "rx_counts"]
        );
        assert_eq!(lab.acquisition.analog_pins(), ["A0", "A1"]);
        assert_eq!(lab.acquisition.firmware_configuration_issue(), None);
        let json = serde_json::to_value(&lab).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(json["acquisition"]["acquisition_mode"], "pulseox_sequenced");
        assert_eq!(
            json["acquisition"]["channels"][0]["split_by_led_state"],
            true
        );
        let parsed: AcquisitionProfile =
            serde_json::from_value(json).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(parsed, lab);
        parsed.validate().unwrap_or_else(|error| panic!("{error}"));

        // An unsplit channel omits the flag: existing labs keep their bytes.
        // The only LED-sequenced factory lab is pulse ox 1.1.0.
        let mut sequenced = Vec::new();
        for profile in built_in_profiles().unwrap_or_else(|error| panic!("{error}")) {
            let bytes = profile
                .canonical_bytes()
                .unwrap_or_else(|error| panic!("{error}"));
            let split = String::from_utf8_lossy(&bytes).contains("split_by_led_state");
            assert_eq!(
                split,
                profile.acquisition.is_led_sequenced(),
                "{}",
                profile.profile_id
            );
            if profile.acquisition.is_led_sequenced() {
                sequenced.push(profile.profile_id.clone());
            }
        }
        assert_eq!(sequenced, [PULSE_ID]);
        let bytes = legacy_pulse_ox_lab()
            .canonical_bytes()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!String::from_utf8_lossy(&bytes).contains("split_by_led_state"));
        assert_eq!(pulse_ox_sequenced_rate_hz(1_000), 1_000);
        assert_eq!(pulse_ox_sequenced_rate_hz(2_500), 400);
        assert_eq!(pulse_ox_sequenced_rate_hz(3_000), 333);
        assert_eq!(pulse_ox_sequenced_rate_hz(5_000), 200);
        assert_eq!(
            [0, 1, 2, 3, 4, 9].map(led_state_name),
            ["RED", "DARK1", "IR", "DARK2", "UNKNOWN", "UNKNOWN"]
        );
    }

    #[test]
    fn led_sequenced_labs_refuse_bad_channels_dwell_rate_outputs_and_old_firmware() {
        use DigitalOutputBehavior::{AcquisitionSequenced, AlwaysLow, HighWhileRecording};
        let lab = sequenced_test_lab();
        let refused: Vec<(&str, AcquisitionProfile)> = vec![
            (
                "no channels",
                rehashed(&lab, |lab| lab.acquisition.channels.clear()),
            ),
            (
                "repeated pin",
                rehashed(&lab, |lab| lab.acquisition.channels[1].pin = "A0".into()),
            ),
            (
                "unsupported pin",
                rehashed(&lab, |lab| lab.acquisition.channels[1].pin = "A6".into()),
            ),
            (
                "repeated ID",
                rehashed(&lab, |lab| lab.acquisition.channels[1].id = "tx".into()),
            ),
            (
                "repeated CSV field",
                rehashed(&lab, |lab| {
                    lab.acquisition.channels[1].csv_name = "tx_counts".into();
                }),
            ),
            (
                "not ADC counts",
                rehashed(&lab, |lab| lab.acquisition.channels[0].units = "V".into()),
            ),
            (
                "reserved led_state",
                rehashed(&lab, |lab| {
                    lab.acquisition.channels[0].csv_name = "led_state".into();
                }),
            ),
            (
                "reserved led_state_name",
                rehashed(&lab, |lab| {
                    lab.acquisition.channels[0].csv_name = "LED_State_Name".into();
                }),
            ),
            (
                "reserved cycle_index",
                rehashed(&lab, |lab| {
                    lab.acquisition.channels[0].csv_name = "cycle_index".into();
                }),
            ),
            (
                "_V column collides",
                rehashed(&lab, |lab| {
                    lab.acquisition.channels[0].csv_name = "rx_V".into();
                    lab.acquisition.channels[1].csv_name = "rx".into();
                }),
            ),
            (
                "pressure column collides",
                rehashed(&lab, |lab| {
                    lab.acquisition.channels[0].csv_name = "rx_kPa".into();
                    lab.acquisition.channels[1].allowed_conversions =
                        vec!["counts_volts".into(), "mpxv_pressure".into()];
                }),
            ),
            (
                "invalid CSV field",
                rehashed(&lab, |lab| {
                    lab.acquisition.channels[0].csv_name = "tx counts".into();
                }),
            ),
            (
                "legacy 250 µs dwell",
                rehashed(&lab, |lab| {
                    lab.acquisition.state_dwell_us = Some(250);
                    lab.acquisition.sample_rate_hz = 1_000;
                }),
            ),
            (
                "999 µs dwell",
                rehashed(&lab, |lab| lab.acquisition.state_dwell_us = Some(999)),
            ),
            (
                "5001 µs dwell",
                rehashed(&lab, |lab| {
                    lab.acquisition.state_dwell_us = Some(5_001);
                    lab.acquisition.sample_rate_hz = 200;
                }),
            ),
            (
                "no dwell",
                rehashed(&lab, |lab| lab.acquisition.state_dwell_us = None),
            ),
            (
                "rate not from dwell",
                rehashed(&lab, |lab| lab.acquisition.state_dwell_us = Some(2_000)),
            ),
            (
                "4-state cycle rate",
                rehashed(&lab, |lab| lab.acquisition.sample_rate_hz = 250),
            ),
            (
                "same RED and IR pin",
                rehashed(&lab, |lab| {
                    lab.acquisition.digital_outputs[1].pin = "D5".into()
                }),
            ),
            (
                "no IR output",
                rehashed(&lab, |lab| lab.acquisition.digital_outputs.truncate(1)),
            ),
            (
                "IR not sequenced",
                rehashed(&lab, |lab| {
                    lab.acquisition.digital_outputs[1].behavior = AlwaysLow;
                }),
            ),
            (
                "another output",
                rehashed(&lab, |lab| {
                    lab.acquisition.digital_outputs.push(DigitalOutput {
                        pin: "D4".into(),
                        label: "Green LED".into(),
                        behavior: AlwaysLow,
                        pulse_ox_role: None,
                    });
                }),
            ),
            (
                "two RED roles",
                rehashed(&lab, |lab| {
                    lab.acquisition.digital_outputs[0].pulse_ox_role = Some(PulseOxRole::Red);
                    lab.acquisition.digital_outputs.push(DigitalOutput {
                        pin: "D4".into(),
                        label: "Spare".into(),
                        behavior: AcquisitionSequenced,
                        pulse_ox_role: Some(PulseOxRole::Red),
                    });
                }),
            ),
            (
                "firmware build 0x00010006",
                rehashed(&lab, |lab| {
                    lab.required_firmware.build = "0x00010006".into()
                }),
            ),
            (
                "firmware build 0x00010008",
                rehashed(&lab, |lab| {
                    lab.required_firmware.build = "0x00010008".into()
                }),
            ),
            (
                "plot defaults miss rx",
                rehashed(&lab, |lab| lab.plot_defaults.groups.truncate(1)),
            ),
            (
                "plot defaults name an LED-state trace",
                rehashed(&lab, |lab| {
                    lab.plot_defaults.groups[0].channel_ids = vec!["red_tx".into()];
                }),
            ),
        ];
        for (rule, profile) in refused {
            assert!(profile.verify_integrity().is_ok(), "{rule}");
            assert!(profile.validate().is_err(), "{rule} was accepted");
        }
        // A sequenced lab never drives D5/D6 HIGH while recording.
        let high = rehashed(&lab, |lab| {
            lab.acquisition.digital_outputs[1].behavior = HighWhileRecording;
        });
        assert!(high.validate().is_err());

        // The firmware build is checked before the hash: an old build is
        // reported as such even when the stored hash no longer matches.
        let mut old_build = lab.clone();
        old_build.required_firmware.build = "0x00010006".into();
        let error = old_build
            .validate()
            .err()
            .unwrap_or_else(|| panic!("an old build was accepted"));
        assert!(
            error.to_string().contains("build 0x00010007 or later"),
            "{error}"
        );

        // Accepted variants: 2.5 ms (400 records/s), six channels, no plot
        // defaults, and explicit roles that override the labels.
        for profile in [
            rehashed(&lab, |lab| {
                lab.acquisition.state_dwell_us = Some(2_500);
                lab.acquisition.sample_rate_hz = 400;
            }),
            rehashed(&lab, |lab| {
                lab.acquisition.state_dwell_us = Some(5_000);
                lab.acquisition.sample_rate_hz = 200;
            }),
            rehashed(&lab, |lab| {
                lab.acquisition.channels = ["A5", "A4", "A3", "A2", "A1", "A0"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, pin)| ProfileChannel {
                        pin: pin.into(),
                        id: format!("ch{index}"),
                        label: format!("Channel {index}"),
                        csv_name: format!("ch{index}_counts"),
                        units: "ADC counts".into(),
                        allowed_conversions: Vec::new(),
                        default_display_unit: None,
                        default_visible: true,
                        split_by_led_state: false,
                    })
                    .collect();
                lab.plot_defaults = PlotDefaults::default();
            }),
            rehashed(&lab, |lab| {
                lab.acquisition.digital_outputs[0].label = "660 nm".into();
                lab.acquisition.digital_outputs[0].pulse_ox_role = Some(PulseOxRole::Red);
                lab.acquisition.digital_outputs[1].pin = "D4".into();
                lab.acquisition.digital_outputs[1].label = "940 nm".into();
                lab.acquisition.digital_outputs[1].pulse_ox_role = Some(PulseOxRole::Ir);
            }),
        ] {
            profile
                .validate_for_authoring()
                .unwrap_or_else(|error| panic!("{error}"));
        }

        // A dwell the firmware cannot run is named before a recording.
        let mut short = lab.acquisition.clone();
        short.state_dwell_us = Some(500);
        assert!(short
            .firmware_configuration_issue()
            .is_some_and(|issue| issue.contains("1000-5000")));
    }

    #[test]
    fn show_by_led_state_and_the_new_reserved_columns_apply_by_mode() {
        // The display-only split flag exists only in the LED-sequenced mode.
        let simultaneous = rehashed(&factory_lab(ECG_ID), |lab| {
            lab.acquisition.channels[0].split_by_led_state = true;
        });
        assert!(simultaneous.validate().is_err());
        let four_state = rehashed(&legacy_pulse_ox_lab(), |lab| {
            lab.acquisition.channels.push(ProfileChannel {
                pin: "A0".into(),
                id: "channel_1".into(),
                label: "Channel 1".into(),
                csv_name: "channel_1_counts".into(),
                units: "ADC counts".into(),
                allowed_conversions: Vec::new(),
                default_display_unit: None,
                default_visible: true,
                split_by_led_state: true,
            });
        });
        assert!(four_state.validate().is_err());

        // A stored simultaneous lab whose CSV field is now reserved still
        // loads; only a revision saved now must rename it.
        let stored = rehashed(&factory_lab(ECG_ID), |lab| {
            lab.acquisition.channels[0].csv_name = "led_state".into();
        });
        assert!(stored.validate().is_ok());
        assert!(stored.validate_for_authoring().is_err());
        assert!(RESERVED_CSV_COLUMNS.contains(&"led_state_name"));
    }

    #[test]
    fn led_sequenced_drafts_follow_the_dwell_and_name_the_firmware_build_that_has_the_mode() {
        let mut draft = sequenced_test_lab();
        draft.status = ProfileStatus::Draft;
        draft.required_firmware.build = "0x00010006".into();
        draft.acquisition.state_dwell_us = Some(2_500);
        draft.acquisition.sample_rate_hz = 77;
        draft.acquisition.analog_inputs = Some(PulseOxInputs {
            tx: "A0".into(),
            rx: "A1".into(),
        });
        draft.acquisition.led_outputs = Some(LedOutputs {
            green: None,
            red: Some("D5".into()),
            ir: Some("D6".into()),
        });
        let channels = draft.acquisition.channels.clone();
        normalize_authoring_draft(&mut draft);
        assert_eq!(draft.acquisition.sample_rate_hz, 400);
        assert_eq!(draft.required_firmware.build, "0x00010007");
        assert_eq!(draft.acquisition.analog_inputs, None);
        assert_eq!(draft.acquisition.led_outputs, None);
        // Channels are kept: unlike the 4-state mode they are what is read.
        assert_eq!(draft.acquisition.channels, channels);
        // A dwell out of range keeps the rate for validation to refuse.
        draft.acquisition.state_dwell_us = Some(700);
        normalize_authoring_draft(&mut draft);
        assert_eq!(draft.acquisition.sample_rate_hz, 400);
        // A build already new enough is kept.
        let mut current = sequenced_test_lab();
        normalize_authoring_draft(&mut current);
        assert_eq!(current.required_firmware.build, "0x00010007");
        // Other modes keep their build.
        let mut ecg = factory_lab(ECG_ID);
        normalize_authoring_draft(&mut ecg);
        assert_eq!(ecg.required_firmware.build, "0x00010006");

        // Save, list, edit, and reload a new LED-sequenced lab.
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        let mut new_lab = sequenced_test_lab();
        new_lab.profile_id = "team.sequenced".into();
        new_lab.status = ProfileStatus::Draft;
        new_lab.source = ProfileSource::Instructor;
        let saved = store
            .save_lab_draft(new_lab, None, "save-sequenced-0001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(saved.profile_version, "1.0.0");
        assert!(saved.acquisition.is_led_sequenced());
        let edited = store
            .begin_lab_edit("team.sequenced")
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(edited.acquisition.channels, saved.acquisition.channels);
        let reopened = ProfileStore::with_root(root).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            reopened
                .get_locked("team.sequenced")
                .unwrap_or_else(|error| panic!("{error}")),
            saved
        );
    }

    #[test]
    fn builtin_profiles_are_valid_and_hashed() {
        let profiles = built_in_profiles().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(profiles.len(), factory_lab_count());
        assert!(profiles
            .iter()
            .all(|p| p.status == ProfileStatus::Locked && p.verify_integrity().is_ok()));
    }
    #[test]
    fn hash_detects_profile_change() {
        let mut profile = factory_lab(ECG_ID);
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
        assert_eq!(
            store.list_all().unwrap_or_default().len(),
            factory_lab_count()
        );
        assert!(!root.join("lab_state.json").exists());
        draft.description.push_str(" edited");
        let final_profile = store
            .save_lab_draft(draft.clone(), Some("1.0.0".into()), "save-ecg-0001".into())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(final_profile.verify_integrity().is_ok());
        assert_eq!(final_profile.profile_version, "1.0.1");
        assert_eq!(
            store.list_all().unwrap_or_default().len(),
            factory_lab_count() + 1
        );
        let retried = store
            .save_lab_draft(draft, Some("1.0.0".into()), "save-ecg-0001".into())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(retried.profile_version, "1.0.1");
        assert_eq!(
            store.list_all().unwrap_or_default().len(),
            factory_lab_count() + 1
        );
        let audit =
            fs::read_to_string(root.join("lab_write_audit.log")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(audit.lines().count(), 1);
        assert!(audit.contains("save_new_version"));
    }

    #[test]
    fn factory_defaults_are_available_after_repeated_read_only_initialization() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let factory_names: BTreeSet<String> = built_in_profiles()
            .unwrap_or_else(|error| panic!("{error}"))
            .into_iter()
            .map(|profile| profile.display_name)
            .collect();
        assert_eq!(factory_names.len(), factory_lab_count());
        for id in [ECG_ID, EMG_ID, BP_ID, PULSE_ID, GENERAL_ID, OP_AMP_ID] {
            assert!(factory_names.contains(&factory_lab(id).display_name));
        }
        for _ in 0..100 {
            let store =
                ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
            let names: BTreeSet<String> = store
                .list()
                .unwrap_or_else(|error| panic!("{error}"))
                .into_iter()
                .map(|profile| profile.display_name)
                .collect();
            assert_eq!(names, factory_names);
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
        // The template is General Analog's course default, whatever its version.
        assert_eq!(draft.profile_version, "1.0.0");
        assert_eq!(
            draft.acquisition.resolved_channels(),
            factory_lab(GENERAL_ID).acquisition.resolved_channels()
        );
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

        let exact = factory_lab(BP_ID);
        let package = root.join("factory-bp.lab.json");
        fs::create_dir_all(&root).unwrap_or_else(|error| panic!("{error}"));
        fs::write(
            &package,
            serde_json::to_vec_pretty(&exact).unwrap_or_default(),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert!(store.import_profile(&package).is_err());
        let entries = store.list_all().unwrap_or_default();
        assert_eq!(entries.len(), factory_lab_count() + 1);
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
        assert_eq!(active.len(), factory_lab_count());
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
    const BP_ID: &str = "wvu.bmeg420l.blood_pressure.ppg.course.capture.v1";
    const PULSE_ID: &str = "wvu.bmeg420l.pulseox.txrx.raw.course.capture.v1";
    const GENERAL_ID: &str = "wvu.bmeg420l.general.analog.development.v2";
    const OP_AMP_IMPORTED_HASH: &str =
        "c73cc1f2941910696d88f934b7b4c09b8b110438a4886d7d799b13807f61471d";
    const OP_AMP_INSTRUCTOR_HASH: &str =
        "bcc08868305ca55e81920e3a5805b020cfa612a0374e75fd5a290fa11f918ad9";
    /// The Op Amp Lab 2.0.1 that the retired v1.0.2 student installer
    /// (`scripts/install_op_amp_lab_profile.ps1`) embedded and installed,
    /// kept byte for byte (including its `\u` escapes).
    const OP_AMP_2_0_1_IMPORTED: &str = include_str!("testdata/op_amp_2.0.1_imported.lab.json");
    const OP_AMP_ID: &str = "wvu.bmeg420l.op_amp.course.capture.v1";
    const OP_AMP_1_0_0_HASH: &str =
        "90113222d07df3e239b95666510ab77b77356bd5dc0ae04b7660d60d0faa579d";

    /// General Analog's compiled-in course default version. Tests read it
    /// here, so a new General Analog factory version does not break them;
    /// only the golden guard and the legacy 2.0.1 constants name versions.
    fn general_default_version() -> String {
        factory_lab(GENERAL_ID).profile_version
    }

    /// No copy committed in Git (the factory lab tool's fallback source).
    fn no_git(_file_name: &str) -> Option<String> {
        None
    }

    fn instructor_store(root: &Path) -> ProfileStore {
        let store =
            ProfileStore::with_root(root.to_path_buf()).unwrap_or_else(|error| panic!("{error}"));
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        store
    }

    /// The legacy Op Amp Lab copy as it is stored in a catalog: as the
    /// installer wrote it (`imported`), or as the maintainer's own Lab
    /// Manager revision (`instructor`).
    fn legacy_op_amp_json(source: ProfileSource) -> String {
        if source == ProfileSource::Imported {
            return OP_AMP_2_0_1_IMPORTED.trim_end().to_string();
        }
        let mut profile: AcquisitionProfile =
            serde_json::from_str(OP_AMP_2_0_1_IMPORTED).unwrap_or_else(|error| panic!("{error}"));
        profile.source = source;
        profile
            .refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        serde_json::to_string_pretty(&profile).unwrap_or_else(|error| panic!("{error}"))
    }

    fn write_catalog(root: &Path, profiles: &[String], active: &[(&str, &str)]) {
        write_catalog_with_retired(root, profiles, &[], active);
    }

    fn write_catalog_with_retired(
        root: &Path,
        profiles: &[String],
        retired: &[(&str, &str)],
        active: &[(&str, &str)],
    ) {
        fs::create_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
        let active = active
            .iter()
            .map(|(id, version)| format!("\"{id}\": \"{version}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let retired = retired
            .iter()
            .map(|(id, version)| format!("[\"{id}\", \"{version}\"]"))
            .collect::<Vec<_>>()
            .join(", ");
        let text = format!(
            "{{\n  \"profiles\": [\n{}\n  ],\n  \"retired\": [{retired}],\n  \"active_versions\": {{ {active} }},\n  \"completed_save_requests\": {{}}\n}}",
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

    /// The repository's `profiles/` folder.
    fn repository_profiles_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../profiles")
    }

    /// `profiles/factory-labs.lock.json` as compiled into the tests.
    fn factory_lock() -> Vec<FactoryLabLockEntry> {
        parse_factory_lab_lock(include_str!("../../profiles/factory-labs.lock.json"))
            .unwrap_or_else(|error| panic!("{error}"))
    }

    /// The factory lab versions v1.0.2 shipped, with their canonical hashes.
    /// These hashes never change: they guard the canonical serialization
    /// (field names, order, and omitted optional fields) that every stored
    /// lab and every recording snapshot was hashed with.
    const PUBLISHED_V1_0_2_LABS: [(&str, &str, &str); 5] = [
        (
            GENERAL_ID,
            "2.0.0",
            "2cfca936f3af638774a6a4239ff9a740ba5272b765cc22c33118498050c2b790",
        ),
        (
            ECG_ID,
            "1.0.0",
            "3c59886674d1b8467003e6900987fa5e2643eb9c15306241eb4fc9603d3cad6d",
        ),
        (
            EMG_ID,
            "1.0.0",
            "cd0af7cf4a1ca25d8a5988f630825a30d7015aa6a40ef282af10f225e1ae60b8",
        ),
        (
            BP_ID,
            "1.0.0",
            "fbd4a5b6f210271694694feaac54876c44c6b09f8525f2f4e84f7c88f0bb266f",
        ),
        (
            PULSE_ID,
            "1.0.0",
            "019c25a69ef6e5b361774acc668915066c7113a7b58170c906a8049a69198bd0",
        ),
    ];

    /// A published factory lab version's file text: the compiled-in file
    /// while it is current, afterwards its copy in `profiles/legacy/`.
    fn published_lab_text(profile_id: &str, profile_version: &str) -> String {
        let is_it = |text: &str| {
            parse_lab_text(text).is_ok_and(|profile| {
                profile.profile_id == profile_id && profile.profile_version == profile_version
            })
        };
        if let Some((_, text)) = FACTORY_LAB_FILES.iter().find(|(_, text)| is_it(text)) {
            return (*text).to_string();
        }
        factory_lab_files_in(&repository_profiles_dir().join(FACTORY_LAB_LEGACY_FOLDER))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|path| fs::read_to_string(path).ok())
            .find(|text| is_it(text.as_str()))
            .unwrap_or_else(|| panic!("no file holds {profile_id} {profile_version}"))
    }

    #[test]
    fn published_factory_lab_files_keep_their_canonical_hashes() {
        let lock = factory_lock();
        for (id, version, hash) in PUBLISHED_V1_0_2_LABS {
            let profile = parse_lab_text(&published_lab_text(id, version))
                .unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(profile.integrity.canonical_hash, hash, "{id}");
            assert_eq!(
                profile
                    .computed_hash()
                    .unwrap_or_else(|error| panic!("{error}")),
                hash,
                "{id}"
            );
            assert!(profile.validate().is_ok(), "{id}");
            assert!(lock.contains(&FactoryLabLockEntry {
                profile_id: id.into(),
                profile_version: version.into(),
                canonical_hash: hash.into(),
            }));
            // The catalog's stored form round-trips to the same canonical bytes.
            let stored = serde_json::to_vec_pretty(&profile).unwrap_or_default();
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

    /// The built-in pulse-ox lab 1.1.0: the LED-sequenced mode with the same
    /// identity, notices and export name as 1.0.0, which stays published in
    /// `profiles/legacy/` (not compiled in) for rollback and old recordings.
    #[test]
    fn built_in_pulse_ox_lab_1_1_0_is_led_sequenced_and_keeps_1_0_0_in_legacy() {
        let lab = factory_lab(PULSE_ID);
        let legacy = legacy_pulse_ox_lab();
        assert_eq!(lab.profile_version, "1.1.0");
        assert_eq!(legacy.profile_version, "1.0.0");
        assert!(lab.validate_for_authoring().is_ok());
        assert!(factory_lock().contains(&FactoryLabLockEntry::of(&lab)));
        assert_eq!(
            published_lab_text(PULSE_ID, "1.0.0"),
            LEGACY_PULSE_OX_LAB_TEXT
        );
        assert!(!FACTORY_LAB_FILES
            .iter()
            .any(|(_, text)| normalized_text(text) == normalized_text(LEGACY_PULSE_OX_LAB_TEXT)));

        // The design's lab: tx (A0) and rx (A1), both shown by LED state,
        // D5 RED and D6 IR, 1000 µs per state, 1000 records/s, build 7.
        let design = sequenced_test_lab();
        assert_eq!(lab.acquisition, design.acquisition);
        assert_eq!(lab.plot_defaults, design.plot_defaults);
        assert_eq!(lab.required_firmware, design.required_firmware);
        assert_eq!(
            lab.acquisition.record_field_names(),
            ["led_state", "tx_counts", "rx_counts"]
        );
        assert!(lab
            .acquisition
            .channels
            .iter()
            .all(|channel| channel.split_by_led_state));
        assert_eq!(
            lab.acquisition
                .resolved_digital_outputs()
                .iter()
                .map(|output| (output.pin.as_str(), output.label.as_str(), &output.behavior))
                .collect::<Vec<_>>(),
            [
                (
                    "D5",
                    "Red LED",
                    &DigitalOutputBehavior::AcquisitionSequenced
                ),
                ("D6", "IR LED", &DigitalOutputBehavior::AcquisitionSequenced)
            ]
        );

        // Same identity, notices (raw only, no SpO2) and export name.
        assert_eq!(lab.profile_id, legacy.profile_id);
        assert_eq!(lab.display_name, legacy.display_name);
        assert_eq!(lab.category, legacy.category);
        assert_eq!(lab.source, legacy.source);
        assert_eq!(lab.status, legacy.status);
        assert_eq!(lab.safety, legacy.safety);
        assert_eq!(lab.export, legacy.export);
        assert!(lab
            .safety
            .notices
            .iter()
            .any(|notice| notice.contains("no SpO2")));
    }

    /// The published 1.0.0 file in `profiles/legacy/` is a rollback: Install
    /// lab file accepts it (it is a published factory version), and it then
    /// records in the legacy 4-state mode until Restore course default.
    #[test]
    fn the_published_pulse_ox_1_0_0_file_installs_as_a_4_state_rollback() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let file = dir
            .path()
            .join("pulseox_tx_rx_course_capture_1.0.0.profile.json");
        fs::write(&file, LEGACY_PULSE_OX_LAB_TEXT).unwrap_or_else(|error| panic!("{error}"));
        let store = ProfileStore::with_root(dir.path().join("profiles"))
            .unwrap_or_else(|error| panic!("{error}"));
        let summary = store
            .inspect_lab_file(&file)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(summary.refusal, None);
        assert_eq!(summary.acquisition_mode, AcquisitionMode::Pulseox4State);
        assert_eq!(
            summary.replaces.map(|replaced| replaced.profile_version),
            Some("1.1.0".to_string())
        );
        let installed = store
            .install_lab_file(&file, false)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(installed.profile_version, "1.0.0");
        assert_eq!(installed.source, ProfileSource::Imported);
        let active = store
            .get_locked(PULSE_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(active.profile_version, "1.0.0");
        assert_eq!(
            active.acquisition.acquisition_mode,
            AcquisitionMode::Pulseox4State
        );
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .restore_course_default(PULSE_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        let restored = store
            .get_locked(PULSE_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(restored.profile_version, "1.1.0");
        assert!(restored.acquisition.is_led_sequenced());
    }

    #[test]
    fn every_compiled_in_factory_lab_is_recorded_in_the_lock_with_its_hash() {
        let lock = factory_lock();
        for profile in built_in_profiles().unwrap_or_else(|error| panic!("{error}")) {
            let id = &profile.profile_id;
            let entry = lock
                .iter()
                .find(|entry| entry.key() == pkey(&profile))
                .unwrap_or_else(|| {
                    panic!(
                        "{id} {} is not in profiles/{FACTORY_LAB_LOCK_FILE}; run scripts/update_factory_labs.ps1 fix",
                        profile.profile_version
                    )
                });
            assert_eq!(
                entry.canonical_hash, profile.integrity.canonical_hash,
                "{id}"
            );
            assert_eq!(
                profile
                    .computed_hash()
                    .unwrap_or_else(|error| panic!("{error}")),
                entry.canonical_hash,
                "{id}"
            );
            // The compiled-in version is the newest published one.
            assert_eq!(
                newest_locked_version(&lock, id),
                Some(profile.profile_version.as_str()),
                "{id}"
            );
        }
    }

    #[test]
    fn every_profile_file_in_the_profiles_folder_is_compiled_in() {
        let dir = repository_profiles_dir();
        let files = factory_lab_files_in(&dir).unwrap_or_else(|error| panic!("{error}"));
        assert!(!files.is_empty());
        let on_disk: BTreeSet<String> = files.iter().map(|path| file_name_of(path)).collect();
        let compiled: BTreeSet<String> = FACTORY_LAB_FILES
            .iter()
            .map(|(name, _)| (*name).to_string())
            .collect();
        assert_eq!(
            on_disk, compiled,
            "every profiles/*.profile.json must be listed in FACTORY_LAB_FILES (and nothing else)"
        );
        for (name, text) in FACTORY_LAB_FILES {
            let file = fs::read_to_string(dir.join(name)).unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(normalized_text(&file), normalized_text(text), "{name}");
        }
    }

    #[test]
    fn factory_labs_follow_the_version_policy() {
        let profiles = built_in_profiles().unwrap_or_else(|error| panic!("{error}"));
        let mut ids = BTreeSet::new();
        for profile in &profiles {
            let id = &profile.profile_id;
            assert!(ids.insert(id.clone()), "{id} is compiled in twice");
            assert!(
                is_factory_lab_version(&profile.profile_version),
                "{id} {} must be MAJOR.MINOR.0",
                profile.profile_version
            );
            assert_eq!(profile.source, ProfileSource::BuiltIn, "{id}");
            assert_eq!(profile.status, ProfileStatus::Locked, "{id}");
            // Labs derived from a factory lab can be saved and imported.
            assert!(profile.validate_for_authoring().is_ok(), "{id}");
        }
        // The blank-lab template and the default recording lab are the one
        // factory lab in the `development` category (General Analog).
        let development: Vec<_> = profiles
            .iter()
            .filter(|profile| profile.category == "development")
            .map(|profile| profile.profile_id.as_str())
            .collect();
        assert_eq!(development, [GENERAL_ID]);
        assert!(is_factory_lab_version("1.10.0"));
        assert!(!is_factory_lab_version("1.0.1"));
        assert!(!is_factory_lab_version("1.01.0"));
    }

    #[test]
    fn the_factory_lab_lock_has_no_conflicting_entries() {
        let lock = factory_lock();
        assert!(!lock.is_empty());
        assert_eq!(factory_lab_lock_problems(&lock), Vec::<String>::new());
        let mut conflicting = lock.clone();
        let mut changed = lock[0].clone();
        changed.canonical_hash = "0".repeat(64);
        conflicting.push(changed);
        assert!(factory_lab_lock_problems(&conflicting)
            .iter()
            .any(|problem| problem.contains("different hashes")));
    }

    #[test]
    fn the_repository_profiles_folder_passes_the_factory_lab_check() {
        let report = factory_lab_check(&repository_profiles_dir(), factory_lab_sources());
        assert!(report.is_ok(), "{:#?}", report.problems);
        let current = report
            .labs
            .iter()
            .filter(|line| !line.starts_with(FACTORY_LAB_LEGACY_FOLDER))
            .count();
        assert_eq!(current, factory_lab_count());
    }

    #[test]
    fn the_legacy_op_amp_lab_copy_keeps_its_documented_hashes() {
        let imported: AcquisitionProfile =
            serde_json::from_str(OP_AMP_2_0_1_IMPORTED).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(imported.profile_id, GENERAL_ID);
        assert_eq!(imported.profile_version, "2.0.1");
        assert_eq!(imported.display_name, "Op Amp Lab");
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
        // The installer's escaped notices are kept byte for byte.
        assert!(
            OP_AMP_2_0_1_IMPORTED.contains("\"Teaching use only \\u2014 not a medical device.\"")
        );
        assert!(OP_AMP_2_0_1_IMPORTED.contains("safe 0\\u20135 V bench signal"));
        // Migration M1 recognizes exactly these two copies.
        assert_eq!(
            LEGACY_OP_AMP_LAB_HASHES,
            [OP_AMP_IMPORTED_HASH, OP_AMP_INSTRUCTOR_HASH]
        );
        assert_eq!(
            (LEGACY_OP_AMP_LAB_ID, LEGACY_OP_AMP_LAB_VERSION),
            (GENERAL_ID, "2.0.1")
        );
    }

    #[test]
    fn the_op_amp_lab_is_a_built_in_course_lab_with_its_own_id() {
        let op_amp = factory_lab(OP_AMP_ID);
        assert_eq!(op_amp.profile_version, "1.0.0");
        assert_eq!(op_amp.display_name, "Op Amp Lab");
        assert_eq!(op_amp.category, "course_op_amp");
        assert_eq!(op_amp.source, ProfileSource::BuiltIn);
        assert_eq!(op_amp.integrity.canonical_hash, OP_AMP_1_0_0_HASH);
        assert_eq!(
            op_amp
                .computed_hash()
                .unwrap_or_else(|error| panic!("{error}")),
            OP_AMP_1_0_0_HASH
        );
        let acquisition = &op_amp.acquisition;
        assert_eq!(acquisition.acquisition_mode, AcquisitionMode::Simultaneous);
        assert_eq!(acquisition.adc_resolution_bits, 14);
        assert_eq!(acquisition.sample_rate_hz, 100);
        assert_eq!(acquisition.analog_pins(), ["A0", "A1"]);
        assert_eq!(
            acquisition.record_field_names(),
            ["vmid_counts", "vout_counts"]
        );
        assert_eq!(
            acquisition
                .channels
                .iter()
                .map(|channel| (channel.id.as_str(), channel.label.as_str()))
                .collect::<Vec<_>>(),
            [("vmid", "Vmid (A0)"), ("vout", "Vout (A1)")]
        );
        assert!(acquisition.channels.iter().all(|channel| {
            channel.allowed_conversions == ["counts_volts"]
                && channel.default_display_unit.as_deref() == Some("counts")
        }));
        assert!(acquisition.resolved_digital_outputs().is_empty());
        assert_eq!(
            op_amp
                .plot_defaults
                .groups
                .iter()
                .map(|group| group.channel_ids.clone())
                .collect::<Vec<_>>(),
            [vec!["vmid".to_string()], vec!["vout".to_string()]]
        );
        assert_eq!(op_amp.export.signal_name, "op_amp_course_capture");
        let general = factory_lab(GENERAL_ID);
        assert_eq!(op_amp.safety, general.safety);
        assert_eq!(
            acquisition.allowed_duration_modes,
            general.acquisition.allowed_duration_modes
        );
        assert_eq!(
            acquisition.timed_presets_seconds,
            general.acquisition.timed_presets_seconds
        );
        // A bench-only course lab: no ECG/EMG acknowledgement at Start.
        assert!(!op_amp.requires_bench_acknowledgement());
        assert!(acquisition.firmware_configuration_issue().is_none());
        // It records what the legacy copy recorded, under its own ID.
        let legacy: AcquisitionProfile =
            serde_json::from_str(OP_AMP_2_0_1_IMPORTED).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(legacy.acquisition.analog_pins(), acquisition.analog_pins());
        assert_eq!(
            legacy.acquisition.sample_rate_hz,
            acquisition.sample_rate_hz
        );
        assert_eq!(
            legacy.acquisition.adc_resolution_bits,
            acquisition.adc_resolution_bits
        );
        // Appended last, so the earlier factory labs keep their order.
        assert_eq!(
            FACTORY_LAB_FILES.last().map(|(name, _)| *name),
            Some("op_amp_course_capture.profile.json")
        );
    }

    /// The catalog file as JSON.
    fn catalog_json(root: &Path) -> serde_json::Value {
        serde_json::from_slice(&fs::read(root.join(CATALOG_FILE)).unwrap_or_default())
            .unwrap_or_else(|error| panic!("{error}"))
    }

    /// `lab_state.pre-migration-*.json` copies in the catalog folder.
    fn pre_migration_backups(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = file_names(root)
            .into_iter()
            .filter(|name| name.starts_with("lab_state.pre-migration-") && name.ends_with(".json"))
            .collect();
        names.sort();
        names
    }

    /// A local lab with its own ID, which the migration must leave alone.
    fn team_lab_json() -> String {
        let mut team = factory_lab(ECG_ID);
        team.profile_id = "team.ecg".into();
        team.source = ProfileSource::Instructor;
        team.refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        serde_json::to_string_pretty(&team).unwrap_or_else(|error| panic!("{error}"))
    }

    const M1_NOTICE: &str = "The Op Amp Lab is now built in. The older Op Amp Lab copy that replaced General Analog was retired; a backup was saved to ";

    #[test]
    fn m1_retires_an_active_legacy_op_amp_copy_with_either_hash_and_restores_general_analog() {
        for (source, hash) in [
            (ProfileSource::Imported, OP_AMP_IMPORTED_HASH),
            (ProfileSource::Instructor, OP_AMP_INSTRUCTOR_HASH),
        ] {
            let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
            let root = dir.path().join("profiles");
            write_catalog(
                &root,
                &[legacy_op_amp_json(source.clone()), team_lab_json()],
                &[(GENERAL_ID, "2.0.1"), ("team.ecg", "1.0.0")],
            );
            let before = fs::read(root.join(CATALOG_FILE)).unwrap_or_default();

            // Student mode: the migration needs no instructor action.
            let store =
                ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
            let general = store
                .get_locked(GENERAL_ID)
                .unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(
                general.profile_version,
                general_default_version(),
                "{source:?}"
            );
            assert_eq!(general.source, ProfileSource::BuiltIn);
            let listed: BTreeMap<String, String> = store
                .list()
                .unwrap_or_else(|error| panic!("{error}"))
                .into_iter()
                .map(|profile| (profile.profile_id, profile.display_name))
                .collect();
            assert_eq!(listed.len(), factory_lab_count() + 1);
            assert_eq!(
                listed.get(OP_AMP_ID).map(String::as_str),
                Some("Op Amp Lab")
            );
            assert_eq!(listed.get(GENERAL_ID), Some(&general.display_name));
            assert!(listed.contains_key("team.ecg"));

            // The catalog was copied first, byte for byte.
            let backups = pre_migration_backups(&root);
            assert_eq!(backups.len(), 1, "{source:?}");
            assert_eq!(fs::read(root.join(&backups[0])).unwrap_or_default(), before);

            // The Lab panel notice names that copy.
            let status = store
                .catalog_status()
                .unwrap_or_else(|error| panic!("{error}"));
            assert!(status.writable);
            assert!(status.skipped_labs.is_empty());
            assert_eq!(status.migration_notices.len(), 1);
            assert!(
                status.migration_notices[0].starts_with(M1_NOTICE),
                "{}",
                status.migration_notices[0]
            );
            assert!(status.migration_notices[0].ends_with(&format!("{}.", backups[0])));
            let serialized =
                serde_json::to_value(&status).unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(
                serialized["migration_notices"][0],
                status.migration_notices[0].as_str()
            );

            // On disk the copy is kept unchanged but retired, General Analog
            // has no stored selection (its course default), and the
            // migration is recorded.
            let written = catalog_json(&root);
            let kept = written["profiles"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|entry| {
                    entry["profile_id"] == GENERAL_ID && entry["profile_version"] == "2.0.1"
                })
                .unwrap_or_else(|| panic!("the legacy copy must be kept"));
            assert_eq!(kept["integrity"]["canonical_hash"], hash);
            assert_eq!(
                written["retired"],
                serde_json::json!([[GENERAL_ID, "2.0.1"]])
            );
            assert!(written["active_versions"].get(GENERAL_ID).is_none());
            assert_eq!(written["active_versions"]["team.ecg"], "1.0.0");
            assert_eq!(
                written["applied_migrations"],
                serde_json::json!([OP_AMP_BUILTIN_MIGRATION])
            );
            let audit = fs::read_to_string(root.join("lab_write_audit.log"))
                .unwrap_or_else(|error| panic!("{error}"));
            assert!(audit.contains(&format!(
                "\t{OP_AMP_BUILTIN_MIGRATION}\t{GENERAL_ID}\t2.0.1\t{}\t",
                general_default_version()
            )));

            // Lab Manager lists the copy as retired, so it can be restored.
            store
                .set_mode(ProfileMode::InstructorAuthoring, true)
                .unwrap_or_else(|error| panic!("{error}"));
            let entry = store
                .list_all()
                .unwrap_or_else(|error| panic!("{error}"))
                .into_iter()
                .find(|entry| pkey(&entry.profile) == legacy_op_amp_lab_key())
                .unwrap_or_else(|| panic!("the legacy copy must be listed"));
            assert!(entry.retired && !entry.active);
        }
    }

    #[test]
    fn m1_runs_once_and_a_deliberate_reactivation_is_respected() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        write_catalog(
            &root,
            &[legacy_op_amp_json(ProfileSource::Imported)],
            &[(GENERAL_ID, "2.0.1")],
        );
        let first = ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            first
                .get_locked(GENERAL_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            general_default_version()
        );
        let migrated = fs::read(root.join(CATALOG_FILE)).unwrap_or_default();

        // Restarting changes nothing and reports nothing.
        for _ in 0..3 {
            let again =
                ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(
                again
                    .get_locked(GENERAL_ID)
                    .unwrap_or_else(|error| panic!("{error}"))
                    .profile_version,
                general_default_version()
            );
            assert!(again
                .catalog_status()
                .unwrap_or_else(|error| panic!("{error}"))
                .migration_notices
                .is_empty());
        }
        assert_eq!(
            fs::read(root.join(CATALOG_FILE)).unwrap_or_default(),
            migrated
        );
        assert_eq!(pre_migration_backups(&root).len(), 1);

        // An instructor who restores the copy keeps it after a restart.
        let instructor = instructor_store(&root);
        instructor
            .restore_retired(GENERAL_ID, "2.0.1")
            .unwrap_or_else(|error| panic!("{error}"));
        let reopened = instructor_store(&root);
        let op_amp = reopened
            .get_locked(GENERAL_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(op_amp.display_name, "Op Amp Lab");
        assert_eq!(op_amp.profile_version, "2.0.1");
        assert_eq!(op_amp.integrity.canonical_hash, OP_AMP_IMPORTED_HASH);
        assert!(reopened
            .catalog_status()
            .unwrap_or_else(|error| panic!("{error}"))
            .migration_notices
            .is_empty());
        assert_eq!(pre_migration_backups(&root).len(), 1);
        assert_eq!(
            catalog_json(&root)["applied_migrations"],
            serde_json::json!([OP_AMP_BUILTIN_MIGRATION])
        );

        // Editing it allocates 2.0.2 (one patch above the copy's 2.0.1) and
        // keeps its CSV fields; that revision also survives a restart.
        let edit = reopened
            .begin_lab_edit(GENERAL_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        let saved = reopened
            .save_lab_draft(edit, Some("2.0.1".into()), "opamp-edit-00001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(saved.profile_version, "2.0.2");
        assert_eq!(saved.acquisition.channels[1].csv_name, "01_counts");
        let last = ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            last.get_locked(GENERAL_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "2.0.2"
        );
        assert_eq!(pre_migration_backups(&root).len(), 1);
    }

    #[test]
    fn m1_leaves_an_inactive_legacy_copy_alone_and_records_itself_at_the_next_write() {
        for retired in [false, true] {
            let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
            let root = dir.path().join("profiles");
            // General Analog's course default is selected: the copy was
            // never selected, or it was retired.
            let retired_keys: &[(&str, &str)] = if retired {
                &[(GENERAL_ID, "2.0.1")]
            } else {
                &[]
            };
            write_catalog_with_retired(
                &root,
                &[legacy_op_amp_json(ProfileSource::Instructor)],
                retired_keys,
                &[],
            );
            let before = fs::read(root.join(CATALOG_FILE)).unwrap_or_default();
            let store =
                ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(
                store
                    .get_locked(GENERAL_ID)
                    .unwrap_or_else(|error| panic!("{error}"))
                    .profile_version,
                general_default_version()
            );
            assert!(store
                .catalog_status()
                .unwrap_or_else(|error| panic!("{error}"))
                .migration_notices
                .is_empty());
            assert!(pre_migration_backups(&root).is_empty());
            assert_eq!(
                fs::read(root.join(CATALOG_FILE)).unwrap_or_default(),
                before,
                "loading writes nothing (retired: {retired})"
            );

            // The next write records the migration because the copy is in
            // the catalog, so this deliberate selection survives a restart.
            let instructor = instructor_store(&root);
            if retired {
                instructor
                    .restore_retired(GENERAL_ID, "2.0.1")
                    .unwrap_or_else(|error| panic!("{error}"));
            } else {
                instructor
                    .set_active_version(GENERAL_ID, "2.0.1")
                    .unwrap_or_else(|error| panic!("{error}"));
            }
            assert_eq!(
                catalog_json(&root)["applied_migrations"],
                serde_json::json!([OP_AMP_BUILTIN_MIGRATION])
            );
            let reopened =
                ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(
                reopened
                    .get_locked(GENERAL_ID)
                    .unwrap_or_else(|error| panic!("{error}"))
                    .profile_version,
                "2.0.1"
            );
            assert!(reopened
                .catalog_status()
                .unwrap_or_else(|error| panic!("{error}"))
                .migration_notices
                .is_empty());
            assert!(pre_migration_backups(&root).is_empty());
        }
    }

    #[test]
    fn m1_is_not_recorded_without_a_legacy_copy_so_a_later_install_is_still_migrated() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = instructor_store(&root);
        let mut draft = store
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        draft.description.push_str(" local");
        store
            .save_lab_draft(draft, Some("1.0.0".into()), "m1-absent-000001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(catalog_json(&root).get("applied_migrations").is_none());
        let reopened =
            ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        assert!(reopened
            .catalog_status()
            .unwrap_or_else(|error| panic!("{error}"))
            .migration_notices
            .is_empty());
        assert!(pre_migration_backups(&root).is_empty());

        // The v1.0.2 student installer later adds its copy and selects it.
        let mut catalog = catalog_json(&root);
        let installed: serde_json::Value =
            serde_json::from_str(OP_AMP_2_0_1_IMPORTED).unwrap_or_else(|error| panic!("{error}"));
        catalog["profiles"]
            .as_array_mut()
            .unwrap_or_else(|| panic!("profiles must be a list"))
            .push(installed);
        catalog["active_versions"][GENERAL_ID] = "2.0.1".into();
        fs::write(
            root.join(CATALOG_FILE),
            serde_json::to_vec_pretty(&catalog).unwrap_or_default(),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        let migrated = instructor_store(&root);
        assert_eq!(
            migrated
                .get_locked(GENERAL_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            general_default_version()
        );
        assert_eq!(
            migrated
                .get_locked(ECG_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "1.0.1"
        );
        assert_eq!(
            migrated
                .catalog_status()
                .unwrap_or_else(|error| panic!("{error}"))
                .migration_notices
                .len(),
            1
        );
        assert_eq!(pre_migration_backups(&root).len(), 1);

        // Reset starts a new catalog without the copy, so the record goes too.
        migrated
            .reset_local_customizations()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(catalog_json(&root).get("applied_migrations").is_none());
    }

    #[test]
    fn m1_waits_for_a_writable_catalog() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        write_catalog(
            &root,
            &[legacy_op_amp_json(ProfileSource::Imported)],
            &[(GENERAL_ID, "2.0.1")],
        );
        let before = fs::read(root.join(CATALOG_FILE)).unwrap_or_default();

        // The read-only factory fallback never migrates or writes.
        let fallback = ProfileStore::factory_only(root.clone(), "unavailable".into());
        assert!(fallback
            .catalog_status()
            .unwrap_or_else(|error| panic!("{error}"))
            .migration_notices
            .is_empty());

        // The catalog lock cannot be taken (its lock file is a folder here):
        // the catalog loads as it is, and nothing is copied or written.
        fs::create_dir(root.join(CATALOG_LOCK_FILE)).unwrap_or_else(|error| panic!("{error}"));
        let locked_out =
            ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            locked_out
                .get_locked(GENERAL_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "2.0.1"
        );
        assert!(locked_out
            .catalog_status()
            .unwrap_or_else(|error| panic!("{error}"))
            .migration_notices
            .is_empty());
        assert!(pre_migration_backups(&root).is_empty());
        assert_eq!(
            fs::read(root.join(CATALOG_FILE)).unwrap_or_default(),
            before
        );

        // The next start with a writable catalog migrates it.
        fs::remove_dir(root.join(CATALOG_LOCK_FILE)).unwrap_or_else(|error| panic!("{error}"));
        let migrated =
            ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            migrated
                .get_locked(GENERAL_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            general_default_version()
        );
        assert_eq!(pre_migration_backups(&root).len(), 1);
    }

    #[test]
    fn m1_is_never_recorded_by_a_write_before_it_ran() {
        // Startup could not take the catalog lock, so the copy is still
        // selected; a later write in that window must not record the
        // migration as done.
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("deferred");
        write_catalog(
            &root,
            &[legacy_op_amp_json(ProfileSource::Imported)],
            &[(GENERAL_ID, "2.0.1")],
        );
        fs::create_dir(root.join(CATALOG_LOCK_FILE)).unwrap_or_else(|error| panic!("{error}"));
        let deferred =
            ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        fs::remove_dir(root.join(CATALOG_LOCK_FILE)).unwrap_or_else(|error| panic!("{error}"));
        deferred
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        save_an_emg_revision(&deferred, "m1-deferred-0001");
        let written = catalog_json(&root);
        assert!(written.get("applied_migrations").is_none(), "{written}");
        assert_eq!(written["active_versions"][GENERAL_ID], "2.0.1");
        let next_start =
            ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            next_start
                .get_locked(GENERAL_ID)
                .map(|lab| lab.profile_version)
                .ok(),
            Some(general_default_version())
        );
        assert_eq!(
            next_start
                .catalog_status()
                .unwrap_or_else(|error| panic!("{error}"))
                .migration_notices
                .len(),
            1
        );
        assert_eq!(
            catalog_json(&root)["applied_migrations"],
            serde_json::json!([OP_AMP_BUILTIN_MIGRATION])
        );

        // Another program adds a selected copy while a window is open: that
        // window reloads the catalog at its next write and writes it back
        // without the record, so the next start still migrates it.
        let root = dir.path().join("reloaded");
        let open = instructor_store(&root);
        save_an_emg_revision(&open, "m1-reloaded-001");
        let mut catalog = catalog_json(&root);
        let installed: serde_json::Value =
            serde_json::from_str(OP_AMP_2_0_1_IMPORTED).unwrap_or_else(|error| panic!("{error}"));
        catalog["profiles"]
            .as_array_mut()
            .unwrap_or_else(|| panic!("profiles must be a list"))
            .push(installed);
        catalog["active_versions"][GENERAL_ID] = "2.0.1".into();
        fs::write(
            root.join(CATALOG_FILE),
            serde_json::to_vec_pretty(&catalog).unwrap_or_default(),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        save_an_emg_revision(&open, "m1-reloaded-002");
        let written = catalog_json(&root);
        assert!(written.get("applied_migrations").is_none(), "{written}");
        assert_eq!(written["active_versions"][GENERAL_ID], "2.0.1");
        assert_eq!(stored_entries(&written, EMG_ID).len(), 2);
        let next_start =
            ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            next_start
                .get_locked(GENERAL_ID)
                .map(|lab| lab.profile_version)
                .ok(),
            Some(general_default_version())
        );
        assert_eq!(pre_migration_backups(&root).len(), 1);
    }

    #[test]
    fn m1_respects_an_instructor_who_imports_the_legacy_copy_deliberately() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let store = instructor_store(&root);
        let package = dir.path().join("op-amp.lab.json");
        fs::write(&package, legacy_op_amp_json(ProfileSource::Imported))
            .unwrap_or_else(|error| panic!("{error}"));
        let imported = store
            .import_profile(&package)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(pkey(&imported), legacy_op_amp_lab_key());
        assert_eq!(imported.integrity.canonical_hash, OP_AMP_IMPORTED_HASH);
        assert_eq!(
            catalog_json(&root)["applied_migrations"],
            serde_json::json!([OP_AMP_BUILTIN_MIGRATION])
        );
        let reopened =
            ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            reopened
                .get_locked(GENERAL_ID)
                .map(|lab| lab.profile_version)
                .ok(),
            Some("2.0.1".to_string())
        );
        assert!(reopened
            .catalog_status()
            .unwrap_or_else(|error| panic!("{error}"))
            .migration_notices
            .is_empty());
        assert!(pre_migration_backups(&root).is_empty());
    }

    #[test]
    fn general_analog_2_0_1_is_reserved_so_an_instructor_revision_installs_on_migrated_computers() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        // The instructor's computer never held the Op Amp Lab copy, and its
        // first edit of General Analog 2.0.0 still skips 2.0.1.
        assert_eq!(general_default_version(), "2.0.0");
        let teacher = dir.path().join("teacher");
        let export = exported_lab(&teacher, GENERAL_ID, None, |draft| {
            draft.description = "Week 3 bench signals.".into();
        });

        // A student computer that ran the v1.0.2 Op Amp Lab installer: the
        // migration retires the copy and keeps it in the catalog.
        let root = dir.path().join("student");
        write_catalog(
            &root,
            &[legacy_op_amp_json(ProfileSource::Imported)],
            &[(GENERAL_ID, "2.0.1")],
        );
        let store = ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            catalog_json(&root)["retired"],
            serde_json::json!([[GENERAL_ID, "2.0.1"]])
        );

        let summary = store
            .inspect_lab_file(&export)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(summary.profile_id, GENERAL_ID);
        assert_eq!(summary.profile_version, "2.0.2");
        assert_eq!(summary.refusal, None);
        let replaces = summary
            .replaces
            .unwrap_or_else(|| panic!("the file replaces General Analog's course default"));
        assert_eq!(replaces.profile_version, general_default_version());
        assert!(replaces.course_default);
        let installed = store
            .install_lab_file(&export, false)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(installed.profile_version, "2.0.2");
        assert_eq!(
            store
                .get_locked(GENERAL_ID)
                .map(|lab| lab.profile_version)
                .ok(),
            Some("2.0.2".to_string())
        );
        let written = catalog_json(&root);
        assert_eq!(written["active_versions"][GENERAL_ID], "2.0.2");
        assert_eq!(
            written["retired"],
            serde_json::json!([[GENERAL_ID, "2.0.1"]])
        );

        // The instructor's next edit continues at 2.0.3.
        let instructor = instructor_store(&teacher.join("catalog"));
        let mut draft = instructor
            .begin_lab_edit(GENERAL_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        draft.description = "Week 4 bench signals.".into();
        let next = instructor
            .save_lab_draft(draft, Some("2.0.2".into()), "reserve-next-0001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(next.profile_version, "2.0.3");
        // Other labs are numbered as before.
        let mut ecg = instructor
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        ecg.description.push_str(" Week 4.");
        let ecg = instructor
            .save_lab_draft(ecg, Some("1.0.0".into()), "reserve-ecg-0001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(ecg.profile_version, "1.0.1");
    }

    /// Another program holds `lab_state.json` open for reading only: the
    /// catalog can be read and copied but not replaced.
    #[cfg(windows)]
    fn hold_catalog_open_for_reading(root: &Path) -> fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
            .open(root.join(CATALOG_FILE))
            .unwrap_or_else(|error| panic!("{error}"))
    }

    #[cfg(windows)]
    #[test]
    fn m1_keeps_its_result_for_this_launch_when_the_write_fails_and_retries_at_the_next_start() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        write_catalog(
            &root,
            &[legacy_op_amp_json(ProfileSource::Imported)],
            &[(GENERAL_ID, "2.0.1")],
        );
        let before = fs::read(root.join(CATALOG_FILE)).unwrap_or_default();
        let holder = hold_catalog_open_for_reading(&root);
        let store = ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        drop(holder);
        assert_eq!(
            fs::read(root.join(CATALOG_FILE)).unwrap_or_default(),
            before,
            "the failed write leaves the catalog unchanged"
        );
        assert!(!file_names(&root).iter().any(|name| name.ends_with(".tmp")));
        let backups = pre_migration_backups(&root);
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(root.join(&backups[0])).unwrap_or_default(), before);
        // This launch shows the migrated labs and says the change was not saved.
        assert_eq!(
            store
                .get_locked(GENERAL_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            general_default_version()
        );
        let notices = store
            .catalog_status()
            .unwrap_or_else(|error| panic!("{error}"))
            .migration_notices;
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("could not be saved"), "{}", notices[0]);
        assert!(notices[0].contains(&backups[0]));

        // The next start migrates again and saves it. The catalog is
        // unchanged, so the backup the first start made is used again.
        let next_start =
            ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            next_start
                .get_locked(GENERAL_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            general_default_version()
        );
        let notices = next_start
            .catalog_status()
            .unwrap_or_else(|error| panic!("{error}"))
            .migration_notices;
        assert!(notices.len() == 1 && notices[0].starts_with(M1_NOTICE));
        assert!(notices[0].contains(&backups[0]), "{}", notices[0]);
        assert_eq!(
            catalog_json(&root)["applied_migrations"],
            serde_json::json!([OP_AMP_BUILTIN_MIGRATION])
        );
        assert_eq!(pre_migration_backups(&root), backups);
        assert_eq!(fs::read(root.join(&backups[0])).unwrap_or_default(), before);
    }

    /// A read-only catalog can be copied but not replaced. The copy is a
    /// normal file (not read-only), and a start whose write keeps failing
    /// does not add another copy every time.
    #[test]
    fn m1_backs_up_a_read_only_catalog_once_however_often_the_write_fails() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        write_catalog(
            &root,
            &[legacy_op_amp_json(ProfileSource::Imported)],
            &[(GENERAL_ID, "2.0.1")],
        );
        let catalog = root.join(CATALOG_FILE);
        let before = fs::read(&catalog).unwrap_or_default();
        let set_read_only = |read_only: bool| {
            let mut permissions = fs::metadata(&catalog)
                .unwrap_or_else(|error| panic!("{error}"))
                .permissions();
            permissions.set_readonly(read_only);
            fs::set_permissions(&catalog, permissions).unwrap_or_else(|error| panic!("{error}"));
        };
        set_read_only(true);
        for _ in 0..3 {
            let store =
                ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
            let backups = pre_migration_backups(&root);
            assert_eq!(backups.len(), 1, "{backups:?}");
            let backup = root.join(&backups[0]);
            assert_eq!(fs::read(&backup).unwrap_or_default(), before);
            assert!(!fs::metadata(&backup)
                .unwrap_or_else(|error| panic!("{error}"))
                .permissions()
                .readonly());
            let notices = store
                .catalog_status()
                .unwrap_or_else(|error| panic!("{error}"))
                .migration_notices;
            assert_eq!(notices.len(), 1);
            assert!(notices[0].contains(&backups[0]), "{}", notices[0]);
            if fs::read(&catalog).unwrap_or_default() != before {
                // A platform that replaces read-only files saved the migration.
                break;
            }
            assert!(notices[0].contains("could not be saved"), "{}", notices[0]);
        }
        set_read_only(false);
    }

    #[test]
    #[allow(clippy::permissions_set_readonly_false)]
    fn copy_durably_copies_bytes_into_a_new_writable_file_and_never_replaces_one() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let source = dir.path().join("source.json");
        fs::write(&source, b"{ \"profiles\": [] }").unwrap_or_else(|error| panic!("{error}"));
        let mut permissions = fs::metadata(&source)
            .unwrap_or_else(|error| panic!("{error}"))
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&source, permissions.clone()).unwrap_or_else(|error| panic!("{error}"));
        let copy = dir.path().join("copy.json");
        copy_durably(&source, &copy).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(fs::read(&copy).unwrap_or_default(), b"{ \"profiles\": [] }");
        assert!(!fs::metadata(&copy)
            .unwrap_or_else(|error| panic!("{error}"))
            .permissions()
            .readonly());
        // An existing file is neither replaced nor removed.
        fs::write(&copy, b"kept").unwrap_or_else(|error| panic!("{error}"));
        assert!(copy_durably(&source, &copy).is_err());
        assert_eq!(fs::read(&copy).unwrap_or_default(), b"kept");
        // A missing source leaves nothing behind.
        let nothing = dir.path().join("nothing.json");
        assert!(copy_durably(&dir.path().join("missing.json"), &nothing).is_err());
        assert!(!nothing.exists());
        permissions.set_readonly(false);
        fs::set_permissions(&source, permissions).unwrap_or_else(|error| panic!("{error}"));
    }

    #[cfg(windows)]
    #[test]
    fn m1_result_kept_after_a_failed_write_is_saved_by_the_next_write_in_that_launch() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        write_catalog(
            &root,
            &[legacy_op_amp_json(ProfileSource::Instructor)],
            &[(GENERAL_ID, "2.0.1")],
        );
        let holder = hold_catalog_open_for_reading(&root);
        let store = ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        drop(holder);
        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        let draft = store
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .save_lab_draft(draft, Some("1.0.0".into()), "m1-after-fail-01".into())
            .unwrap_or_else(|error| panic!("{error}"));
        let written = catalog_json(&root);
        assert_eq!(
            written["retired"],
            serde_json::json!([[GENERAL_ID, "2.0.1"]])
        );
        assert!(written["active_versions"].get(GENERAL_ID).is_none());
        assert_eq!(
            written["applied_migrations"],
            serde_json::json!([OP_AMP_BUILTIN_MIGRATION])
        );
        // So the next start has nothing left to migrate.
        let reopened =
            ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        assert!(reopened
            .catalog_status()
            .unwrap_or_else(|error| panic!("{error}"))
            .migration_notices
            .is_empty());
        assert_eq!(pre_migration_backups(&root).len(), 1);
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
        let mut legacy = legacy_pulse_ox_lab();
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
        let store = four_state_pulse_ox_store(&dir.path().join("profiles"));
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
            split_by_led_state: false,
        });
        draft.acquisition.state_dwell_us = Some(2_000);
        draft.acquisition.sample_rate_hz = 125;
        let saved = store
            .save_lab_draft(draft, Some("1.0.1".into()), "pulse-save-00001".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(saved.acquisition.channels.is_empty());
        assert_eq!(saved.acquisition.sample_rate_hz, 125);

        let mut wrong_rate = store
            .begin_lab_edit(PULSE_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        wrong_rate.acquisition.sample_rate_hz = 1_000;
        let error = store
            .save_lab_draft(wrong_rate, Some("1.0.2".into()), "pulse-save-00002".into())
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
                Some("1.0.2".into()),
                "pulse-save-00003".into(),
            )
            .err()
            .unwrap_or_else(|| panic!("an extra pulse-ox output must be refused"));
        assert!(
            error.to_string().contains("only their RED and IR"),
            "{error}"
        );

        // The built-in pulse-ox lab 1.1.0 (LED-sequenced) edits as its real
        // channels, and its record rate must follow the dwell.
        let fresh = instructor_store(&dir.path().join("fresh"));
        let mut sequenced = fresh
            .begin_lab_edit(PULSE_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(sequenced.acquisition.is_led_sequenced());
        assert_eq!(sequenced.acquisition.analog_pins(), ["A0", "A1"]);
        assert!(sequenced.acquisition.led_outputs.is_none());
        assert!(sequenced.acquisition.analog_inputs.is_none());
        assert_eq!(sequenced.acquisition.digital_outputs.len(), 2);
        sequenced.acquisition.state_dwell_us = Some(2_000);
        sequenced.acquisition.sample_rate_hz = 125;
        let error = fresh
            .save_lab_draft(
                sequenced.clone(),
                Some("1.1.0".into()),
                "pulse-save-00004".into(),
            )
            .err()
            .unwrap_or_else(|| panic!("a record rate that does not follow the dwell"));
        assert!(error.to_string().contains("record rate"), "{error}");
        sequenced.acquisition.sample_rate_hz = 500;
        let saved = fresh
            .save_lab_draft(sequenced, Some("1.1.0".into()), "pulse-save-00005".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(saved.profile_version, "1.1.1");
        assert_eq!(saved.acquisition.sample_rate_hz, 500);
        assert_eq!(saved.acquisition.analog_pins(), ["A0", "A1"]);
    }

    /// An instructor store whose active pulse-ox lab is a legacy 4-state
    /// revision (1.0.1: the published 1.0.0 saved by an instructor), as on a
    /// lab PC that edited the pulse-ox lab before it moved to 1.1.0.
    fn four_state_pulse_ox_store(root: &Path) -> ProfileStore {
        let mut revision = legacy_pulse_ox_lab();
        revision.profile_version = "1.0.1".into();
        revision.source = ProfileSource::Instructor;
        revision
            .refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        write_catalog(
            root,
            &[serde_json::to_string_pretty(&revision).unwrap_or_default()],
            &[(PULSE_ID, "1.0.1")],
        );
        let store = instructor_store(root);
        let active = store
            .get_locked(PULSE_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(active.profile_version, "1.0.1");
        assert_eq!(
            active.acquisition.acquisition_mode,
            AcquisitionMode::Pulseox4State
        );
        store
    }

    #[test]
    fn pulse_ox_roles_come_from_an_explicit_field_with_the_label_rule_as_fallback() {
        let mut pulse = factory_lab(PULSE_ID);
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
        let mut bp = factory_lab(BP_ID);
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
        let general_base = general.profile_version.clone();
        general.acquisition.channels.truncate(1);
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
                Some(general_base.clone()),
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
        let error = store
            .save_lab_draft(
                general.clone(),
                Some(general_base.clone()),
                "csv-collide-0002".into(),
            )
            .err()
            .unwrap_or_else(|| panic!("a raw name equal to a derived kPa column must be refused"));
        assert!(error.to_string().contains("would appear twice"), "{error}");
        general.acquisition.channels[1].csv_name = "01_counts".into();
        let saved = store
            .save_lab_draft(
                general,
                Some(general_base.clone()),
                "csv-collide-0003".into(),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        // The next patch, skipping General Analog 2.0.1, which is reserved
        // for the legacy Op Amp Lab copy (see `next_revision_version`).
        assert_eq!(
            version_key(&saved.profile_version),
            version_key(&general_base).map(|(major, minor, patch)| {
                let next = if (major, minor, patch) == (2, 0, 0) {
                    2
                } else {
                    patch + 1
                };
                (major, minor, next)
            })
        );

        // A stored lab whose CSV name predates these rules still loads.
        let mut legacy = factory_lab(ECG_ID);
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

    #[test]
    fn export_replaces_the_chosen_file_atomically_and_leaves_no_temporary() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let author = instructor_store(&dir.path().join("author"));
        let exports = dir.path().join("exports");
        fs::create_dir_all(&exports).unwrap_or_else(|error| panic!("{error}"));
        let package = exports.join("ecg.lab.json");
        fs::write(&package, b"an older export").unwrap_or_else(|error| panic!("{error}"));
        let version = author
            .get_locked(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"))
            .profile_version;
        author
            .export_profile(ECG_ID, &version, &package)
            .unwrap_or_else(|error| panic!("{error}"));
        let exported: AcquisitionProfile =
            serde_json::from_slice(&fs::read(&package).unwrap_or_else(|error| panic!("{error}")))
                .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(exported.profile_id, ECG_ID);
        let names = fs::read_dir(&exports)
            .unwrap_or_else(|error| panic!("{error}"))
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["ecg.lab.json".to_owned()]);
    }

    #[test]
    fn versions_written_now_are_canonical_while_stored_ones_still_load() {
        assert!(canonical_semver("1.0.1"));
        assert!(canonical_semver("0.10.0"));
        assert!(!canonical_semver("1.0.01"));
        assert!(!canonical_semver("18446744073709551616.0.0"));
        assert_eq!(version_key("1.0.01"), Some((1, 0, 1)));
        // An oversized field is unknown, never silently 0.
        assert_eq!(version_key("18446744073709551616.0.0"), None);

        let mut lab = factory_lab(ECG_ID);
        for version in ["1.0.01", "18446744073709551616.0.0"] {
            lab.profile_version = version.into();
            lab.refresh_hash().unwrap_or_else(|error| panic!("{error}"));
            assert!(lab.validate().is_ok(), "{version} must stay loadable");
            assert!(lab.validate_for_authoring().is_err(), "{version}");
        }
    }

    /// A pulse-ox revision as the v1.0.2 editor saved it: an unused
    /// "Channel 1 / A0" plus the legacy `led_outputs` next to the outputs.
    fn v1_0_2_pulse_ox_revision() -> AcquisitionProfile {
        let mut pulse = legacy_pulse_ox_lab();
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
            split_by_led_state: false,
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
        assert_eq!(store.list().unwrap_or_default().len(), factory_lab_count());
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
        let mut acquisition = factory_lab(ECG_ID).acquisition;
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

    fn written_catalog(root: &Path) -> serde_json::Value {
        serde_json::from_slice(&fs::read(root.join(CATALOG_FILE)).unwrap_or_default())
            .unwrap_or_else(|error| panic!("{error}"))
    }

    fn stored_entries<'a>(catalog: &'a serde_json::Value, id: &str) -> Vec<&'a serde_json::Value> {
        catalog["profiles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|entry| entry["profile_id"] == id)
            .collect()
    }

    fn save_an_emg_revision(store: &ProfileStore, request_id: &str) {
        let mut emg = store
            .begin_lab_edit(EMG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        emg.description.push_str(" edited");
        let base = emg.profile_version.clone();
        store
            .save_lab_draft(emg, Some(base), request_id.into())
            .unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    fn a_stored_lab_with_a_course_default_identity_but_other_content_is_kept_and_reported() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let course_default = factory_lab(ECG_ID);
        let mut colliding = course_default.clone();
        colliding.source = ProfileSource::Instructor;
        colliding
            .description
            .push_str(" A local lab saved under the course default's version.");
        colliding
            .refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(colliding.validate().is_ok());
        write_catalog(
            &root,
            &[serde_json::to_string_pretty(&colliding).unwrap_or_default()],
            &[(ECG_ID, colliding.profile_version.as_str())],
        );

        let store = instructor_store(&root);
        // The course default stays the lab students see.
        let active = store
            .get_locked(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(active, course_default);
        let status = store
            .catalog_status()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(status.writable);
        assert_eq!(status.skipped_labs.len(), 1);
        let skipped = &status.skipped_labs[0];
        assert_eq!(skipped.profile_id.as_deref(), Some(ECG_ID));
        assert_eq!(
            skipped.profile_version.as_deref(),
            Some(course_default.profile_version.as_str())
        );
        assert!(
            skipped.reason.contains(&format!(
                "\"{}\" {}",
                course_default.display_name, course_default.profile_version
            )),
            "{}",
            skipped.reason
        );
        assert!(skipped.reason.contains("different content"));

        // An instructor save writes the stored lab back unchanged.
        save_an_emg_revision(&store, "collision-save-01");
        let written = written_catalog(&root);
        let kept = stored_entries(&written, ECG_ID);
        assert_eq!(kept.len(), 1);
        assert_eq!(
            *kept[0],
            serde_json::to_value(&colliding).unwrap_or_default()
        );
        // Restoring the course default and editing it never erases it either.
        store
            .restore_course_default(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        let mut ecg = store
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        ecg.description.push_str(" edited");
        let saved = store
            .save_lab_draft(
                ecg,
                Some(course_default.profile_version.clone()),
                "collision-save-02".into(),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        assert_ne!(saved.profile_version, course_default.profile_version);
        let reopened = instructor_store(&root);
        assert_eq!(
            reopened
                .catalog_status()
                .unwrap_or_else(|error| panic!("{error}"))
                .skipped_labs
                .len(),
            1
        );
        assert_eq!(stored_entries(&written_catalog(&root), ECG_ID).len(), 2);
    }

    #[test]
    fn a_stored_copy_of_a_course_default_is_dropped_quietly() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        // For example a lab installed locally and later shipped as a factory lab.
        let mut copy = factory_lab(BP_ID);
        copy.source = ProfileSource::Imported;
        copy.refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        let mut team = factory_lab(ECG_ID);
        team.profile_id = "team.ecg".into();
        team.source = ProfileSource::Instructor;
        team.refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        write_catalog(
            &root,
            &[
                serde_json::to_string_pretty(&copy).unwrap_or_default(),
                serde_json::to_string_pretty(&team).unwrap_or_default(),
            ],
            &[
                (BP_ID, copy.profile_version.as_str()),
                ("team.ecg", "1.0.0"),
            ],
        );

        let store = instructor_store(&root);
        assert!(store
            .catalog_status()
            .unwrap_or_else(|error| panic!("{error}"))
            .skipped_labs
            .is_empty());
        assert_eq!(
            store
                .get_locked(BP_ID)
                .unwrap_or_else(|error| panic!("{error}"))
                .source,
            ProfileSource::BuiltIn
        );
        assert!(store.get_locked("team.ecg").is_ok());
        assert_eq!(
            store.list_all().unwrap_or_default().len(),
            factory_lab_count() + 1
        );
        // The next write drops the copy and keeps every other stored lab.
        save_an_emg_revision(&store, "promoted-save-01");
        let written = written_catalog(&root);
        assert!(stored_entries(&written, BP_ID).is_empty());
        assert_eq!(stored_entries(&written, "team.ecg").len(), 1);
        assert_eq!(stored_entries(&written, EMG_ID).len(), 1);
    }

    #[test]
    fn a_stored_built_in_lab_that_is_not_a_course_default_survives_a_write() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("profiles");
        let mut older = factory_lab(ECG_ID);
        older.profile_version = "0.9.0".into();
        older
            .refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(older.source, ProfileSource::BuiltIn);
        write_catalog(
            &root,
            &[serde_json::to_string_pretty(&older).unwrap_or_default()],
            &[],
        );
        let store = instructor_store(&root);
        save_an_emg_revision(&store, "built-in-keep-01");
        let written = written_catalog(&root);
        let kept = stored_entries(&written, ECG_ID);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0]["profile_version"], "0.9.0");
    }

    /// A copy of the repository's factory lab folder (the compiled-in files,
    /// the lock, and the published versions kept in `legacy/`).
    fn factory_lab_folder(root: &Path) -> PathBuf {
        let dir = root.join("profiles");
        fs::create_dir_all(&dir).unwrap_or_else(|error| panic!("{error}"));
        for (name, text) in FACTORY_LAB_FILES {
            fs::write(dir.join(name), text).unwrap_or_else(|error| panic!("{error}"));
        }
        fs::write(
            dir.join(FACTORY_LAB_LOCK_FILE),
            include_str!("../../profiles/factory-labs.lock.json"),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        let legacy = dir.join(FACTORY_LAB_LEGACY_FOLDER);
        fs::create_dir_all(&legacy).unwrap_or_else(|error| panic!("{error}"));
        for path in repository_legacy_lab_files() {
            fs::copy(&path, legacy.join(file_name_of(&path)))
                .unwrap_or_else(|error| panic!("{error}"));
        }
        dir
    }

    /// The published factory lab versions kept in the repository's
    /// `profiles/legacy/`.
    fn repository_legacy_lab_files() -> Vec<PathBuf> {
        factory_lab_files_in(&repository_profiles_dir().join(FACTORY_LAB_LEGACY_FOLDER))
            .unwrap_or_else(|error| panic!("{error}"))
    }

    /// The folder's lab files as the compiled-in list a rebuild would give.
    fn compiled_from(dir: &Path) -> Vec<(String, String)> {
        factory_lab_files_in(dir)
            .unwrap_or_else(|error| panic!("{error}"))
            .into_iter()
            .map(|path| {
                (
                    file_name_of(&path),
                    fs::read_to_string(&path).unwrap_or_default(),
                )
            })
            .collect()
    }

    fn check_rebuilt(dir: &Path) -> FactoryLabReport {
        let compiled = compiled_from(dir);
        let sources: Vec<(&str, &str)> = compiled
            .iter()
            .map(|(name, text)| (name.as_str(), text.as_str()))
            .collect();
        factory_lab_check(dir, &sources)
    }

    fn write_lab_file(path: &Path, profile: &AcquisitionProfile) {
        fs::write(
            path,
            factory_lab_file_text(profile, "\n").unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
    }

    fn file_text(path: &Path) -> String {
        fs::read_to_string(path).unwrap_or_else(|error| panic!("{error}"))
    }

    const ECG_FILE: &str = "ecg_course_capture.profile.json";
    const EMG_FILE: &str = "emg_force_course_capture.profile.json";

    #[test]
    fn factory_lab_check_reports_each_kind_of_problem() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let clean = factory_lab_folder(&dir.path().join("clean"));
        let report = factory_lab_check(&clean, FACTORY_LAB_FILES);
        assert!(report.is_ok(), "{:#?}", report.problems);
        let legacy_files = repository_legacy_lab_files();
        assert!(!legacy_files.is_empty());
        assert_eq!(report.labs.len(), factory_lab_count() + legacy_files.len());
        assert!(report.labs.iter().any(
            |line| line.starts_with("legacy/pulseox_tx_rx_course_capture_1.0.0.profile.json: ")
        ));

        let mut case = 0;
        let mut problems_after = |edit: &dyn Fn(&Path)| -> Vec<String> {
            case += 1;
            let folder = factory_lab_folder(&dir.path().join(format!("case-{case}")));
            edit(&folder);
            factory_lab_check(&folder, FACTORY_LAB_FILES).problems
        };
        let has = |problems: &[String], text: &str| problems.iter().any(|p| p.contains(text));

        // A hand edit without re-hashing.
        let problems = problems_after(&|folder| {
            let path = folder.join(ECG_FILE);
            let text = file_text(&path).replace("Synchronized raw", "Synchronised raw");
            fs::write(&path, text).unwrap_or_else(|error| panic!("{error}"));
        });
        assert!(has(&problems, "hash mismatch"), "{problems:#?}");
        assert!(has(&problems, "compiled-in copy differs"), "{problems:#?}");

        // Re-hashed, but a published version changed its content.
        let problems = problems_after(&|folder| {
            let mut ecg = factory_lab(ECG_ID);
            ecg.description.push_str(" Changed.");
            ecg.refresh_hash().unwrap_or_else(|error| panic!("{error}"));
            write_lab_file(&folder.join(ECG_FILE), &ecg);
        });
        assert!(has(&problems, "was published with hash"), "{problems:#?}");

        // A new file that is neither compiled in nor recorded in the lock.
        let problems = problems_after(&|folder| {
            let mut extra = factory_lab(ECG_ID);
            extra.profile_id = "team.extra".into();
            extra
                .refresh_hash()
                .unwrap_or_else(|error| panic!("{error}"));
            write_lab_file(&folder.join("extra.profile.json"), &extra);
        });
        assert!(
            has(&problems, "factory_lab_file!(\"extra.profile.json\"),"),
            "{problems:#?}"
        );
        assert!(
            has(&problems, "team.extra 1.0.0 is not recorded"),
            "{problems:#?}"
        );

        // A local-revision version, the wrong source, and a duplicate lab ID.
        let problems = problems_after(&|folder| {
            let mut ecg = factory_lab(ECG_ID);
            ecg.profile_version = "1.0.1".into();
            ecg.source = ProfileSource::Instructor;
            ecg.refresh_hash().unwrap_or_else(|error| panic!("{error}"));
            write_lab_file(&folder.join(ECG_FILE), &ecg);
            write_lab_file(&folder.join(EMG_FILE), &ecg);
        });
        assert!(has(&problems, "not MAJOR.MINOR.0"), "{problems:#?}");
        assert!(
            has(&problems, "source must be \"built_in\""),
            "{problems:#?}"
        );
        assert!(has(&problems, "is also used by"), "{problems:#?}");

        // A compiled-in file that is missing, and a conflicting lock entry.
        let problems = problems_after(&|folder| {
            fs::remove_file(folder.join(EMG_FILE)).unwrap_or_else(|error| panic!("{error}"));
            let lock_path = folder.join(FACTORY_LAB_LOCK_FILE);
            let mut lock =
                read_factory_lab_lock(&lock_path).unwrap_or_else(|error| panic!("{error}"));
            let mut conflict = lock[1].clone();
            conflict.canonical_hash = "f".repeat(64);
            lock.push(conflict);
            fs::write(
                &lock_path,
                factory_lab_lock_text(&lock, "\n").unwrap_or_else(|error| panic!("{error}")),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        });
        assert!(
            has(
                &problems,
                &format!("{EMG_FILE}: compiled into the application but missing")
            ),
            "{problems:#?}"
        );
        assert!(
            has(&problems, "listed twice with different hashes"),
            "{problems:#?}"
        );
        // The published EMG 1.0.0 is no longer held by any file.
        assert!(
            has(
                &problems,
                &format!("{EMG_ID} 1.0.0 is published, but no file in profiles/ or profiles/legacy/ holds it")
            ),
            "{problems:#?}"
        );

        // A published version kept in legacy/ under the tool's name is held.
        let problems = problems_after(&|folder| {
            let legacy = folder.join(FACTORY_LAB_LEGACY_FOLDER);
            fs::create_dir_all(&legacy).unwrap_or_else(|error| panic!("{error}"));
            fs::rename(
                folder.join(EMG_FILE),
                legacy.join(legacy_factory_lab_file_name(EMG_FILE, "1.0.0")),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        });
        assert_eq!(problems.len(), 1, "{problems:#?}");
        assert!(
            has(
                &problems,
                &format!("{EMG_FILE}: compiled into the application but missing")
            ),
            "{problems:#?}"
        );

        // A factory lab that goes back to an older version than one published.
        let problems = problems_after(&|folder| {
            let lock_path = folder.join(FACTORY_LAB_LOCK_FILE);
            let mut lock =
                read_factory_lab_lock(&lock_path).unwrap_or_else(|error| panic!("{error}"));
            lock.push(FactoryLabLockEntry {
                profile_id: ECG_ID.into(),
                profile_version: "1.1.0".into(),
                canonical_hash: "a".repeat(64),
            });
            fs::write(
                &lock_path,
                factory_lab_lock_text(&lock, "\n").unwrap_or_else(|error| panic!("{error}")),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        });
        assert!(has(&problems, "never goes back"), "{problems:#?}");

        // A legacy file must match its lock entry, and a missing lock is reported.
        let problems = problems_after(&|folder| {
            let legacy = folder.join(FACTORY_LAB_LEGACY_FOLDER);
            fs::create_dir_all(&legacy).unwrap_or_else(|error| panic!("{error}"));
            let mut old = factory_lab(EMG_ID);
            old.profile_version = "0.9.0".into();
            old.refresh_hash().unwrap_or_else(|error| panic!("{error}"));
            write_lab_file(&legacy.join("emg_0.9.0.profile.json"), &old);
        });
        assert!(
            has(&problems, "legacy/emg_0.9.0.profile.json: wvu.bmeg420l.emg.force.course.capture.v1 0.9.0 is not recorded"),
            "{problems:#?}"
        );
        let problems = problems_after(&|folder| {
            fs::remove_file(folder.join(FACTORY_LAB_LOCK_FILE))
                .unwrap_or_else(|error| panic!("{error}"));
        });
        assert!(has(&problems, FACTORY_LAB_LOCK_FILE), "{problems:#?}");
    }

    #[test]
    fn fix_rehashes_a_hand_edit_and_only_publishes_a_change_as_a_new_minor_version() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let folder = factory_lab_folder(dir.path());
        let lab_path = folder.join(ECG_FILE);
        let lock_path = folder.join(FACTORY_LAB_LOCK_FILE);
        let original_lock = file_text(&lock_path);
        let original = file_text(&lab_path);
        let published = factory_lab(ECG_ID);
        let legacy_path = folder
            .join(FACTORY_LAB_LEGACY_FOLDER)
            .join("ecg_course_capture_1.0.0.profile.json");

        // Content changed at the published 1.0.0: refused without --bump-minor.
        let edited = file_text(&lab_path).replace("Synchronized raw", "Synchronised raw");
        fs::write(&lab_path, &edited).unwrap_or_else(|error| panic!("{error}"));
        let refused = rehash_factory_file(&lab_path, &lock_path, false, &no_git)
            .err()
            .unwrap_or_else(|| panic!("a published version must not change"));
        assert!(refused.to_string().contains("--bump-minor"), "{refused}");
        assert!(refused.to_string().contains("1.1.0"), "{refused}");
        assert_eq!(file_text(&lab_path), edited);
        assert_eq!(file_text(&lock_path), original_lock);

        // The edit replaced the published 1.0.0 in the file, so --bump-minor
        // needs that version from Git; without it nothing is written.
        let refused = rehash_factory_file(&lab_path, &lock_path, true, &no_git)
            .err()
            .unwrap_or_else(|| panic!("the published 1.0.0 must not be lost"));
        assert!(refused.to_string().contains("would lose it"), "{refused}");
        assert_eq!(file_text(&lab_path), edited);
        assert_eq!(file_text(&lock_path), original_lock);
        assert!(!legacy_path.exists());

        // --bump-minor publishes 1.1.0; only the version and hash text change,
        // and the committed 1.0.0 is kept in legacy/ byte for byte.
        let committed = |name: &str| (name == ECG_FILE).then(|| original.clone());
        let bumped = rehash_factory_file(&lab_path, &lock_path, true, &committed)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(bumped.previous_version, "1.0.0");
        assert_eq!(bumped.profile_version, "1.1.0");
        assert!(bumped.file_changed && bumped.lock_appended);
        assert_eq!(bumped.kept_files, std::slice::from_ref(&legacy_path));
        assert_eq!(file_text(&legacy_path), original);
        let expected = edited
            .replace(
                "\"profile_version\": \"1.0.0\"",
                "\"profile_version\": \"1.1.0\"",
            )
            .replace(&published.integrity.canonical_hash, &bumped.canonical_hash);
        assert_eq!(file_text(&lab_path), expected);
        let saved = parse_lab_text(&file_text(&lab_path)).unwrap_or_else(|error| panic!("{error}"));
        assert!(saved.validate_for_authoring().is_ok());
        assert_eq!(saved.integrity.canonical_hash, bumped.canonical_hash);
        assert!(saved.description.starts_with("Synchronised raw"));
        // The lock keeps every earlier line and gains one entry.
        let lock_text = file_text(&lock_path);
        let earlier = original_lock.trim_end().trim_end_matches(']').trim_end();
        assert!(lock_text.starts_with(earlier), "{lock_text}");
        let lock = parse_factory_lab_lock(&lock_text).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(lock.len(), factory_lock().len() + 1);
        assert_eq!(lock[..lock.len() - 1], factory_lock()[..]);
        assert_eq!(
            lock.last(),
            Some(&FactoryLabLockEntry {
                profile_id: ECG_ID.into(),
                profile_version: "1.1.0".into(),
                canonical_hash: bumped.canonical_hash.clone(),
            })
        );
        assert!(
            check_rebuilt(&folder).is_ok(),
            "{:#?}",
            check_rebuilt(&folder).problems
        );

        // Nothing left to do: neither a plain fix nor --bump-minor changes anything.
        for bump in [false, true] {
            let again = rehash_factory_file(&lab_path, &lock_path, bump, &no_git)
                .unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(again.profile_version, "1.1.0");
            assert!(!again.file_changed && !again.lock_appended);
            assert!(again.kept_files.is_empty());
        }
        assert_eq!(file_text(&lab_path), expected);

        // Another edit of 1.1.0 needs 1.2.0.
        fs::write(
            &lab_path,
            expected.replace("Synchronised raw", "Synchronized raw"),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        let refused = rehash_factory_file(&lab_path, &lock_path, false, &no_git)
            .err()
            .unwrap_or_else(|| panic!("1.1.0 is published"));
        assert!(refused.to_string().contains("1.2.0"), "{refused}");

        // A damaged hash over published content is simply corrected.
        let emg_path = folder.join(EMG_FILE);
        let emg_original = file_text(&emg_path);
        let emg_hash = factory_lab(EMG_ID).integrity.canonical_hash;
        fs::write(&emg_path, emg_original.replace(&emg_hash, &"0".repeat(64)))
            .unwrap_or_else(|error| panic!("{error}"));
        let repaired = rehash_factory_file(&emg_path, &lock_path, false, &no_git)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(repaired.file_changed && !repaired.lock_appended);
        assert_eq!(repaired.profile_version, "1.0.0");
        assert!(repaired.kept_files.is_empty());
        assert_eq!(file_text(&emg_path), emg_original);

        // Only factory lab files are re-hashed.
        let mut local = factory_lab(EMG_ID);
        local.source = ProfileSource::Instructor;
        local
            .refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        write_lab_file(&emg_path, &local);
        let refused = rehash_factory_file(&emg_path, &lock_path, true, &no_git)
            .err()
            .unwrap_or_else(|| panic!("an instructor lab is not a factory file"));
        assert!(refused.to_string().contains("promote"), "{refused}");
    }

    #[test]
    fn fix_records_a_new_factory_version_and_refuses_a_local_revision_number() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let folder = factory_lab_folder(dir.path());
        let lock_path = folder.join(FACTORY_LAB_LOCK_FILE);
        let path = folder.join("team_lab.profile.json");
        let mut lab = factory_lab(ECG_ID);
        lab.profile_id = "team.lab".into();
        lab.display_name = "Team Lab".into();
        lab.integrity.canonical_hash = "stale".into();
        write_lab_file(&path, &lab);
        let recorded = rehash_factory_file(&path, &lock_path, false, &no_git)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(recorded.profile_version, "1.0.0");
        assert!(recorded.file_changed && recorded.lock_appended);
        assert!(recorded.kept_files.is_empty());
        let lock = read_factory_lab_lock(&lock_path).unwrap_or_else(|error| panic!("{error}"));
        assert!(lock
            .iter()
            .any(|entry| entry.profile_id == "team.lab" && entry.profile_version == "1.0.0"));
        // As committed after that fix.
        let committed_1_0_0 = file_text(&path);

        let mut revision = lab.clone();
        revision.profile_version = "1.0.3".into();
        revision.description.push_str(" Revised.");
        write_lab_file(&path, &revision);
        let refused = rehash_factory_file(&path, &lock_path, false, &no_git)
            .err()
            .unwrap_or_else(|| panic!("1.0.3 is a local revision number"));
        assert!(refused.to_string().contains("MAJOR.MINOR.0"), "{refused}");
        let committed = |_: &str| Some(committed_1_0_0.clone());
        let bumped = rehash_factory_file(&path, &lock_path, true, &committed)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(bumped.profile_version, "1.1.0");
        assert!(bumped.lock_appended);
        let kept = folder
            .join(FACTORY_LAB_LEGACY_FOLDER)
            .join("team_lab_1.0.0.profile.json");
        assert_eq!(bumped.kept_files, std::slice::from_ref(&kept));
        assert_eq!(file_text(&kept), committed_1_0_0);
    }

    /// Saves an edited copy of `from` in a scratch instructor catalog and
    /// exports it as Lab Manager would.
    fn exported_lab(
        root: &Path,
        from: &str,
        new_id: Option<&str>,
        edit: impl FnOnce(&mut AcquisitionProfile),
    ) -> PathBuf {
        let store = instructor_store(&root.join("catalog"));
        let (mut draft, base) = match new_id {
            Some(id) => (
                store
                    .duplicate_lab(from, id)
                    .unwrap_or_else(|error| panic!("{error}")),
                None,
            ),
            None => {
                let draft = store
                    .begin_lab_edit(from)
                    .unwrap_or_else(|error| panic!("{error}"));
                let base = draft.profile_version.clone();
                (draft, Some(base))
            }
        };
        edit(&mut draft);
        let saved = store
            .save_lab_draft(draft, base, "promote-export-01".into())
            .unwrap_or_else(|error| panic!("{error}"));
        let path = root.join(format!("{}.lab.json", saved.profile_id));
        store
            .export_profile(&saved.profile_id, &saved.profile_version, &path)
            .unwrap_or_else(|error| panic!("{error}"));
        path
    }

    #[test]
    fn promote_turns_a_lab_manager_export_into_the_next_factory_version() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let folder = factory_lab_folder(&dir.path().join("repo"));
        let lock_path = folder.join(FACTORY_LAB_LOCK_FILE);

        // A revised course lab replaces its factory file as the next MINOR,
        // and the published 1.0.0 file is kept in legacy/ unchanged.
        let published_text = file_text(&folder.join(ECG_FILE));
        let export = exported_lab(&dir.path().join("ecg"), ECG_ID, None, |draft| {
            draft.description = "Revised ECG course capture.".into();
        });
        let promoted = promote_lab_file(&export, &folder, &lock_path, None, &no_git)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(promoted.file_name, ECG_FILE);
        assert_eq!(promoted.profile_version, "1.1.0");
        assert_eq!(promoted.replaced_version.as_deref(), Some("1.0.0"));
        let legacy_ecg = folder
            .join(FACTORY_LAB_LEGACY_FOLDER)
            .join("ecg_course_capture_1.0.0.profile.json");
        assert_eq!(promoted.kept_files, std::slice::from_ref(&legacy_ecg));
        assert_eq!(file_text(&legacy_ecg), published_text);
        let file =
            parse_lab_text(&file_text(&promoted.path)).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(file.source, ProfileSource::BuiltIn);
        assert_eq!(file.status, ProfileStatus::Locked);
        assert_eq!(file.description, "Revised ECG course capture.");
        assert_eq!(file.integrity.canonical_hash, promoted.canonical_hash);
        assert!(file.validate_for_authoring().is_ok());
        assert!(normalized_text(&file_text(&promoted.path))
            .starts_with("{\n  \"schema_version\": 1,\n"));
        assert!(read_factory_lab_lock(&lock_path)
            .unwrap_or_else(|error| panic!("{error}"))
            .contains(&FactoryLabLockEntry::of(&file)));
        assert!(
            check_rebuilt(&folder).is_ok(),
            "{:#?}",
            check_rebuilt(&folder).problems
        );
        // The same content again is not a new version.
        let again = promote_lab_file(&export, &folder, &lock_path, None, &no_git)
            .err()
            .unwrap_or_else(|| panic!("identical content must not be promoted"));
        assert!(again.to_string().contains("same content"), "{again}");

        // A new lab, saved locally as 1.0.0, becomes 1.1.0 (one MINOR above
        // its local revisions) in a file named after it, not yet compiled in.
        let export = exported_lab(
            &dir.path().join("strain-gauge"),
            EMG_ID,
            Some("team.strain_gauge"),
            |draft| {
                draft.display_name = "Strain Gauge — Course Capture".into();
            },
        );
        assert_eq!(
            parse_lab_text(&file_text(&export))
                .unwrap_or_else(|error| panic!("{error}"))
                .profile_version,
            "1.0.0"
        );
        let new_lab = promote_lab_file(&export, &folder, &lock_path, None, &no_git)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            new_lab.file_name,
            "strain_gauge_course_capture.profile.json"
        );
        assert_eq!(new_lab.profile_version, "1.1.0");
        assert_eq!(new_lab.replaced_version, None);
        assert!(new_lab.kept_files.is_empty());
        let report = factory_lab_check(&folder, &[]);
        assert!(report
            .problems
            .iter()
            .any(|problem| problem.contains(&factory_lab_source_line(
                "strain_gauge_course_capture.profile.json"
            ))));
        assert!(
            check_rebuilt(&folder).is_ok(),
            "{:#?}",
            check_rebuilt(&folder).problems
        );

        // A file name that belongs to another lab is refused.
        let export = exported_lab(
            &dir.path().join("other"),
            ECG_ID,
            Some("team.other"),
            |_| {},
        );
        let clash = promote_lab_file(
            &export,
            &folder,
            &lock_path,
            Some("emg_force_course_capture"),
            &no_git,
        )
        .err()
        .unwrap_or_else(|| panic!("another lab's file must not be replaced"));
        assert!(clash.to_string().contains("another lab"), "{clash}");
        // A damaged export is refused before anything is written.
        let text = file_text(&export).replace("team.other", "team.othe");
        fs::write(&export, text).unwrap_or_else(|error| panic!("{error}"));
        let damaged = promote_lab_file(&export, &folder, &lock_path, None, &no_git)
            .err()
            .unwrap_or_else(|| panic!("a damaged export must be refused"));
        assert!(damaged.to_string().contains("hash mismatch"), "{damaged}");
        assert!(check_rebuilt(&folder).is_ok());

        // A factory file edited by hand no longer holds its published
        // version: promote keeps that version from Git, or refuses.
        let lock_before = file_text(&lock_path);
        let emg_path = folder.join(EMG_FILE);
        let emg_published = file_text(&emg_path);
        // Edited without re-hashing: the file still says 1.0.0.
        let mut hand_edited = factory_lab(EMG_ID);
        hand_edited.description.push_str(" Hand edit.");
        write_lab_file(&emg_path, &hand_edited);
        let edited_text = file_text(&emg_path);
        let export = exported_lab(&dir.path().join("emg"), EMG_ID, None, |draft| {
            draft.description = "Revised EMG course capture.".into();
        });
        let refused = promote_lab_file(&export, &folder, &lock_path, None, &no_git)
            .err()
            .unwrap_or_else(|| panic!("the published EMG 1.0.0 must not be lost"));
        assert!(refused.to_string().contains("would lose it"), "{refused}");
        assert_eq!(file_text(&emg_path), edited_text);
        assert_eq!(file_text(&lock_path), lock_before);
        let committed = |name: &str| (name == EMG_FILE).then(|| emg_published.clone());
        let promoted = promote_lab_file(&export, &folder, &lock_path, None, &committed)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(promoted.profile_version, "1.1.0");
        let legacy_emg = folder
            .join(FACTORY_LAB_LEGACY_FOLDER)
            .join("emg_force_course_capture_1.0.0.profile.json");
        assert_eq!(promoted.kept_files, std::slice::from_ref(&legacy_emg));
        assert_eq!(file_text(&legacy_emg), emg_published);
        assert!(
            check_rebuilt(&folder).is_ok(),
            "{:#?}",
            check_rebuilt(&folder).problems
        );
    }

    #[test]
    fn factory_lab_files_and_the_lock_are_written_in_the_hand_written_layout() {
        for profile in built_in_profiles().unwrap_or_else(|error| panic!("{error}")) {
            let text =
                factory_lab_file_text(&profile, "\n").unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(
                parse_lab_text(&text).unwrap_or_else(|error| panic!("{error}")),
                profile
            );
            assert!(text.starts_with("{\n  \"schema_version\": 1,\n  \"profile_id\": "));
            assert!(text.contains(
                "\n  \"required_firmware\": { \"protocol_major\": 0, \"protocol_minor_min\": 4, \"build\": "
            ));
            assert!(text.contains("\n  \"acquisition\": {\n    \"analog_pin\": \"A0\",\n"));
            assert!(text.ends_with("\n}\n"));
            let crlf =
                factory_lab_file_text(&profile, "\r\n").unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(crlf.replace("\r\n", "\n"), text);
        }
        let bp = factory_lab_file_text(&factory_lab(BP_ID), "\n").unwrap_or_default();
        assert!(bp.contains(
            "\n    \"channels\": [\n      { \"pin\": \"A0\", \"id\": \"ppg\", \"label\": \"PPG\", "
        ));
        assert!(bp.contains("\"allowed_conversions\": [\"counts_volts\"], "));
        // The committed lock is exactly what the tool writes.
        assert_eq!(
            factory_lab_lock_text(&factory_lock(), "\n").unwrap_or_default(),
            normalized_text(include_str!("../../profiles/factory-labs.lock.json"))
        );
        // A value is replaced only when its key occurs exactly once.
        assert_eq!(
            replace_json_string_value("{ \"a\": \"1\", \"b\" : \"2\" }", "b", "3").as_deref(),
            Some("{ \"a\": \"1\", \"b\" : \"3\" }")
        );
        assert_eq!(
            replace_json_string_value("{ \"a\": \"1\", \"x\": { \"a\": \"2\" } }", "a", "3"),
            None
        );
        assert_eq!(
            factory_lab_file_stem("Op Amp — Course Capture"),
            "op_amp_course_capture"
        );
        assert_eq!(
            factory_lab_file_stem("ECG — Course Capture"),
            "ecg_course_capture"
        );
        assert_eq!(factory_lab_file_stem(" — "), "lab");
    }

    fn audit_log(root: &Path) -> String {
        fs::read_to_string(root.join("lab_write_audit.log")).unwrap_or_default()
    }

    fn catalog_bytes(root: &Path) -> Vec<u8> {
        fs::read(root.join(CATALOG_FILE)).unwrap_or_default()
    }

    /// Writes a lab to `path` the way Export writes a `.lab.json` file.
    fn lab_file(path: &Path, profile: &AcquisitionProfile) -> PathBuf {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap_or_else(|error| panic!("{error}"));
        }
        fs::write(
            path,
            serde_json::to_vec_pretty(profile).unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        path.to_path_buf()
    }

    #[test]
    fn save_as_course_default_file_writes_what_promote_makes_of_the_same_revision() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("catalog");
        let store = instructor_store(&root);
        let mut draft = store
            .begin_lab_edit(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        draft.description = "Revised ECG course capture.".into();
        let saved = store
            .save_lab_draft(draft, Some("1.0.0".into()), "course-default-01".into())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(saved.profile_version, "1.0.1");
        // Lab Manager suggests the lab's own factory file name.
        let entries = store.list_all().unwrap_or_else(|error| panic!("{error}"));
        assert!(entries
            .iter()
            .filter(|entry| entry.profile.profile_id == ECG_ID)
            .all(|entry| entry.course_default_file_name == ECG_FILE));
        let catalog_before = catalog_bytes(&root);

        let destination = dir.path().join("out").join(ECG_FILE);
        fs::create_dir_all(dir.path().join("out")).unwrap_or_else(|error| panic!("{error}"));
        let outcome = store
            .export_course_default(ECG_ID, "1.0.1", &destination)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(outcome.profile_version, "1.1.0");
        assert_eq!(outcome.replaces_version.as_deref(), Some("1.0.0"));
        assert_eq!(outcome.factory_file_name.as_deref(), Some(ECG_FILE));
        assert_eq!(outcome.file_name, ECG_FILE);
        assert_eq!(outcome.source_line, factory_lab_source_line(ECG_FILE));
        assert_eq!(outcome.path, destination.display().to_string());
        let text = file_text(&destination);
        let file = parse_lab_text(&text).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(file.source, ProfileSource::BuiltIn);
        assert_eq!(file.status, ProfileStatus::Locked);
        assert_eq!(file.profile_version, "1.1.0");
        assert_eq!(file.description, "Revised ECG course capture.");
        assert_eq!(file.integrity.canonical_hash, outcome.canonical_hash);
        assert!(file.verify_integrity().is_ok());
        assert!(file.validate_for_authoring().is_ok());
        assert_eq!(text, factory_lab_file_text(&file, "\n").unwrap_or_default());
        assert!(file_names(&dir.path().join("out"))
            .iter()
            .all(|name| name == ECG_FILE));

        // `promote` makes the same file from an export of the same revision.
        let export = dir.path().join("ecg.lab.json");
        store
            .export_profile(ECG_ID, "1.0.1", &export)
            .unwrap_or_else(|error| panic!("{error}"));
        let folder = factory_lab_folder(&dir.path().join("repo"));
        let promoted = promote_lab_file(
            &export,
            &folder,
            &folder.join(FACTORY_LAB_LOCK_FILE),
            None,
            &no_git,
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(normalized_text(&file_text(&promoted.path)), text);

        // The documented next steps: copy the file over the factory file;
        // `check` asks for `fix` and reports that no file holds the published
        // 1.0.0 any more; `fix` records the new version and keeps 1.0.0 in
        // legacy/ from the committed copy; then the check passes.
        let fresh = factory_lab_folder(&dir.path().join("fresh"));
        let committed_ecg = file_text(&fresh.join(ECG_FILE));
        fs::copy(&destination, fresh.join(ECG_FILE)).unwrap_or_else(|error| panic!("{error}"));
        let report = check_rebuilt(&fresh);
        for expected in [
            "is not recorded in".to_string(),
            format!(
                "{ECG_ID} 1.0.0 is published, but no file in profiles/ or profiles/legacy/ holds it"
            ),
            "profiles/legacy/ecg_course_capture_1.0.0.profile.json".to_string(),
        ] {
            assert!(
                report
                    .problems
                    .iter()
                    .any(|problem| problem.contains(&expected)),
                "{expected}: {:#?}",
                report.problems
            );
        }
        let committed = |name: &str| (name == ECG_FILE).then(|| committed_ecg.clone());
        let fixed = rehash_factory_file(
            &fresh.join(ECG_FILE),
            &fresh.join(FACTORY_LAB_LOCK_FILE),
            false,
            &committed,
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert!(fixed.lock_appended);
        assert!(!fixed.file_changed);
        assert_eq!(fixed.kept_files.len(), 1);
        assert_eq!(file_text(&fixed.kept_files[0]), committed_ecg);
        assert!(
            check_rebuilt(&fresh).is_ok(),
            "{:#?}",
            check_rebuilt(&fresh).problems
        );

        // Saving the file never changes the catalog, and it is audited.
        assert_eq!(catalog_bytes(&root), catalog_before);
        assert!(
            audit_log(&root).contains(&format!(
                "\tLAB_WRITE\tcourse_default_export\t{ECG_ID}\t1.0.1\t1.1.0\trequest=explicit"
            )),
            "{}",
            audit_log(&root)
        );
    }

    #[test]
    fn save_as_course_default_file_is_instructor_only_and_numbers_new_labs_above_their_revisions() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let root = dir.path().join("catalog");
        let out = dir.path().join("out");
        fs::create_dir_all(&out).unwrap_or_else(|error| panic!("{error}"));
        let store = instructor_store(&root);
        let mut draft = store
            .duplicate_lab(EMG_ID, "team.strain_gauge")
            .unwrap_or_else(|error| panic!("{error}"));
        draft.display_name = "Strain Gauge — Course Capture".into();
        let strain = store
            .save_lab_draft(draft, None, "course-default-02".into())
            .unwrap_or_else(|error| panic!("{error}"));
        let mut draft = store
            .duplicate_lab(ECG_ID, "team.ecg")
            .unwrap_or_else(|error| panic!("{error}"));
        draft.description = "Team ECG.".into();
        store
            .save_lab_draft(draft, None, "course-default-03".into())
            .unwrap_or_else(|error| panic!("{error}"));
        let suggested = |id: &str| {
            store
                .list_all()
                .unwrap_or_default()
                .into_iter()
                .find(|entry| entry.profile.profile_id == id)
                .map(|entry| entry.course_default_file_name)
                .unwrap_or_default()
        };
        assert_eq!(
            suggested("team.strain_gauge"),
            "strain_gauge_course_capture.profile.json"
        );
        // A name another factory lab uses is never suggested.
        assert_eq!(suggested("team.ecg"), "ecg_course_capture_2.profile.json");

        // Student mode is refused in Rust and writes nothing.
        store
            .set_mode(ProfileMode::Student, false)
            .unwrap_or_else(|error| panic!("{error}"));
        let destination = out.join("strain_gauge_course_capture.profile.json");
        let refused = store
            .export_course_default("team.strain_gauge", &strain.profile_version, &destination)
            .err()
            .unwrap_or_else(|| panic!("Student mode must not save a course default file"));
        assert!(matches!(refused, ProfileError::StudentMode), "{refused}");
        assert!(!destination.exists());

        store
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        // A new lab ID saved locally as 1.0.0 becomes 1.1.0, so the factory
        // version never has the ID and version of a local revision; it needs
        // its FACTORY_LAB_FILES line.
        assert_eq!(strain.profile_version, "1.0.0");
        let outcome = store
            .export_course_default("team.strain_gauge", &strain.profile_version, &destination)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(outcome.profile_version, "1.1.0");
        assert_eq!(outcome.replaces_version, None);
        assert_eq!(outcome.factory_file_name, None);
        assert_eq!(
            outcome.source_line,
            factory_lab_source_line("strain_gauge_course_capture.profile.json")
        );
        let file =
            parse_lab_text(&file_text(&destination)).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(file.profile_id, "team.strain_gauge");
        assert_eq!(file.profile_version, "1.1.0");
        assert!(file.verify_integrity().is_ok());
        // The usual workflow edits it first (1.0.1, 1.0.2): still 1.1.0, above
        // every local revision of the lab.
        for request in ["course-default-04", "course-default-05"] {
            let mut draft = store
                .begin_lab_edit("team.strain_gauge")
                .unwrap_or_else(|error| panic!("{error}"));
            draft.description.push_str(" Revised.");
            let base = draft.profile_version.clone();
            store
                .save_lab_draft(draft, Some(base), request.into())
                .unwrap_or_else(|error| panic!("{error}"));
        }
        let revised = store
            .export_course_default("team.strain_gauge", "1.0.2", &destination)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(revised.profile_version, "1.1.0");
        let local_versions: Vec<String> = store
            .list_all()
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry.profile.profile_id == "team.strain_gauge")
            .map(|entry| entry.profile.profile_version)
            .collect();
        assert_eq!(local_versions, ["1.0.0", "1.0.1", "1.0.2"]);

        // The shipped course default itself has nothing new to save.
        let same = out.join(ECG_FILE);
        let unchanged = store
            .export_course_default(ECG_ID, "1.0.0", &same)
            .err()
            .unwrap_or_else(|| panic!("the course default itself must be refused"));
        assert!(
            unchanged.to_string().contains("same content"),
            "{unchanged}"
        );
        assert!(!same.exists());
        // The lab-file check reads only *.profile.json files.
        let misnamed = out.join("strain.json");
        let wrong_name = store
            .export_course_default("team.strain_gauge", &strain.profile_version, &misnamed)
            .err()
            .unwrap_or_else(|| panic!("a name without .profile.json must be refused"));
        assert!(
            wrong_name
                .to_string()
                .contains("strain_gauge_course_capture.profile.json"),
            "{wrong_name}"
        );
        assert!(!misnamed.exists());
        assert!(store
            .export_course_default("team.missing", "1.0.0", &out.join("x.profile.json"))
            .is_err());
    }

    #[test]
    fn a_student_installs_a_lab_file_in_place_of_the_active_lab_and_the_instructor_can_undo_it() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let export = exported_lab(&dir.path().join("teacher"), ECG_ID, None, |draft| {
            draft.description = "Week 3 ECG.".into();
        });
        let root = dir.path().join("student");
        let store = ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(store.mode().ok(), Some(ProfileMode::Student));

        let summary = store
            .inspect_lab_file(&export)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(summary.profile_id, ECG_ID);
        assert_eq!(summary.profile_version, "1.0.1");
        assert_eq!(summary.acquisition_mode, AcquisitionMode::Simultaneous);
        assert_eq!(summary.sample_rate_hz, 1_000);
        assert_eq!(summary.channels.len(), 1);
        assert_eq!(summary.channels[0].pin, "A0");
        assert!(!summary.already_installed);
        assert_eq!(summary.refusal, None);
        let replaces = summary
            .replaces
            .unwrap_or_else(|| panic!("the file replaces the ECG course default"));
        assert_eq!(replaces.profile_version, "1.0.0");
        assert!(replaces.course_default);
        assert!(replaces.has_course_default);
        // Inspecting changes nothing.
        assert!(!root.join(CATALOG_FILE).exists());

        let installed = store
            .install_lab_file(&export, false)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(installed.source, ProfileSource::Imported);
        assert_eq!(installed.profile_version, "1.0.1");
        assert!(installed.verify_integrity().is_ok());
        assert_eq!(
            store.get_locked(ECG_ID).map(|lab| lab.profile_version).ok(),
            Some("1.0.1".to_string())
        );
        let catalog = written_catalog(&root);
        let stored = stored_entries(&catalog, ECG_ID);
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0]["source"], "imported");
        assert_eq!(catalog["active_versions"][ECG_ID], "1.0.1");
        assert!(
            audit_log(&root).contains(&format!(
                "\tLAB_WRITE\tstudent_install\t{ECG_ID}\t1.0.0\t1.0.1\trequest=explicit"
            )),
            "{}",
            audit_log(&root)
        );

        // The same file again is reported as already installed.
        let again = store
            .inspect_lab_file(&export)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(again.already_installed);
        assert!(again.replaces.is_none());
        assert!(again
            .refusal
            .as_deref()
            .is_some_and(|refusal| refusal.contains("already installed")));
        let bytes = catalog_bytes(&root);
        let repeat = store
            .install_lab_file(&export, false)
            .err()
            .unwrap_or_else(|| panic!("an installed lab version must not be installed twice"));
        assert!(repeat.to_string().contains("already installed"), "{repeat}");
        assert_eq!(catalog_bytes(&root), bytes);

        // It survives a restart; Restore course default undoes it.
        let restarted =
            ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            restarted
                .get_locked(ECG_ID)
                .map(|lab| lab.profile_version)
                .ok(),
            Some("1.0.1".to_string())
        );
        restarted
            .set_mode(ProfileMode::InstructorAuthoring, true)
            .unwrap_or_else(|error| panic!("{error}"));
        restarted
            .restore_course_default(ECG_ID)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            restarted
                .get_locked(ECG_ID)
                .map(|lab| lab.profile_version)
                .ok(),
            Some("1.0.0".to_string())
        );

        // Installing the file again makes the stored 1.0.1 active again
        // (nothing is added), instead of reporting it as installed.
        let summary = restarted
            .inspect_lab_file(&export)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!summary.already_installed);
        assert_eq!(summary.refusal, None);
        assert_eq!(
            summary.replaces.map(|replaced| replaced.profile_version),
            Some("1.0.0".to_string())
        );
        let reactivated = restarted
            .install_lab_file(&export, false)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(reactivated, installed);
        assert_eq!(
            restarted
                .get_locked(ECG_ID)
                .map(|lab| lab.profile_version)
                .ok(),
            Some("1.0.1".to_string())
        );
        let catalog = written_catalog(&root);
        assert_eq!(stored_entries(&catalog, ECG_ID).len(), 1);
        assert_eq!(catalog["active_versions"][ECG_ID], "1.0.1");
        assert_eq!(
            audit_log(&root)
                .matches(&format!(
                    "\tLAB_WRITE\tstudent_install\t{ECG_ID}\t1.0.0\t1.0.1\trequest=explicit"
                ))
                .count(),
            2
        );
        // A retired version is not installed again.
        restarted
            .retire(ECG_ID, "1.0.1")
            .unwrap_or_else(|error| panic!("{error}"));
        let bytes = catalog_bytes(&root);
        let summary = restarted
            .inspect_lab_file(&export)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!summary.already_installed);
        assert!(summary
            .refusal
            .as_deref()
            .is_some_and(|refusal| refusal.contains("was retired")));
        let refused = restarted
            .install_lab_file(&export, false)
            .err()
            .unwrap_or_else(|| panic!("a retired lab version must not be installed again"));
        assert!(refused.to_string().contains("was retired"), "{refused}");
        assert_eq!(catalog_bytes(&root), bytes);
        assert_eq!(
            restarted
                .get_locked(ECG_ID)
                .map(|lab| lab.profile_version)
                .ok(),
            Some("1.0.0".to_string())
        );

        // A new lab ID is added to the list and replaces nothing; Instructor
        // mode can use the same button.
        let new_lab = exported_lab(
            &dir.path().join("teacher-new"),
            EMG_ID,
            Some("team.strain_gauge"),
            |draft| draft.display_name = "Strain Gauge — Course Capture".into(),
        );
        let summary = restarted
            .inspect_lab_file(&new_lab)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(summary.replaces.is_none());
        assert_eq!(summary.channels.len(), 4);
        restarted
            .install_lab_file(&new_lab, false)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(restarted
            .list()
            .unwrap_or_default()
            .iter()
            .any(|lab| lab.profile_id == "team.strain_gauge"));
        assert_eq!(
            restarted.list().unwrap_or_default().len(),
            factory_lab_count() + 1
        );
    }

    #[test]
    fn install_lab_file_refuses_conflicts_recordings_an_unavailable_catalog_and_the_old_op_amp_file(
    ) {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let first = exported_lab(&dir.path().join("teacher-a"), ECG_ID, None, |draft| {
            draft.description = "Teacher A.".into();
        });
        let other = exported_lab(&dir.path().join("teacher-b"), ECG_ID, None, |draft| {
            draft.description = "Teacher B.".into();
        });
        let root = dir.path().join("student");
        let store = ProfileStore::with_root(root.clone()).unwrap_or_else(|error| panic!("{error}"));

        // A recording blocks the install before anything is read or written.
        let refused = store
            .install_lab_file(&first, true)
            .err()
            .unwrap_or_else(|| panic!("a running recording must block the install"));
        assert!(
            matches!(refused, ProfileError::RecordingActive),
            "{refused}"
        );
        assert!(!root.join(CATALOG_FILE).exists());

        store
            .install_lab_file(&first, false)
            .unwrap_or_else(|error| panic!("{error}"));
        let bytes = catalog_bytes(&root);
        // The same ID and version with other content is refused.
        let summary = store
            .inspect_lab_file(&other)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!summary.already_installed);
        assert!(summary
            .refusal
            .as_deref()
            .is_some_and(|refusal| refusal.contains("different lab")));
        let conflict = store
            .install_lab_file(&other, false)
            .err()
            .unwrap_or_else(|| panic!("a conflicting lab version must be refused"));
        assert!(conflict.to_string().contains("different lab"), "{conflict}");
        assert_eq!(catalog_bytes(&root), bytes);
        assert_eq!(
            store
                .get_locked(ECG_ID)
                .map(|lab| lab.description)
                .unwrap_or_default(),
            "Teacher A."
        );
        // A shipped course default is already installed.
        let factory_file = lab_file(&dir.path().join("factory-bp.json"), &factory_lab(BP_ID));
        let summary = store
            .inspect_lab_file(&factory_file)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(summary.already_installed);
        assert!(store.install_lab_file(&factory_file, false).is_err());
        assert_eq!(catalog_bytes(&root), bytes);
        // A damaged file is refused by the hash check.
        let damaged = dir.path().join("damaged.lab.json");
        fs::write(
            &damaged,
            file_text(&first).replace("Teacher A.", "Teacher C."),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert!(store.inspect_lab_file(&damaged).is_err());
        assert!(store.install_lab_file(&damaged, false).is_err());
        assert_eq!(catalog_bytes(&root), bytes);

        // The older Op Amp Lab file would replace General Analog again.
        for source in [ProfileSource::Imported, ProfileSource::Instructor] {
            let legacy = dir.path().join(format!("op-amp-{source:?}.lab.json"));
            fs::write(&legacy, legacy_op_amp_json(source))
                .unwrap_or_else(|error| panic!("{error}"));
            let summary = store
                .inspect_lab_file(&legacy)
                .unwrap_or_else(|error| panic!("{error}"));
            assert!(summary
                .refusal
                .as_deref()
                .is_some_and(|refusal| refusal.contains("Op Amp Lab is now built in")));
            let refused = store
                .install_lab_file(&legacy, false)
                .err()
                .unwrap_or_else(|| panic!("the older Op Amp Lab file must be refused"));
            assert!(refused.to_string().contains("built in"), "{refused}");
            assert_eq!(catalog_bytes(&root), bytes);
        }
        assert_eq!(
            store
                .get_locked(GENERAL_ID)
                .map(|lab| lab.profile_version)
                .ok(),
            Some(general_default_version())
        );

        // A "Save as course default file…" file is for the next build, not
        // for students: every file saved from one build gets the same
        // version, whatever its content.
        let mut draft_default = factory_lab(ECG_ID);
        draft_default.description = "Draft course default.".into();
        draft_default.profile_version = "1.1.0".into();
        draft_default
            .refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        let draft_file = lab_file(&dir.path().join(ECG_FILE), &draft_default);
        let summary = store
            .inspect_lab_file(&draft_file)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(summary
            .refusal
            .as_deref()
            .is_some_and(|refusal| refusal.contains("course default file")));
        let refused = store
            .install_lab_file(&draft_file, false)
            .err()
            .unwrap_or_else(|| panic!("a course default file must be refused"));
        assert!(
            refused.to_string().contains("course default file"),
            "{refused}"
        );
        assert_eq!(catalog_bytes(&root), bytes);

        // A file that breaks a rule for labs installed now is refused before
        // the student is asked (here a frame rate the firmware lacks).
        let mut unsupported = factory_lab(ECG_ID);
        unsupported.profile_id = "team.ecg_300".into();
        unsupported.source = ProfileSource::Instructor;
        unsupported.acquisition.sample_rate_hz = 300;
        unsupported
            .refresh_hash()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(unsupported.validate().is_ok());
        let unsupported_file = lab_file(&dir.path().join("ecg-300.lab.json"), &unsupported);
        let summary = store
            .inspect_lab_file(&unsupported_file)
            .unwrap_or_else(|error| panic!("{error}"));
        let refusal = summary
            .refusal
            .unwrap_or_else(|| panic!("the rule must be reported before installing"));
        assert!(refusal.contains("does not meet the rules"), "{refusal}");
        assert!(refusal.contains("frame rate"), "{refusal}");
        let refused = store
            .install_lab_file(&unsupported_file, false)
            .err()
            .unwrap_or_else(|| panic!("an unsupported frame rate must be refused"));
        assert_eq!(refused.to_string(), refusal);
        assert_eq!(catalog_bytes(&root), bytes);

        // Without a usable catalog nothing can be installed.
        let unavailable =
            ProfileStore::factory_only(dir.path().join("broken"), "unreadable catalog".into());
        let summary = unavailable
            .inspect_lab_file(&other)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(summary
            .refusal
            .as_deref()
            .is_some_and(|refusal| refusal.contains("could not be loaded")));
        let refused = unavailable
            .install_lab_file(&other, false)
            .err()
            .unwrap_or_else(|| panic!("an unavailable catalog must refuse the install"));
        assert!(
            refused.to_string().contains("could not be loaded"),
            "{refused}"
        );
        assert!(!dir.path().join("broken").exists());
    }
}
