//! Bench-only analog-interface validation evidence.
//!
//! This module deliberately keeps validation evidence separate from acquisition
//! profiles and raw recordings. A SHA-256 hash detects changes to finalized
//! evidence; it is not a signature, authentication system, or human-use approval.
use crate::{
    profiles::{safe_filename_component, AcquisitionProfile, ProfileError, ProfileMode},
    recording::{BmegReader, RawSample},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

pub const VALIDATION_SCHEMA_VERSION: u32 = 1;
pub const METRIC_ALGORITHM_VERSION: &str = "phase3b.raw_metrics.v1";

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationEvidenceStatus {
    Draft,
    Finalized,
    Retired,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationTestType {
    Baseline,
    DcSweep,
    SineWave,
    SaturationMargin,
    Repeatability,
}

impl ValidationTestType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Baseline => "zero_input_baseline",
            Self::DcSweep => "dc_operating_range_sweep",
            Self::SineWave => "known_sine_wave_acquisition",
            Self::SaturationMargin => "saturation_margin",
            Self::Repeatability => "repeatability",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionOperator {
    LessThanOrEqual,
    GreaterThanOrEqual,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidationHardware {
    pub board: String,
    pub board_serial: String,
    pub com_port: String,
    pub firmware_build: String,
    pub firmware_device: String,
    pub module_name: String,
    pub module_identifier: String,
    pub module_revision: String,
    pub module_serial: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EquipmentItem {
    pub name: String,
    pub identifier: String,
    pub calibration_or_notes: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricValue {
    pub name: String,
    pub value: f64,
    pub units: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    pub metric: String,
    pub operator: CriterionOperator,
    pub threshold: f64,
    pub units: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CriterionResult {
    pub criterion: AcceptanceCriterion,
    pub observed_value: Option<f64>,
    pub passed: bool,
    pub explanation: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidationRun {
    pub run_number: u32,
    pub test_type: ValidationTestType,
    pub source_description: String,
    pub source_setpoint_v: Option<f64>,
    pub source_frequency_hz: Option<f64>,
    pub source_peak_to_peak_v: Option<f64>,
    pub bmeg_path: String,
    pub metadata_path: String,
    pub csv_path: String,
    pub raw_sample_count: u64,
    pub algorithm_version: String,
    pub metrics: Vec<MetricValue>,
    pub criteria: Vec<CriterionResult>,
    pub notes: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidationIntegrity {
    pub canonical_hash_algorithm: String,
    pub canonical_hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidationEvidence {
    pub schema_version: u32,
    pub validation_id: String,
    pub profile_id: String,
    pub profile_version: String,
    pub profile_hash: String,
    pub status: ValidationEvidenceStatus,
    pub validation_type: String,
    pub created_at: DateTime<Utc>,
    pub created_by_mode: ProfileMode,
    pub hardware: ValidationHardware,
    pub equipment: Vec<EquipmentItem>,
    pub test_conditions: BTreeMap<String, String>,
    pub tests: Vec<ValidationRun>,
    pub acceptance_summary: Vec<CriterionResult>,
    pub accepted: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub notes: String,
    pub integrity: ValidationIntegrity,
    #[serde(flatten, default)]
    pub additional: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileValidationStatus {
    Unvalidated,
    DraftValidation,
    BenchValidated,
    ValidationExpired,
    ValidationDoesNotMatchProfile,
    ValidationDoesNotMatchFirmware,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidationStatusSummary {
    pub profile_id: String,
    pub profile_version: String,
    pub status: ProfileValidationStatus,
    pub validation_id: Option<String>,
    pub explanation: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidationPackageManifest {
    pub schema_version: u32,
    pub validation_id: String,
    pub profile_id: String,
    pub profile_version: String,
    pub created_at: DateTime<Utc>,
    pub app_version: String,
    pub firmware_build: String,
    pub firmware_device: String,
    pub files: Vec<ManifestFile>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManifestFile {
    pub name: String,
    pub sha256: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("validation evidence error: {0}")]
    Validation(String),
    #[error("validation evidence I/O at {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("validation evidence JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("recording metrics: {0}")]
    Recording(#[from] crate::recording::RecordingError),
    #[error("profile: {0}")]
    Profile(#[from] ProfileError),
}

impl ValidationEvidence {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ValidationError> {
        let mut value = self.clone();
        value.integrity.canonical_hash.clear();
        serde_json::to_vec(&value).map_err(ValidationError::Json)
    }

    pub fn computed_hash(&self) -> Result<String, ValidationError> {
        Ok(format!("{:x}", Sha256::digest(self.canonical_bytes()?)))
    }

    pub fn refresh_hash(&mut self) -> Result<(), ValidationError> {
        self.integrity.canonical_hash_algorithm = "SHA-256".into();
        self.integrity.canonical_hash = self.computed_hash()?;
        Ok(())
    }

    pub fn verify_integrity(&self) -> Result<(), ValidationError> {
        if !self
            .integrity
            .canonical_hash_algorithm
            .eq_ignore_ascii_case("SHA-256")
        {
            return Err(ValidationError::Validation(
                "finalized validation evidence requires SHA-256 integrity".into(),
            ));
        }
        let calculated = self.computed_hash()?;
        if self.integrity.canonical_hash.len() != 64 || self.integrity.canonical_hash != calculated
        {
            return Err(ValidationError::Validation(format!(
                "validation {} has an integrity hash mismatch",
                self.validation_id
            )));
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != VALIDATION_SCHEMA_VERSION {
            return Err(ValidationError::Validation(format!(
                "unsupported validation schema version {}",
                self.schema_version
            )));
        }
        if !valid_identifier(&self.validation_id) || self.profile_id.trim().is_empty() {
            return Err(ValidationError::Validation(
                "validation and profile IDs must be non-empty safe identifiers".into(),
            ));
        }
        if !valid_semver(&self.profile_version) || self.profile_hash.len() != 64 {
            return Err(ValidationError::Validation(
                "profile version or profile hash is invalid".into(),
            ));
        }
        if self.validation_type != "analog_interface" {
            return Err(ValidationError::Validation(
                "Phase 3B supports only analog_interface validation".into(),
            ));
        }
        if self.created_by_mode != ProfileMode::InstructorAuthoring {
            return Err(ValidationError::Validation(
                "validation evidence must be created in instructor authoring mode".into(),
            ));
        }
        if self.hardware.board != "Arduino UNO R4 WiFi"
            || self.hardware.firmware_build.trim().is_empty()
            || self.hardware.firmware_device.trim().is_empty()
        {
            return Err(ValidationError::Validation(
                "UNO R4 WiFi and controlled firmware identity are required".into(),
            ));
        }
        if self.status == ValidationEvidenceStatus::Finalized {
            if self.tests.is_empty() || !self.accepted {
                return Err(ValidationError::Validation(
                    "finalized evidence requires completed passing tests and explicit acceptance"
                        .into(),
                ));
            }
            if self
                .tests
                .iter()
                .any(|test| test.criteria.iter().any(|result| !result.passed))
                || self.acceptance_summary.iter().any(|result| !result.passed)
            {
                return Err(ValidationError::Validation(
                    "finalized evidence contains a failed acceptance criterion".into(),
                ));
            }
            self.verify_integrity()?;
        }
        Ok(())
    }

    pub fn matches_profile(&self, profile: &AcquisitionProfile) -> Result<(), ValidationError> {
        profile.validate()?;
        if self.profile_id != profile.profile_id
            || self.profile_version != profile.profile_version
            || self.profile_hash != profile.integrity.canonical_hash
        {
            return Err(ValidationError::Validation(
                "validation evidence does not match the selected locked profile ID/version/hash"
                    .into(),
            ));
        }
        if !same_identity(
            &self.hardware.firmware_build,
            &profile.required_firmware.build,
        ) || !same_identity(
            &self.hardware.firmware_device,
            &profile.required_firmware.device,
        ) {
            return Err(ValidationError::Validation(
                "validation evidence does not match the selected profile firmware requirement"
                    .into(),
            ));
        }
        Ok(())
    }
}

/// Summary of raw samples. All values are calculated from the complete supplied
/// sequence without filtering, decimation, or deletion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SampleMetrics {
    pub sample_count: u64,
    pub mean_counts: f64,
    pub mean_volts: f64,
    pub standard_deviation_counts: f64,
    pub standard_deviation_volts: f64,
    pub rms_counts: f64,
    pub rms_volts: f64,
    pub minimum_counts: u16,
    pub maximum_counts: u16,
    pub minimum_volts: f64,
    pub maximum_volts: f64,
    pub peak_to_peak_counts: u16,
    pub peak_to_peak_volts: f64,
    pub zero_rail_samples: u64,
    pub full_scale_rail_samples: u64,
    pub rail_margin_samples: u64,
    pub clipping_percentage: f64,
    pub rail_margin_percentage: f64,
    pub measured_sample_rate_hz: Option<f64>,
}

impl SampleMetrics {
    pub fn metric_values(&self) -> Vec<MetricValue> {
        vec![
            metric("mean_counts", self.mean_counts, "ADC counts"),
            metric("mean_volts", self.mean_volts, "V"),
            metric(
                "standard_deviation_counts",
                self.standard_deviation_counts,
                "ADC counts",
            ),
            metric(
                "standard_deviation_volts",
                self.standard_deviation_volts,
                "V",
            ),
            metric("rms_counts", self.rms_counts, "ADC counts"),
            metric("rms_volts", self.rms_volts, "V"),
            metric(
                "minimum_counts",
                f64::from(self.minimum_counts),
                "ADC counts",
            ),
            metric(
                "maximum_counts",
                f64::from(self.maximum_counts),
                "ADC counts",
            ),
            metric("minimum_volts", self.minimum_volts, "V"),
            metric("maximum_volts", self.maximum_volts, "V"),
            metric("peak_to_peak_volts", self.peak_to_peak_volts, "V"),
            metric("clipping_percentage", self.clipping_percentage, "%"),
            metric("rail_margin_percentage", self.rail_margin_percentage, "%"),
        ]
        .into_iter()
        .chain(
            self.measured_sample_rate_hz
                .map(|rate| metric("measured_sample_rate_hz", rate, "Hz")),
        )
        .collect()
    }
}

pub fn calculate_sample_metrics(
    samples: &[RawSample],
    rail_margin_fraction: f64,
) -> Result<SampleMetrics, ValidationError> {
    if samples.is_empty() {
        return Err(ValidationError::Validation(
            "a validation run contains no raw samples".into(),
        ));
    }
    let mut accumulator = MetricsAccumulator::new(rail_margin_fraction)?;
    for sample in samples {
        accumulator.observe(sample);
    }
    accumulator.finish()
}

pub fn calculate_bmeg_metrics(
    path: &Path,
    rail_margin_fraction: f64,
) -> Result<SampleMetrics, ValidationError> {
    let mut reader = BmegReader::open(path)?;
    let mut accumulator = MetricsAccumulator::new(rail_margin_fraction)?;
    while let Some(sample) = reader.next_sample()? {
        accumulator.observe(&sample);
    }
    accumulator.finish()
}

/// Streaming metric accumulator: BMEG validation needs constant memory even for
/// long raw recordings. Frequency estimation intentionally remains a separate,
/// explicit raw-data operation because its crossings need adjacent timestamps.
struct MetricsAccumulator {
    margin_counts: u16,
    count: u64,
    mean: f64,
    m2: f64,
    sum_squares: f64,
    minimum: u16,
    maximum: u16,
    zero_rails: u64,
    full_rails: u64,
    margin_samples: u64,
    first_timestamp_us: Option<u64>,
    last_timestamp_us: Option<u64>,
}

impl MetricsAccumulator {
    fn new(rail_margin_fraction: f64) -> Result<Self, ValidationError> {
        if !(0.0..=0.5).contains(&rail_margin_fraction) {
            return Err(ValidationError::Validation(
                "rail margin fraction must be between 0 and 0.5".into(),
            ));
        }
        Ok(Self {
            margin_counts: (4095.0 * rail_margin_fraction).round() as u16,
            count: 0,
            mean: 0.0,
            m2: 0.0,
            sum_squares: 0.0,
            minimum: u16::MAX,
            maximum: 0,
            zero_rails: 0,
            full_rails: 0,
            margin_samples: 0,
            first_timestamp_us: None,
            last_timestamp_us: None,
        })
    }
    fn observe(&mut self, sample: &RawSample) {
        let value = f64::from(sample.counts);
        self.count += 1;
        let delta = value - self.mean;
        self.mean += delta / self.count as f64;
        self.m2 += delta * (value - self.mean);
        self.sum_squares += value * value;
        self.minimum = self.minimum.min(sample.counts);
        self.maximum = self.maximum.max(sample.counts);
        self.zero_rails += u64::from(sample.counts == 0);
        self.full_rails += u64::from(sample.counts == 4095);
        self.margin_samples += u64::from(
            sample.counts <= self.margin_counts
                || sample.counts >= 4095u16.saturating_sub(self.margin_counts),
        );
        if self.first_timestamp_us.is_none() {
            self.first_timestamp_us = Some(sample.timestamp_us);
        }
        self.last_timestamp_us = Some(sample.timestamp_us);
    }
    fn finish(self) -> Result<SampleMetrics, ValidationError> {
        if self.count == 0 {
            return Err(ValidationError::Validation(
                "a validation run contains no raw samples".into(),
            ));
        }
        let elapsed_us = self
            .last_timestamp_us
            .unwrap_or_default()
            .saturating_sub(self.first_timestamp_us.unwrap_or_default());
        let measured_sample_rate_hz = (self.count > 1 && elapsed_us > 0)
            .then(|| (self.count - 1) as f64 * 1_000_000.0 / elapsed_us as f64);
        let variance = if self.count > 1 {
            self.m2 / (self.count - 1) as f64
        } else {
            0.0
        };
        let standard_deviation_counts = variance.sqrt();
        let rms_counts = (self.sum_squares / self.count as f64).sqrt();
        let volts = |counts: f64| counts * 5.0 / 4095.0;
        Ok(SampleMetrics {
            sample_count: self.count,
            mean_counts: self.mean,
            mean_volts: volts(self.mean),
            standard_deviation_counts,
            standard_deviation_volts: volts(standard_deviation_counts),
            rms_counts,
            rms_volts: volts(rms_counts),
            minimum_counts: self.minimum,
            maximum_counts: self.maximum,
            minimum_volts: volts(f64::from(self.minimum)),
            maximum_volts: volts(f64::from(self.maximum)),
            peak_to_peak_counts: self.maximum.saturating_sub(self.minimum),
            peak_to_peak_volts: volts(f64::from(self.maximum.saturating_sub(self.minimum))),
            zero_rail_samples: self.zero_rails,
            full_scale_rail_samples: self.full_rails,
            rail_margin_samples: self.margin_samples,
            clipping_percentage: (self.zero_rails + self.full_rails) as f64 * 100.0
                / self.count as f64,
            rail_margin_percentage: self.margin_samples as f64 * 100.0 / self.count as f64,
            measured_sample_rate_hz,
        })
    }
}

/// Transparent mean-threshold, rising-crossing frequency estimate. It uses the
/// raw samples solely to report a metric; it neither changes nor filters the BMEG data.
pub fn estimate_frequency_hz(samples: &[RawSample]) -> Option<f64> {
    if samples.len() < 3 {
        return None;
    }
    let mean = samples
        .iter()
        .map(|sample| f64::from(sample.counts))
        .sum::<f64>()
        / samples.len() as f64;
    let mut crossings = Vec::new();
    for pair in samples.windows(2) {
        let a = f64::from(pair[0].counts) - mean;
        let b = f64::from(pair[1].counts) - mean;
        if a <= 0.0 && b > 0.0 {
            let denominator = b - a;
            if denominator != 0.0 {
                let fraction = (-a / denominator).clamp(0.0, 1.0);
                crossings.push(
                    pair[0].timestamp_us as f64
                        + fraction
                            * (pair[1].timestamp_us.saturating_sub(pair[0].timestamp_us)) as f64,
                );
            }
        }
    }
    if crossings.len() < 2 {
        return None;
    }
    let periods: Vec<f64> = crossings
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|period| *period > 0.0)
        .collect();
    if periods.is_empty() {
        return None;
    }
    Some(1_000_000.0 / (periods.iter().sum::<f64>() / periods.len() as f64))
}

/// Streaming BMEG counterpart of `estimate_frequency_hz`. It makes two bounded
/// passes: first the arithmetic mean, then adjacent rising crossings.
pub fn estimate_bmeg_frequency_hz(path: &Path) -> Result<Option<f64>, ValidationError> {
    let mut reader = BmegReader::open(path)?;
    let mut count = 0u64;
    let mut sum = 0.0;
    while let Some(sample) = reader.next_sample()? {
        count += 1;
        sum += f64::from(sample.counts);
    }
    if count < 3 {
        return Ok(None);
    }
    let mean = sum / count as f64;
    let mut reader = BmegReader::open(path)?;
    let mut previous = reader.next_sample()?;
    let mut crossings = 0u64;
    let mut first_crossing = None;
    let mut last_crossing = None;
    while let (Some(first), Some(second)) = (previous, reader.next_sample()?) {
        let a = f64::from(first.counts) - mean;
        let b = f64::from(second.counts) - mean;
        if a <= 0.0 && b > 0.0 {
            let fraction = (-a / (b - a)).clamp(0.0, 1.0);
            let timestamp = first.timestamp_us as f64
                + fraction * second.timestamp_us.saturating_sub(first.timestamp_us) as f64;
            first_crossing.get_or_insert(timestamp);
            last_crossing = Some(timestamp);
            crossings += 1;
        }
        previous = Some(second);
    }
    match (crossings, first_crossing, last_crossing) {
        (count, Some(first), Some(last)) if count >= 2 && last > first => {
            Ok(Some((count - 1) as f64 * 1_000_000.0 / (last - first)))
        }
        _ => Ok(None),
    }
}

pub fn metrics_for_validation_run(
    bmeg_path: &Path,
    test_type: &ValidationTestType,
    source_setpoint_v: Option<f64>,
    source_frequency_hz: Option<f64>,
) -> Result<(SampleMetrics, Vec<MetricValue>), ValidationError> {
    let summary = calculate_bmeg_metrics(bmeg_path, 0.05)?;
    let mut metrics = summary.metric_values();
    if let Some(setpoint) = source_setpoint_v {
        let absolute_error = (summary.mean_volts - setpoint).abs();
        metrics.push(metric("absolute_voltage_error", absolute_error, "V"));
        if setpoint.abs() > 1e-12 {
            metrics.push(metric(
                "percentage_voltage_error",
                absolute_error / setpoint.abs() * 100.0,
                "%",
            ));
        }
    }
    if *test_type == ValidationTestType::SineWave {
        if let Some(frequency) = estimate_bmeg_frequency_hz(bmeg_path)? {
            metrics.push(metric("measured_frequency_hz", frequency, "Hz"));
            if let Some(expected) = source_frequency_hz {
                metrics.push(metric(
                    "absolute_frequency_error_hz",
                    (frequency - expected).abs(),
                    "Hz",
                ));
            }
        }
    }
    Ok((summary, metrics))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepeatabilitySummary {
    pub run_count: usize,
    pub between_run_mean_volts: f64,
    pub between_run_standard_deviation_volts: f64,
    pub coefficient_of_variation_percent: Option<f64>,
}

pub fn calculate_repeatability(
    runs: &[SampleMetrics],
) -> Result<RepeatabilitySummary, ValidationError> {
    if runs.len() < 3 {
        return Err(ValidationError::Validation(
            "repeatability requires at least three runs".into(),
        ));
    }
    let values: Vec<f64> = runs.iter().map(|run| run.mean_volts).collect();
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (values.len() - 1) as f64;
    let stddev = variance.sqrt();
    let coefficient_of_variation_percent =
        (mean.abs() > 1e-12).then(|| stddev / mean.abs() * 100.0);
    Ok(RepeatabilitySummary {
        run_count: runs.len(),
        between_run_mean_volts: mean,
        between_run_standard_deviation_volts: stddev,
        coefficient_of_variation_percent,
    })
}

pub fn evaluate_criteria(
    metrics: &[MetricValue],
    criteria: &[AcceptanceCriterion],
) -> Vec<CriterionResult> {
    criteria
        .iter()
        .map(|criterion| {
            let observed = metrics
                .iter()
                .find(|metric| metric.name == criterion.metric && metric.units == criterion.units)
                .map(|metric| metric.value);
            let passed = match (criterion.operator.clone(), observed) {
                (CriterionOperator::LessThanOrEqual, Some(value)) => value <= criterion.threshold,
                (CriterionOperator::GreaterThanOrEqual, Some(value)) => {
                    value >= criterion.threshold
                }
                (_, None) => false,
            };
            let explanation = match observed {
                Some(value) => format!(
                    "{} {} {} {}; observed {} {}",
                    criterion.metric,
                    operator_label(&criterion.operator),
                    criterion.threshold,
                    criterion.units,
                    value,
                    criterion.units
                ),
                None => format!(
                    "metric {} with units {} was not supplied",
                    criterion.metric, criterion.units
                ),
            };
            CriterionResult {
                criterion: criterion.clone(),
                observed_value: observed,
                passed,
                explanation,
            }
        })
        .collect()
}

#[derive(Clone)]
pub struct ValidationStore {
    root: PathBuf,
    runtime: Arc<Mutex<ValidationRuntime>>,
}

struct ValidationRuntime {
    evidence: BTreeMap<String, ValidationEvidence>,
    retired: BTreeSet<String>,
}

impl Default for ValidationStore {
    fn default() -> Self {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("WVU Bioinstrumentation Studio")
            .join("validation");
        Self::with_root(root).unwrap_or_else(|_| Self {
            root: std::env::temp_dir(),
            runtime: Arc::new(Mutex::new(ValidationRuntime {
                evidence: BTreeMap::new(),
                retired: BTreeSet::new(),
            })),
        })
    }
}

impl ValidationStore {
    pub fn with_root(root: PathBuf) -> Result<Self, ValidationError> {
        let mut evidence = BTreeMap::new();
        for subdirectory in ["draft", "finalized"] {
            let directory = root.join(subdirectory);
            if !directory.exists() {
                continue;
            }
            for entry in fs::read_dir(&directory).map_err(|source| io_error(&directory, source))? {
                let path = entry.map_err(|source| io_error(&directory, source))?.path();
                if path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
                {
                    let value: ValidationEvidence = serde_json::from_slice(
                        &fs::read(&path).map_err(|source| io_error(&path, source))?,
                    )?;
                    value.validate()?;
                    evidence.insert(value.validation_id.clone(), value);
                }
            }
        }
        let retired_path = retired_index_path(&root);
        let retired = if retired_path.exists() {
            serde_json::from_slice(
                &fs::read(&retired_path).map_err(|source| io_error(&retired_path, source))?,
            )?
        } else {
            BTreeSet::new()
        };
        Ok(Self {
            root,
            runtime: Arc::new(Mutex::new(ValidationRuntime { evidence, retired })),
        })
    }

    pub fn list(&self) -> Result<Vec<ValidationEvidence>, ValidationError> {
        let runtime = self.lock()?;
        Ok(runtime
            .evidence
            .values()
            .filter(|evidence| !runtime.retired.contains(&evidence.validation_id))
            .cloned()
            .collect())
    }

    pub fn get(&self, validation_id: &str) -> Result<ValidationEvidence, ValidationError> {
        self.lock()?
            .evidence
            .get(validation_id)
            .cloned()
            .ok_or_else(|| ValidationError::Validation("validation record not found".into()))
    }

    pub fn create_draft(
        &self,
        mode: ProfileMode,
        profile: &AcquisitionProfile,
        validation_id: String,
        hardware: ValidationHardware,
    ) -> Result<ValidationEvidence, ValidationError> {
        require_instructor_mode(mode)?;
        profile.validate()?;
        if !valid_identifier(&validation_id) {
            return Err(ValidationError::Validation(
                "validation ID must use lowercase letters, numbers, dots, dashes, or underscores"
                    .into(),
            ));
        }
        let mut runtime = self.lock()?;
        if runtime.evidence.contains_key(&validation_id) {
            return Err(ValidationError::Validation(
                "validation ID already exists".into(),
            ));
        }
        let evidence = ValidationEvidence {
            schema_version: VALIDATION_SCHEMA_VERSION,
            validation_id: validation_id.clone(),
            profile_id: profile.profile_id.clone(),
            profile_version: profile.profile_version.clone(),
            profile_hash: profile.integrity.canonical_hash.clone(),
            status: ValidationEvidenceStatus::Draft,
            validation_type: "analog_interface".into(),
            created_at: Utc::now(),
            created_by_mode: ProfileMode::InstructorAuthoring,
            hardware,
            equipment: Vec::new(),
            test_conditions: BTreeMap::new(),
            tests: Vec::new(),
            acceptance_summary: Vec::new(),
            accepted: false,
            expires_at: None,
            notes: "Bench-validation use only. No person or electrode system may be connected. Not a medical device.".into(),
            integrity: ValidationIntegrity { canonical_hash_algorithm: "SHA-256".into(), canonical_hash: String::new() },
            additional: BTreeMap::new(),
        };
        runtime.evidence.insert(validation_id, evidence.clone());
        drop(runtime);
        self.persist(&evidence)?;
        Ok(evidence)
    }

    pub fn update_draft_details(
        &self,
        mode: ProfileMode,
        validation_id: &str,
        hardware: ValidationHardware,
        equipment: Vec<EquipmentItem>,
        test_conditions: BTreeMap<String, String>,
        notes: String,
    ) -> Result<ValidationEvidence, ValidationError> {
        require_instructor_mode(mode)?;
        let mut runtime = self.lock()?;
        let evidence = runtime
            .evidence
            .get_mut(validation_id)
            .ok_or_else(|| ValidationError::Validation("validation draft not found".into()))?;
        if evidence.status != ValidationEvidenceStatus::Draft {
            return Err(ValidationError::Validation(
                "finalized validation evidence is immutable; create a new draft revision".into(),
            ));
        }
        evidence.hardware = hardware;
        evidence.equipment = equipment;
        evidence.test_conditions = test_conditions;
        evidence.notes = notes;
        evidence.validate()?;
        let result = evidence.clone();
        drop(runtime);
        self.persist(&result)?;
        Ok(result)
    }

    pub fn add_run(
        &self,
        mode: ProfileMode,
        validation_id: &str,
        run: ValidationRun,
    ) -> Result<ValidationEvidence, ValidationError> {
        require_instructor_mode(mode)?;
        let mut runtime = self.lock()?;
        let evidence = runtime
            .evidence
            .get_mut(validation_id)
            .ok_or_else(|| ValidationError::Validation("validation draft not found".into()))?;
        if evidence.status != ValidationEvidenceStatus::Draft {
            return Err(ValidationError::Validation(
                "only a validation draft may receive a new run".into(),
            ));
        }
        if run.raw_sample_count == 0 || run.algorithm_version != METRIC_ALGORITHM_VERSION {
            return Err(ValidationError::Validation(
                "validation run requires raw samples and a supported metrics algorithm".into(),
            ));
        }
        if evidence
            .tests
            .iter()
            .any(|existing| existing.run_number == run.run_number)
        {
            return Err(ValidationError::Validation(
                "validation run number already exists".into(),
            ));
        }
        evidence.tests.push(run);
        let result = evidence.clone();
        drop(runtime);
        self.persist(&result)?;
        Ok(result)
    }

    pub fn set_acceptance_summary(
        &self,
        mode: ProfileMode,
        validation_id: &str,
        summary: Vec<CriterionResult>,
        accepted: bool,
    ) -> Result<ValidationEvidence, ValidationError> {
        require_instructor_mode(mode)?;
        let mut runtime = self.lock()?;
        let evidence = runtime
            .evidence
            .get_mut(validation_id)
            .ok_or_else(|| ValidationError::Validation("validation draft not found".into()))?;
        if evidence.status != ValidationEvidenceStatus::Draft {
            return Err(ValidationError::Validation(
                "only a draft may set acceptance criteria".into(),
            ));
        }
        evidence.acceptance_summary = summary;
        evidence.accepted = accepted;
        let result = evidence.clone();
        drop(runtime);
        self.persist(&result)?;
        Ok(result)
    }

    pub fn finalize(
        &self,
        mode: ProfileMode,
        validation_id: &str,
        profile: &AcquisitionProfile,
    ) -> Result<ValidationEvidence, ValidationError> {
        require_instructor_mode(mode)?;
        let mut runtime = self.lock()?;
        let evidence = runtime
            .evidence
            .get_mut(validation_id)
            .ok_or_else(|| ValidationError::Validation("validation draft not found".into()))?;
        if evidence.status != ValidationEvidenceStatus::Draft {
            return Err(ValidationError::Validation(
                "only a draft may be finalized".into(),
            ));
        }
        evidence.matches_profile(profile)?;
        let kinds: BTreeSet<_> = evidence
            .tests
            .iter()
            .map(|test| test.test_type.clone())
            .collect();
        let required = [
            ValidationTestType::Baseline,
            ValidationTestType::DcSweep,
            ValidationTestType::SineWave,
            ValidationTestType::SaturationMargin,
            ValidationTestType::Repeatability,
        ];
        if required
            .iter()
            .any(|required_type| !kinds.contains(required_type))
        {
            return Err(ValidationError::Validation("finalization requires baseline, DC, sine, saturation-margin, and repeatability test evidence".into()));
        }
        if evidence
            .tests
            .iter()
            .filter(|test| test.test_type == ValidationTestType::Repeatability)
            .count()
            < 3
        {
            return Err(ValidationError::Validation(
                "finalization requires at least three separate repeatability runs".into(),
            ));
        }
        if evidence.tests.iter().any(|test| test.criteria.is_empty())
            || evidence.acceptance_summary.is_empty()
        {
            return Err(ValidationError::Validation("finalization requires explicit instructor-defined criteria for every run and the acceptance summary".into()));
        }
        evidence.status = ValidationEvidenceStatus::Finalized;
        evidence.refresh_hash()?;
        evidence.validate()?;
        let result = evidence.clone();
        drop(runtime);
        self.persist(&result)?;
        Ok(result)
    }

    pub fn retire(&self, mode: ProfileMode, validation_id: &str) -> Result<(), ValidationError> {
        require_instructor_mode(mode)?;
        let mut runtime = self.lock()?;
        let evidence = runtime
            .evidence
            .get(validation_id)
            .ok_or_else(|| ValidationError::Validation("validation record not found".into()))?;
        if evidence.status != ValidationEvidenceStatus::Finalized {
            return Err(ValidationError::Validation(
                "only finalized validation evidence may be retired".into(),
            ));
        }
        runtime.retired.insert(validation_id.into());
        let retired = runtime.retired.clone();
        drop(runtime);
        self.persist_retired(&retired)?;
        Ok(())
    }

    pub fn profile_status(
        &self,
        profile: &AcquisitionProfile,
    ) -> Result<ValidationStatusSummary, ValidationError> {
        let runtime = self.lock()?;
        let matching: Vec<_> = runtime
            .evidence
            .values()
            .filter(|evidence| {
                evidence.profile_id == profile.profile_id
                    && !runtime.retired.contains(&evidence.validation_id)
            })
            .collect();
        if matching.is_empty() {
            return Ok(status_summary(
                profile,
                ProfileValidationStatus::Unvalidated,
                None,
                "No validation evidence is associated with this profile.",
            ));
        }
        if let Some(evidence) = matching.iter().find(|evidence| {
            evidence.status == ValidationEvidenceStatus::Draft
                && evidence.profile_version == profile.profile_version
                && evidence.profile_hash == profile.integrity.canonical_hash
        }) {
            return Ok(status_summary(
                profile,
                ProfileValidationStatus::DraftValidation,
                Some(evidence.validation_id.clone()),
                "An instructor validation draft exists; it is not bench validation.",
            ));
        }
        if let Some(evidence) = matching.iter().find(|evidence| {
            evidence.status == ValidationEvidenceStatus::Finalized
                && evidence.profile_version == profile.profile_version
                && evidence.profile_hash == profile.integrity.canonical_hash
        }) {
            if is_simulator_evidence(evidence) {
                return Ok(status_summary(
                    profile,
                    ProfileValidationStatus::Unvalidated,
                    Some(evidence.validation_id.clone()),
                    "Finalized simulator evidence is available, but it does not establish physical bench validation.",
                ));
            }
            if evidence.expires_at.is_some_and(|date| date < Utc::now()) {
                return Ok(status_summary(
                    profile,
                    ProfileValidationStatus::ValidationExpired,
                    Some(evidence.validation_id.clone()),
                    "The matching validation evidence has expired.",
                ));
            }
            if !same_identity(
                &evidence.hardware.firmware_build,
                &profile.required_firmware.build,
            ) || !same_identity(
                &evidence.hardware.firmware_device,
                &profile.required_firmware.device,
            ) {
                return Ok(status_summary(
                    profile,
                    ProfileValidationStatus::ValidationDoesNotMatchFirmware,
                    Some(evidence.validation_id.clone()),
                    "Evidence profile matches, but its firmware identity does not.",
                ));
            }
            return Ok(status_summary(
                profile,
                ProfileValidationStatus::BenchValidated,
                Some(evidence.validation_id.clone()),
                "Finalized bench-validation evidence matches this profile and controlled firmware.",
            ));
        }
        Ok(status_summary(
            profile,
            ProfileValidationStatus::ValidationDoesNotMatchProfile,
            None,
            "Existing validation evidence does not match this profile version and hash.",
        ))
    }

    pub fn export_package(
        &self,
        mode: ProfileMode,
        validation_id: &str,
        destination: &Path,
    ) -> Result<PathBuf, ValidationError> {
        require_instructor_mode(mode)?;
        reject_parent_components(destination)?;
        let evidence = self
            .lock()?
            .evidence
            .get(validation_id)
            .cloned()
            .ok_or_else(|| ValidationError::Validation("validation record not found".into()))?;
        if evidence.status != ValidationEvidenceStatus::Finalized {
            return Err(ValidationError::Validation(
                "only finalized evidence may be exported as a validation package".into(),
            ));
        }
        evidence.validate()?;
        fs::create_dir_all(destination).map_err(|source| io_error(destination, source))?;
        let package = destination.join(format!(
            "{}_{}",
            safe_filename_component(&evidence.validation_id),
            evidence.created_at.format("%Y%m%d_%H%M%S")
        ));
        if package.exists() {
            return Err(ValidationError::Validation(
                "validation package destination already exists".into(),
            ));
        }
        fs::create_dir(&package).map_err(|source| io_error(&package, source))?;
        let validation_path = package.join("validation.json");
        let summary_path = package.join("summary.csv");
        write_json(&validation_path, &evidence)?;
        write_summary_csv(&summary_path, &evidence)?;
        let manifest = ValidationPackageManifest {
            schema_version: VALIDATION_SCHEMA_VERSION,
            validation_id: evidence.validation_id.clone(),
            profile_id: evidence.profile_id.clone(),
            profile_version: evidence.profile_version.clone(),
            created_at: Utc::now(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            firmware_build: evidence.hardware.firmware_build.clone(),
            firmware_device: evidence.hardware.firmware_device.clone(),
            files: vec![
                ManifestFile {
                    name: "validation.json".into(),
                    sha256: file_hash(&validation_path)?,
                },
                ManifestFile {
                    name: "summary.csv".into(),
                    sha256: file_hash(&summary_path)?,
                },
            ],
        };
        write_json(&package.join("manifest.json"), &manifest)?;
        Ok(package)
    }

    pub fn import_package(
        &self,
        mode: ProfileMode,
        package: &Path,
        profile: &AcquisitionProfile,
    ) -> Result<ValidationEvidence, ValidationError> {
        require_instructor_mode(mode)?;
        reject_parent_components(package)?;
        let manifest: ValidationPackageManifest = serde_json::from_slice(
            &fs::read(package.join("manifest.json"))
                .map_err(|source| io_error(&package.join("manifest.json"), source))?,
        )?;
        if manifest.schema_version != VALIDATION_SCHEMA_VERSION {
            return Err(ValidationError::Validation(
                "unsupported validation package schema".into(),
            ));
        }
        for expected in ["validation.json", "summary.csv"] {
            let entry = manifest
                .files
                .iter()
                .find(|entry| entry.name == expected)
                .ok_or_else(|| {
                    ValidationError::Validation(format!("manifest is missing {expected}"))
                })?;
            if entry.sha256 != file_hash(&package.join(expected))? {
                return Err(ValidationError::Validation(format!(
                    "manifest hash mismatch for {expected}"
                )));
            }
        }
        let evidence: ValidationEvidence = serde_json::from_slice(
            &fs::read(package.join("validation.json"))
                .map_err(|source| io_error(&package.join("validation.json"), source))?,
        )?;
        if evidence.validation_id != manifest.validation_id
            || evidence.profile_id != manifest.profile_id
            || evidence.profile_version != manifest.profile_version
        {
            return Err(ValidationError::Validation(
                "manifest identity does not match validation evidence".into(),
            ));
        }
        if evidence.status != ValidationEvidenceStatus::Finalized {
            return Err(ValidationError::Validation(
                "only finalized evidence packages may be imported".into(),
            ));
        }
        evidence.validate()?;
        evidence.matches_profile(profile)?;
        let mut runtime = self.lock()?;
        if runtime.evidence.contains_key(&evidence.validation_id) {
            return Err(ValidationError::Validation(
                "validation ID already exists".into(),
            ));
        }
        runtime
            .evidence
            .insert(evidence.validation_id.clone(), evidence.clone());
        drop(runtime);
        self.persist(&evidence)?;
        Ok(evidence)
    }

    fn persist(&self, evidence: &ValidationEvidence) -> Result<(), ValidationError> {
        let state = match evidence.status {
            ValidationEvidenceStatus::Draft => "draft",
            ValidationEvidenceStatus::Finalized | ValidationEvidenceStatus::Retired => "finalized",
        };
        let directory = self.root.join(state);
        fs::create_dir_all(&directory).map_err(|source| io_error(&directory, source))?;
        let path = directory.join(format!(
            "{}.json",
            safe_filename_component(&evidence.validation_id)
        ));
        write_json(&path, evidence)
    }

    fn persist_retired(&self, retired: &BTreeSet<String>) -> Result<(), ValidationError> {
        let path = retired_index_path(&self.root);
        write_json(&path, retired)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ValidationRuntime>, ValidationError> {
        self.runtime
            .lock()
            .map_err(|_| ValidationError::Validation("validation store lock poisoned".into()))
    }
}

fn retired_index_path(root: &Path) -> PathBuf {
    root.join("retired.json")
}

/// Simulator evidence exercises the production acquisition and export path, but
/// it cannot establish that a physical module-to-Arduino interface has been
/// bench validated. Keep that distinction visible in the profile status.
fn is_simulator_evidence(evidence: &ValidationEvidence) -> bool {
    evidence.hardware.com_port.eq_ignore_ascii_case("SIM")
        || evidence
            .hardware
            .board_serial
            .eq_ignore_ascii_case("SIMULATOR")
}

fn require_instructor_mode(mode: ProfileMode) -> Result<(), ValidationError> {
    if mode == ProfileMode::InstructorAuthoring {
        Ok(())
    } else {
        Err(ValidationError::Validation(
            "student mode may not author validation evidence".into(),
        ))
    }
}

fn status_summary(
    profile: &AcquisitionProfile,
    status: ProfileValidationStatus,
    validation_id: Option<String>,
    explanation: &str,
) -> ValidationStatusSummary {
    ValidationStatusSummary {
        profile_id: profile.profile_id.clone(),
        profile_version: profile.profile_version.clone(),
        status,
        validation_id,
        explanation: explanation.into(),
    }
}

fn metric(name: &str, value: f64, units: &str) -> MetricValue {
    MetricValue {
        name: name.into(),
        value,
        units: units.into(),
    }
}
fn operator_label(operator: &CriterionOperator) -> &'static str {
    match operator {
        CriterionOperator::LessThanOrEqual => "≤",
        CriterionOperator::GreaterThanOrEqual => "≥",
    }
}
fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '-' | '_')
        })
}
fn valid_semver(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
        })
}
fn same_identity(actual: &str, expected: &str) -> bool {
    actual
        .trim()
        .trim_start_matches("0x")
        .eq_ignore_ascii_case(expected.trim().trim_start_matches("0x"))
}
fn reject_parent_components(path: &Path) -> Result<(), ValidationError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        Err(ValidationError::Validation(
            "validation paths may not be empty or use parent traversal".into(),
        ))
    } else {
        Ok(())
    }
}
fn io_error(path: &Path, source: std::io::Error) -> ValidationError {
    ValidationError::Io {
        path: path.display().to_string(),
        source,
    }
}
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ValidationError> {
    fs::write(path, serde_json::to_vec_pretty(value)?).map_err(|source| io_error(path, source))
}
fn file_hash(path: &Path) -> Result<String, ValidationError> {
    let mut file = fs::File::open(path).map_err(|source| io_error(path, source))?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut HashWriter(&mut hash))
        .map_err(|source| io_error(path, source))?;
    Ok(format!("{:x}", hash.finalize()))
}
struct HashWriter<'a>(&'a mut Sha256);
impl Write for HashWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.update(buffer);
        Ok(buffer.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn write_summary_csv(path: &Path, evidence: &ValidationEvidence) -> Result<(), ValidationError> {
    let mut writer = fs::File::create(path).map_err(|source| io_error(path, source))?;
    writeln!(writer, "validation_id,profile_id,profile_version,test_type,run_number,metric,value,units,criterion_passed").map_err(|source| io_error(path, source))?;
    for test in &evidence.tests {
        for metric in &test.metrics {
            let passed = test
                .criteria
                .iter()
                .find(|result| {
                    result.criterion.metric == metric.name && result.criterion.units == metric.units
                })
                .map(|result| result.passed.to_string())
                .unwrap_or_default();
            writeln!(
                writer,
                "{},{},{},{},{},{},{:.12},{},{}",
                csv(&evidence.validation_id),
                csv(&evidence.profile_id),
                csv(&evidence.profile_version),
                test.test_type.label(),
                test.run_number,
                csv(&metric.name),
                metric.value,
                csv(&metric.units),
                passed
            )
            .map_err(|source| io_error(path, source))?;
        }
    }
    writer.flush().map_err(|source| io_error(path, source))
}
fn csv(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::built_in_profiles;
    use tempfile::tempdir;

    fn hardware() -> ValidationHardware {
        ValidationHardware {
            board: "Arduino UNO R4 WiFi".into(),
            board_serial: "serial".into(),
            com_port: "SIM".into(),
            firmware_build: "0x00010001".into(),
            firmware_device: "0x554E4F34".into(),
            module_name: "ECG module".into(),
            module_identifier: "bench".into(),
            module_revision: String::new(),
            module_serial: String::new(),
        }
    }
    fn samples(count: u32) -> Vec<RawSample> {
        (0..count)
            .map(|index| RawSample {
                sequence: index,
                timestamp_us: u64::from(index) * 1_000,
                counts: (2048.0 + ((index as f64) * std::f64::consts::TAU / 20.0).sin() * 400.0)
                    as u16,
            })
            .collect()
    }
    #[test]
    fn metrics_frequency_and_criteria_are_transparent() {
        let raw = samples(1_000);
        let summary =
            calculate_sample_metrics(&raw, 0.05).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(summary.sample_count, 1_000);
        assert_eq!(summary.measured_sample_rate_hz, Some(1_000.0));
        assert!((estimate_frequency_hz(&raw).unwrap_or_default() - 50.0).abs() < 0.2);
        let results = evaluate_criteria(
            &summary.metric_values(),
            &[AcceptanceCriterion {
                metric: "clipping_percentage".into(),
                operator: CriterionOperator::LessThanOrEqual,
                threshold: 0.0,
                units: "%".into(),
            }],
        );
        assert!(results[0].passed);
    }
    #[test]
    fn repeatability_requires_three_runs_and_handles_zero_mean() {
        let run = calculate_sample_metrics(
            &[RawSample {
                sequence: 0,
                timestamp_us: 0,
                counts: 0,
            }],
            0.05,
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert!(calculate_repeatability(&[run.clone(), run.clone()]).is_err());
        let result = calculate_repeatability(&[run.clone(), run.clone(), run])
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(result.coefficient_of_variation_percent, None);
    }
    #[test]
    fn finalized_evidence_is_hashed_and_package_tamper_is_rejected() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let store = ValidationStore::with_root(dir.path().join("store"))
            .unwrap_or_else(|error| panic!("{error}"));
        let profile = built_in_profiles().unwrap_or_else(|error| panic!("{error}"))[1].clone();
        let mut evidence = store
            .create_draft(
                ProfileMode::InstructorAuthoring,
                &profile,
                "wvu.bmeg420l.ecg.interface.validation.001".into(),
                hardware(),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        for (index, test_type) in [
            ValidationTestType::Baseline,
            ValidationTestType::DcSweep,
            ValidationTestType::SineWave,
            ValidationTestType::SaturationMargin,
            ValidationTestType::Repeatability,
            ValidationTestType::Repeatability,
            ValidationTestType::Repeatability,
        ]
        .into_iter()
        .enumerate()
        {
            evidence = store
                .add_run(
                    ProfileMode::InstructorAuthoring,
                    &evidence.validation_id,
                    ValidationRun {
                        run_number: index as u32 + 1,
                        test_type,
                        source_description: "deterministic simulator".into(),
                        source_setpoint_v: Some(2.5),
                        source_frequency_hz: Some(50.0),
                        source_peak_to_peak_v: Some(1.0),
                        bmeg_path: "raw.bmeg".into(),
                        metadata_path: "raw.metadata.json".into(),
                        csv_path: "raw.csv".into(),
                        raw_sample_count: 1_000,
                        algorithm_version: METRIC_ALGORITHM_VERSION.into(),
                        metrics: vec![metric("clipping_percentage", 0.0, "%")],
                        criteria: vec![CriterionResult {
                            criterion: AcceptanceCriterion {
                                metric: "clipping_percentage".into(),
                                operator: CriterionOperator::LessThanOrEqual,
                                threshold: 0.0,
                                units: "%".into(),
                            },
                            observed_value: Some(0.0),
                            passed: true,
                            explanation: "pass".into(),
                        }],
                        notes: String::new(),
                    },
                )
                .unwrap_or_else(|error| panic!("{error}"));
        }
        store
            .set_acceptance_summary(
                ProfileMode::InstructorAuthoring,
                &evidence.validation_id,
                vec![CriterionResult {
                    criterion: AcceptanceCriterion {
                        metric: "clipping_percentage".into(),
                        operator: CriterionOperator::LessThanOrEqual,
                        threshold: 0.0,
                        units: "%".into(),
                    },
                    observed_value: Some(0.0),
                    passed: true,
                    explanation: "pass".into(),
                }],
                true,
            )
            .unwrap_or_else(|error| panic!("{error}"));
        let finalized = store
            .finalize(
                ProfileMode::InstructorAuthoring,
                &evidence.validation_id,
                &profile,
            )
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(finalized.verify_integrity().is_ok());
        let package = store
            .export_package(
                ProfileMode::InstructorAuthoring,
                &finalized.validation_id,
                &dir.path().join("packages"),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        fs::write(package.join("summary.csv"), "tampered")
            .unwrap_or_else(|error| panic!("{error}"));
        let importer = ValidationStore::with_root(dir.path().join("other"))
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(importer
            .import_package(ProfileMode::InstructorAuthoring, &package, &profile)
            .is_err());

        store
            .retire(ProfileMode::InstructorAuthoring, &finalized.validation_id)
            .unwrap_or_else(|error| panic!("{error}"));
        let reopened = ValidationStore::with_root(dir.path().join("store"))
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(reopened
            .list()
            .unwrap_or_else(|error| panic!("{error}"))
            .is_empty());
        assert!(reopened.get(&finalized.validation_id).is_ok());
    }
    #[test]
    fn profile_match_and_status_are_explicit() {
        let profile = built_in_profiles().unwrap_or_else(|error| panic!("{error}"))[2].clone();
        let store = ValidationStore::with_root(
            tempdir()
                .unwrap_or_else(|error| panic!("{error}"))
                .path()
                .join("store"),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            store
                .profile_status(&profile)
                .unwrap_or_else(|error| panic!("{error}"))
                .status,
            ProfileValidationStatus::Unvalidated
        );
    }

    #[test]
    fn finalized_simulator_evidence_does_not_claim_physical_bench_validation() {
        let profile = built_in_profiles().unwrap_or_else(|error| panic!("{error}"))[1].clone();
        let store = ValidationStore::with_root(
            tempdir()
                .unwrap_or_else(|error| panic!("{error}"))
                .path()
                .join("store"),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        let mut evidence = store
            .create_draft(
                ProfileMode::InstructorAuthoring,
                &profile,
                "wvu.bmeg420l.ecg.interface.validation.sim.001".into(),
                hardware(),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        evidence.status = ValidationEvidenceStatus::Finalized;
        let validation_id = evidence.validation_id.clone();
        store
            .lock()
            .unwrap_or_else(|error| panic!("{error}"))
            .evidence
            .insert(validation_id, evidence);

        let status = store
            .profile_status(&profile)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(status.status, ProfileValidationStatus::Unvalidated);
        assert!(status.explanation.contains("simulator evidence"));
    }

    #[test]
    fn student_mode_cannot_create_or_modify_validation_evidence() {
        let dir = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let store = ValidationStore::with_root(dir.path().join("store"))
            .unwrap_or_else(|error| panic!("{error}"));
        let profile = built_in_profiles().unwrap_or_else(|error| panic!("{error}"))[1].clone();
        assert!(store
            .create_draft(
                ProfileMode::Student,
                &profile,
                "wvu.student.blocked.001".into(),
                hardware()
            )
            .is_err());
    }
}
