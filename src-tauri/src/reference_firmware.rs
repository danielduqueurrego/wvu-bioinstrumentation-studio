//! Immutable WVU reference firmware metadata and source.
//!
//! The distributed application restores only this repository-controlled sketch;
//! it does not provide a general Arduino sketch workspace.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const REFERENCE_SOURCE: &[u8] =
    include_bytes!("../../firmware/reference_unor4wifi/reference_unor4wifi.ino");

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirmwareVerificationKind {
    WvuProtocolReference,
    /// Retained only to read historical recording metadata created by older app versions.
    NonWvu,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FirmwareIdentity {
    pub protocol_version: String,
    pub firmware_build: u32,
    pub device_id: u32,
}

pub fn controlled_reference_source() -> &'static [u8] {
    REFERENCE_SOURCE
}

pub fn source_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_source_is_present_and_hash_is_stable() {
        assert!(controlled_reference_source().starts_with(b"/*"));
        assert_eq!(
            source_hash(controlled_reference_source()),
            source_hash(controlled_reference_source())
        );
    }
}
