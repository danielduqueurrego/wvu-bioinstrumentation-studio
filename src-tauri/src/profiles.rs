use serde::Deserialize;
use std::{fs, path::Path};

#[derive(Debug, Deserialize, PartialEq)]
pub struct LabProfile {
    pub schema_version: String,
    pub profile_id: String,
    pub display_name: String,
    pub locked: bool,
    pub max_recording_seconds: u32,
    pub adc_bits: u8,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("cannot read profile {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid profile {path}: {source}")]
    Parse {
        path: String,
        source: serde_json::Error,
    },
    #[error("profile {0} has unsupported ADC resolution")]
    Adc(String),
}

pub fn load_profile(path: &Path) -> Result<LabProfile, ProfileError> {
    let text = fs::read_to_string(path).map_err(|source| ProfileError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let profile: LabProfile =
        serde_json::from_str(&text).map_err(|source| ProfileError::Parse {
            path: path.display().to_string(),
            source,
        })?;
    if !matches!(profile.adc_bits, 10 | 12 | 14) {
        return Err(ProfileError::Adc(profile.profile_id));
    }
    Ok(profile)
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
        "unnamed".to_string()
    } else {
        clean.chars().take(64).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_and_filename() {
        let p = load_profile(Path::new("../profiles/ecg_emg.profile.json"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(p.profile_id, "ecg_emg");
        assert_eq!(safe_filename_component("../Group: 1"), "Group__1");
    }
}
