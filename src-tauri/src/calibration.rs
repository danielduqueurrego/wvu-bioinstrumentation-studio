//! Lightweight course calibration support. Calibrations are derived display/export
//! settings: BMEG records always retain the raw ADC counts acquired by the session.
use crate::recording::BmegReader;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
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
    if ss_xx.abs() <= f64::EPSILON {
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

pub fn fit_xgzp_from_recording(request: &XgzpFitRequest) -> Result<LinearFit, CalibrationError> {
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
    fit_linear(&points)
}

#[derive(Clone)]
pub struct CalibrationStore {
    root: PathBuf,
}

impl Default for CalibrationStore {
    fn default() -> Self {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("WVU Bioinstrumentation Studio")
            .join("calibrations");
        Self { root }
    }
}

impl CalibrationStore {
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn list(
        &self,
        profile_id: &str,
        channel_id: &str,
    ) -> Result<Vec<CalibrationPreset>, CalibrationError> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut values = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let preset: CalibrationPreset = serde_json::from_slice(&fs::read(&path)?)?;
            validate_preset(&preset)?;
            if preset.profile_id == profile_id && preset.channel_id == channel_id {
                values.push(preset);
            }
        }
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
}
