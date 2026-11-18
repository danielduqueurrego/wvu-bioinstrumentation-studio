//! Lightweight course calibration support. Calibrations are derived display/export
//! settings: BMEG records always retain the raw ADC counts acquired by the session.
use crate::recording::BmegReader;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub const CALIBRATION_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_ADC_REFERENCE_V: f64 = 5.0;
pub const DEFAULT_MPXV_SUPPLY_V: f64 = 5.0;
pub const MMHG_PER_KPA: f64 = 7.5006;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationType {
    FixedFormula,
    Linear,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationPreset {
    pub schema_version: u32,
    pub calibration_id: String,
    pub profile_id: String,
    pub channel_id: String,
    pub calibration_type: CalibrationType,
    pub input_quantity: String,
    pub output_quantity: String,
    pub output_units: String,
    pub parameters: BTreeMap<String, f64>,
    pub created_at: DateTime<Utc>,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordingCalibration {
    pub adc_reference_v: f64,
    pub mpxv_sensor_supply_v: f64,
    #[serde(default)]
    pub channel_units: BTreeMap<String, String>,
    #[serde(default)]
    pub active_calibrations: Vec<CalibrationPreset>,
}

impl Default for RecordingCalibration {
    fn default() -> Self {
        Self {
            adc_reference_v: DEFAULT_ADC_REFERENCE_V,
            mpxv_sensor_supply_v: DEFAULT_MPXV_SUPPLY_V,
            channel_units: BTreeMap::new(),
            active_calibrations: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationPoint {
    pub input_voltage: f64,
    pub reference_value: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinearFit {
    pub slope: f64,
    pub offset: f64,
    pub r_squared: f64,
    pub paired_samples: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct XgzpFitRequest {
    pub bmeg_path: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub adc_reference_v: f64,
    pub mpxv_sensor_supply_v: f64,
}

/// An XGZP fit plus the voltages behind it. The fit always uses the
/// requested Vref and Vs; the values frozen into the recording are reported
/// alongside so the page can show a difference instead of hiding it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XgzpFit {
    #[serde(flatten)]
    pub fit: LinearFit,
    /// The ADC reference and MPXV supply this fit used (the request's).
    pub adc_reference_v: f64,
    pub mpxv_sensor_supply_v: f64,
    /// The values frozen into the recording; `None` for a recording made
    /// before calibration metadata was stored.
    pub recording_adc_reference_v: Option<f64>,
    pub recording_mpxv_sensor_supply_v: Option<f64>,
}

#[derive(Debug, thiserror::Error)]
pub enum CalibrationError {
    #[error("calibration value must be finite and within the supported 0.1–10 V range")]
    InvalidVoltage,
    #[error("calibration values must be finite")]
    NonFinite,
    #[error("a linear calibration requires at least two points")]
    InsufficientPoints,
    #[error("a linear calibration requires varying input voltages")]
    ZeroInputVariance,
    #[error("invalid calibration interval")]
    InvalidInterval,
    #[error("recording does not contain the synchronized BP MPXV and XGZP channels")]
    MissingBpChannels,
    #[error("recording is not a Blood Pressure + PPG course capture")]
    WrongProfile,
    #[error("calibration JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("calibration I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("recording: {0}")]
    Recording(#[from] crate::recording::RecordingError),
    #[error("invalid calibration: {0}")]
    Invalid(String),
}

pub fn validate_voltage(value: f64) -> Result<(), CalibrationError> {
    if value.is_finite() && (0.1..=10.0).contains(&value) {
        Ok(())
    } else {
        Err(CalibrationError::InvalidVoltage)
    }
}

pub fn counts_to_volts(
    counts: u16,
    adc_bits: u8,
    reference_v: f64,
) -> Result<f64, CalibrationError> {
    validate_voltage(reference_v)?;
    if !(1..=16).contains(&adc_bits) {
        return Err(CalibrationError::Invalid(
            "ADC resolution must be 1 through 16 bits".into(),
        ));
    }
    Ok(f64::from(counts) * reference_v / (f64::from((1_u32 << adc_bits) - 1)))
}

pub fn mpxv_kpa(v_out: f64, sensor_supply_v: f64) -> Result<f64, CalibrationError> {
    validate_voltage(sensor_supply_v)?;
    if !v_out.is_finite() {
        return Err(CalibrationError::NonFinite);
    }
    Ok((v_out / sensor_supply_v - 0.04) / 0.009)
}

pub fn mpxv_mmhg(v_out: f64, sensor_supply_v: f64) -> Result<f64, CalibrationError> {
    Ok(mpxv_kpa(v_out, sensor_supply_v)? * MMHG_PER_KPA)
}

pub fn fit_linear(points: &[CalibrationPoint]) -> Result<LinearFit, CalibrationError> {
    if points.len() < 2 {
        return Err(CalibrationError::InsufficientPoints);
    }
    if points
        .iter()
        .any(|point| !point.input_voltage.is_finite() || !point.reference_value.is_finite())
    {
        return Err(CalibrationError::NonFinite);
    }
    let n = points.len() as f64;
    let mean_x = points.iter().map(|point| point.input_voltage).sum::<f64>() / n;
    let mean_y = points
        .iter()
        .map(|point| point.reference_value)
        .sum::<f64>()
        / n;
    let ss_xx = points
        .iter()
        .map(|point| (point.input_voltage - mean_x).powi(2))
        .sum::<f64>();
    // Relative to the inputs' magnitude: a spread lost in rounding is no
    // spread at all, whatever the voltage level.
    let scale = mean_x.abs().max(1.0);
    if ss_xx.is_nan() || ss_xx <= f64::EPSILON * n * scale * scale {
        return Err(CalibrationError::ZeroInputVariance);
    }
    let ss_xy = points
        .iter()
        .map(|point| (point.input_voltage - mean_x) * (point.reference_value - mean_y))
        .sum::<f64>();
    let slope = ss_xy / ss_xx;
    let offset = mean_y - slope * mean_x;
    let ss_total = points
        .iter()
        .map(|point| (point.reference_value - mean_y).powi(2))
        .sum::<f64>();
    let ss_residual = points
        .iter()
        .map(|point| (point.reference_value - (slope * point.input_voltage + offset)).powi(2))
        .sum::<f64>();
    let r_squared = if ss_total <= f64::EPSILON {
        1.0
    } else {
        1.0 - ss_residual / ss_total
    };
    // Finite inputs can still overflow (for example reference values near
    // 1e200). A non-finite result would serialize as null and could not be
    // saved, so it is refused here with a clear error.
    if !slope.is_finite() || !offset.is_finite() || !r_squared.is_finite() {
        return Err(CalibrationError::NonFinite);
    }
    Ok(LinearFit {
        slope,
        offset,
        r_squared,
        paired_samples: points.len() as u64,
    })
}

pub fn apply_linear(volts: f64, calibration: &CalibrationPreset) -> Result<f64, CalibrationError> {
    if calibration.calibration_type != CalibrationType::Linear {
        return Err(CalibrationError::Invalid(
            "expected a linear calibration".into(),
        ));
    }
    let slope = calibration
        .parameters
        .get("slope")
        .copied()
        .ok_or_else(|| CalibrationError::Invalid("linear calibration has no slope".into()))?;
    let offset = calibration
        .parameters
        .get("offset")
        .copied()
        .ok_or_else(|| CalibrationError::Invalid("linear calibration has no offset".into()))?;
    if !volts.is_finite() || !slope.is_finite() || !offset.is_finite() {
        return Err(CalibrationError::NonFinite);
    }
    Ok(slope * volts + offset)
}

impl RecordingCalibration {
    pub fn validate(&self) -> Result<(), CalibrationError> {
        validate_voltage(self.adc_reference_v)?;
        validate_voltage(self.mpxv_sensor_supply_v)?;
        for calibration in &self.active_calibrations {
            validate_preset(calibration)?;
        }
        Ok(())
    }

    pub fn for_channel(&self, channel_id: &str) -> Option<&CalibrationPreset> {
        self.active_calibrations
            .iter()
            .find(|calibration| calibration.channel_id == channel_id)
    }
}

pub fn fixed_mpxv_calibration(
    profile_id: String,
    channel_id: String,
    sensor_supply_v: f64,
    adc_reference_v: f64,
) -> Result<CalibrationPreset, CalibrationError> {
    validate_voltage(sensor_supply_v)?;
    validate_voltage(adc_reference_v)?;
    Ok(CalibrationPreset {
        schema_version: CALIBRATION_SCHEMA_VERSION,
        calibration_id: format!("builtin.mpxv.{}", channel_id),
        profile_id,
        channel_id,
        calibration_type: CalibrationType::FixedFormula,
        input_quantity: "volts".into(),
        output_quantity: "pressure".into(),
        output_units: "kPa/mmHg".into(),
        parameters: BTreeMap::from([
            ("sensor_supply_v".into(), sensor_supply_v),
            ("adc_reference_v".into(), adc_reference_v),
        ]),
        created_at: Utc::now(),
        label: "MPXV transfer equation".into(),
    })
}

pub fn validate_preset(preset: &CalibrationPreset) -> Result<(), CalibrationError> {
    if preset.schema_version != CALIBRATION_SCHEMA_VERSION
        || preset.calibration_id.trim().is_empty()
        || preset.profile_id.trim().is_empty()
        || preset.channel_id.trim().is_empty()
        || preset.output_units.trim().is_empty()
        || preset.label.trim().is_empty()
    {
        return Err(CalibrationError::Invalid(
            "schema, ID, profile, channel, units, and label are required".into(),
        ));
    }
    if preset.parameters.values().any(|value| !value.is_finite()) {
        return Err(CalibrationError::NonFinite);
    }
    match preset.calibration_type {
        CalibrationType::FixedFormula => {
            validate_voltage(*preset.parameters.get("sensor_supply_v").ok_or_else(|| {
                CalibrationError::Invalid("fixed MPXV formula requires sensor_supply_v".into())
            })?)?;
        }
        CalibrationType::Linear => {
            if !preset.parameters.contains_key("slope") || !preset.parameters.contains_key("offset")
            {
                return Err(CalibrationError::Invalid(
                    "linear calibration requires slope and offset".into(),
                ));
            }
        }
    }
    Ok(())
}

pub fn fit_xgzp_from_recording(request: &XgzpFitRequest) -> Result<XgzpFit, CalibrationError> {
    validate_voltage(request.adc_reference_v)?;
    validate_voltage(request.mpxv_sensor_supply_v)?;
    if !request.start_seconds.is_finite()
        || !request.end_seconds.is_finite()
        || request.start_seconds < 0.0
        || request.end_seconds <= request.start_seconds
    {
        return Err(CalibrationError::InvalidInterval);
    }
    let mut reader = BmegReader::open(Path::new(&request.bmeg_path))?;
    let snapshot = reader
        .metadata
        .profile_snapshot
        .as_ref()
        .ok_or(CalibrationError::WrongProfile)?;
    if snapshot.profile.category != "course_blood_pressure" {
        return Err(CalibrationError::WrongProfile);
    }
    let channels = snapshot.profile.acquisition.resolved_channels();
    let mpxv = channels
        .iter()
        .position(|channel| channel.id == "mpxv")
        .ok_or(CalibrationError::MissingBpChannels)?;
    let xgzp = channels
        .iter()
        .position(|channel| channel.id == "xgzp")
        .ok_or(CalibrationError::MissingBpChannels)?;
    let recorded = reader.metadata.calibration.as_ref();
    let recording_adc_reference_v = recorded.map(|calibration| calibration.adc_reference_v);
    let recording_mpxv_sensor_supply_v =
        recorded.map(|calibration| calibration.mpxv_sensor_supply_v);
    let mut origin = None;
    let mut points = Vec::new();
    while let Some(record) = reader.next_record()? {
        let start = *origin.get_or_insert(record.timestamp_us);
        let elapsed = record.timestamp_us.saturating_sub(start) as f64 / 1_000_000.0;
        if elapsed < request.start_seconds || elapsed > request.end_seconds {
            continue;
        }
        let reference = mpxv_mmhg(
            counts_to_volts(
                record.counts[mpxv],
                reader.metadata.adc_bits,
                request.adc_reference_v,
            )?,
            request.mpxv_sensor_supply_v,
        )?;
        let x_voltage = counts_to_volts(
            record.counts[xgzp],
            reader.metadata.adc_bits,
            request.adc_reference_v,
        )?;
        points.push(CalibrationPoint {
            input_voltage: x_voltage,
            reference_value: reference,
        });
    }
    Ok(XgzpFit {
        fit: fit_linear(&points)?,
        adc_reference_v: request.adc_reference_v,
        mpxv_sensor_supply_v: request.mpxv_sensor_supply_v,
        recording_adc_reference_v,
        recording_mpxv_sensor_supply_v,
    })
}

#[derive(Clone)]
pub struct CalibrationStore {
    root: PathBuf,
    /// Unreadable preset files already written to the application log, so
    /// repeated listing does not repeat the entry.
    reported: Arc<Mutex<BTreeSet<PathBuf>>>,
}

impl Default for CalibrationStore {
    fn default() -> Self {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("WVU Bioinstrumentation Studio")
            .join("calibrations");
        Self::with_root(root)
    }
}

/// A preset file that could not be used, and why.
type SkippedPreset = (PathBuf, String);

fn read_preset_file(path: &Path) -> Result<CalibrationPreset, String> {
    let bytes = fs::read(path).map_err(|error| format!("could not be read ({error})"))?;
    let preset: CalibrationPreset = serde_json::from_slice(&bytes)
        .map_err(|error| format!("is not a valid calibration preset ({error})"))?;
    validate_preset(&preset).map_err(|error| format!("failed validation ({error})"))?;
    Ok(preset)
}

impl CalibrationStore {
    pub fn with_root(root: PathBuf) -> Self {
        Self {
            root,
            reported: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }
    /// Every usable preset, plus the `.json` files that were skipped because
    /// they could not be read or validated. One bad file never hides the
    /// others; the file itself is left untouched.
    fn scan(&self) -> Result<(Vec<CalibrationPreset>, Vec<SkippedPreset>), CalibrationError> {
        let mut presets = Vec::new();
        let mut skipped = Vec::new();
        if !self.root.exists() {
            return Ok((presets, skipped));
        }
        for entry in fs::read_dir(&self.root)? {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(error) => {
                    skipped.push((self.root.clone(), format!("could not be listed ({error})")));
                    continue;
                }
            };
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            match read_preset_file(&path) {
                Ok(preset) => presets.push(preset),
                Err(reason) => skipped.push((path, reason)),
            }
        }
        self.log_new_skips(&skipped);
        Ok((presets, skipped))
    }
    fn log_new_skips(&self, skipped: &[SkippedPreset]) {
        let Ok(mut reported) = self.reported.lock() else {
            return;
        };
        for (path, reason) in skipped {
            if reported.insert(path.clone()) {
                crate::app_log::record(
                    "WARN",
                    &format!(
                        "CALIBRATION_PRESET_SKIPPED path={} reason={reason}",
                        path.display()
                    ),
                );
            }
        }
    }
    /// Student-facing descriptions of preset files that were skipped.
    pub fn load_issues(&self) -> Vec<String> {
        match self.scan() {
            Ok((_, skipped)) => skipped
                .into_iter()
                .map(|(path, reason)| {
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    format!("Calibration file {name} {reason} and was skipped.")
                })
                .collect(),
            Err(error) => vec![format!("The calibration folder could not be read: {error}")],
        }
    }
    pub fn list(
        &self,
        profile_id: &str,
        channel_id: &str,
    ) -> Result<Vec<CalibrationPreset>, CalibrationError> {
        let (presets, _) = self.scan()?;
        let mut values: Vec<CalibrationPreset> = presets
            .into_iter()
            .filter(|preset| preset.profile_id == profile_id && preset.channel_id == channel_id)
            .collect();
        values.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then(left.calibration_id.cmp(&right.calibration_id))
        });
        Ok(values)
    }
    pub fn save(&self, preset: CalibrationPreset) -> Result<CalibrationPreset, CalibrationError> {
        validate_preset(&preset)?;
        fs::create_dir_all(&self.root)?;
        let name = safe_component(&preset.calibration_id).ok_or_else(|| {
            CalibrationError::Invalid("calibration ID contains unsafe filename characters".into())
        })?;
        let destination = self.root.join(format!("{name}.json"));
        let temporary = self.root.join(format!("{name}.json.part"));
        if destination.exists() {
            return Err(CalibrationError::Invalid(
                "a calibration with this ID already exists; choose another label/ID".into(),
            ));
        }
        fs::write(&temporary, serde_json::to_vec_pretty(&preset)?)?;
        fs::rename(temporary, destination)?;
        Ok(preset)
    }
    pub fn delete(&self, calibration_id: &str) -> Result<(), CalibrationError> {
        let name = safe_component(calibration_id)
            .ok_or_else(|| CalibrationError::Invalid("invalid calibration ID".into()))?;
        let path = self.root.join(format!("{name}.json"));
        if !path.exists() {
            return Err(CalibrationError::Invalid(
                "calibration was not found".into(),
            ));
        }
        fs::remove_file(path)?;
        Ok(())
    }
}

fn safe_component(value: &str) -> Option<String> {
    if value.is_empty()
        || value.len() > 120
        || value.contains("..")
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
    {
        None
    } else {
        Some(value.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn counts_endpoints_and_custom_reference_are_exact() {
        assert_eq!(counts_to_volts(0, 12, 5.0).unwrap_or(-1.0), 0.0);
        assert_eq!(counts_to_volts(4095, 12, 5.0).unwrap_or(-1.0), 5.0);
        assert_eq!(counts_to_volts(16383, 14, 3.3).unwrap_or(-1.0), 3.3);
        assert!(counts_to_volts(1, 12, 0.0).is_err());
    }

    #[test]
    fn mpxv_formula_retains_negative_offset_and_converts_units() {
        assert!((mpxv_kpa(0.2, 5.0).unwrap_or_default()).abs() < 1e-12);
        assert!(mpxv_kpa(0.1, 5.0).unwrap_or_default() < 0.0);
        assert!((mpxv_mmhg(0.2, 5.0).unwrap_or_default()).abs() < 1e-12);
    }

    #[test]
    fn mpxv_kpa_and_mmhg_match_the_documented_transfer_equation() {
        let kpa = mpxv_kpa(2.5, 5.0).unwrap_or_default();
        assert!((kpa - ((2.5 / 5.0 - 0.04) / 0.009)).abs() < 1e-12);
        assert!((mpxv_mmhg(2.5, 5.0).unwrap_or_default() - kpa * MMHG_PER_KPA).abs() < 1e-12);
        assert!((mpxv_mmhg(2.5, 5.0).unwrap_or_default() - 383.364).abs() < 1e-3);
    }

    #[test]
    fn linear_fit_handles_exact_lines_and_degenerate_input() {
        let fit = fit_linear(&[
            CalibrationPoint {
                input_voltage: 0.8,
                reference_value: 20.0,
            },
            CalibrationPoint {
                input_voltage: 1.2,
                reference_value: 60.0,
            },
            CalibrationPoint {
                input_voltage: 1.6,
                reference_value: 100.0,
            },
        ])
        .unwrap_or_else(|error| panic!("{error}"));
        assert!((fit.slope - 100.0).abs() < 1e-9);
        assert!((fit.offset + 60.0).abs() < 1e-9);
        assert!((fit.r_squared - 1.0).abs() < 1e-12);
        assert!(fit_linear(&[
            CalibrationPoint {
                input_voltage: 1.0,
                reference_value: 1.0
            },
            CalibrationPoint {
                input_voltage: 1.0,
                reference_value: 2.0
            },
        ])
        .is_err());
    }

    #[test]
    fn linear_fit_refuses_overflowing_results_and_spreads_lost_in_rounding() {
        let point = |input_voltage: f64, reference_value: f64| CalibrationPoint {
            input_voltage,
            reference_value,
        };
        assert!(matches!(
            fit_linear(&[point(1.0, 1e200), point(2.0, -1e200), point(3.0, 1e200)]),
            Err(CalibrationError::NonFinite)
        ));
        // Two points a few nanovolts apart at 5 V are not a measurable spread.
        assert!(matches!(
            fit_linear(&[point(5.0, 0.0), point(5.0 + 2e-9, 100.0)]),
            Err(CalibrationError::ZeroInputVariance)
        ));
        // One ADC step (about 0.3 mV at 14 bits and 5 V) still fits.
        assert!(fit_linear(&[point(5.0, 0.0), point(5.0 + 3e-4, 100.0)]).is_ok());
    }

    #[test]
    fn presets_are_stored_only_for_compatible_profile_and_channel() {
        let root = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let store = CalibrationStore::with_root(root.path().into());
        let preset = CalibrationPreset {
            schema_version: 1,
            calibration_id: "team2.xgzp.001".into(),
            profile_id: "profile".into(),
            channel_id: "xgzp".into(),
            calibration_type: CalibrationType::Linear,
            input_quantity: "volts".into(),
            output_quantity: "pressure".into(),
            output_units: "mmHg".into(),
            parameters: BTreeMap::from([("slope".into(), 100.0), ("offset".into(), -5.0)]),
            created_at: Utc::now(),
            label: "Team 2".into(),
        };
        store
            .save(preset.clone())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            store.list("profile", "xgzp").unwrap_or_default(),
            vec![preset]
        );
        assert!(store.list("other", "xgzp").unwrap_or_default().is_empty());
        store
            .delete("team2.xgzp.001")
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(store.list("profile", "xgzp").unwrap_or_default().is_empty());
    }

    #[test]
    fn one_unreadable_preset_file_does_not_hide_the_others() {
        let root = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let store = CalibrationStore::with_root(root.path().into());
        let preset = CalibrationPreset {
            schema_version: 1,
            calibration_id: "team3.xgzp.001".into(),
            profile_id: "profile".into(),
            channel_id: "xgzp".into(),
            calibration_type: CalibrationType::Linear,
            input_quantity: "volts".into(),
            output_quantity: "pressure".into(),
            output_units: "mmHg".into(),
            parameters: BTreeMap::from([("slope".into(), 100.0), ("offset".into(), -5.0)]),
            created_at: Utc::now(),
            label: "Team 3".into(),
        };
        store
            .save(preset.clone())
            .unwrap_or_else(|error| panic!("{error}"));
        // A hand-edited file, an empty file, and a future-schema preset.
        fs::write(root.path().join("broken.json"), b"{ not json")
            .unwrap_or_else(|error| panic!("{error}"));
        fs::write(root.path().join("empty.json"), b"").unwrap_or_else(|error| panic!("{error}"));
        let mut future = preset.clone();
        future.schema_version = 2;
        future.calibration_id = "future.001".into();
        fs::write(
            root.path().join("future.001.json"),
            serde_json::to_vec(&future).unwrap_or_default(),
        )
        .unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(
            store
                .list("profile", "xgzp")
                .unwrap_or_else(|error| panic!("{error}")),
            vec![preset]
        );
        let issues = store.load_issues();
        assert_eq!(issues.len(), 3, "{issues:?}");
        assert!(issues.iter().any(|issue| issue.contains("broken.json")));
        // The unreadable files are left untouched for the instructor.
        assert!(root.path().join("broken.json").is_file());
        assert!(CalibrationStore::with_root(root.path().join("missing"))
            .load_issues()
            .is_empty());
    }
}
