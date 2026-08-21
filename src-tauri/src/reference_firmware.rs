//! Immutable WVU reference firmware metadata and source.
//!
//! The distributed application restores only this repository-controlled sketch;
//! it does not provide a general Arduino sketch workspace.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const REFERENCE_SOURCE: &[u8] =
    include_bytes!("../../firmware/reference_unor4wifi/reference_unor4wifi.ino");
/// SHA-256 of the controlled source shipped with this application release.
/// Updating the sketch requires an explicit provenance review and manifest
/// update rather than a tautological "hash is stable" test.
pub const REFERENCE_SOURCE_SHA256: &str =
    "6b1f3d277225e112b732f888023a7c459a7d6bae4ad9df0600f525222f195d5e";

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
    fn reference_source_matches_the_reviewed_sha256() {
        assert!(controlled_reference_source().starts_with(b"/*"));
        assert_eq!(
            source_hash(controlled_reference_source()),
            REFERENCE_SOURCE_SHA256
        );
    }

    #[test]
    fn controlled_reference_preserves_the_protocol_and_safe_output_contract() {
        let source = std::str::from_utf8(controlled_reference_source())
            .unwrap_or_else(|error| panic!("reference sketch is not UTF-8: {error}"));

        assert!(source.contains("PROTOCOL_MAJOR = 0, PROTOCOL_MINOR = 3"));
        assert!(source.contains("FIRMWARE_BUILD = 0x00010003UL"));
        assert!(source.contains("D4_GREEN = 4, D5_RED = 5, D6_IR = 6"));
        assert!(source.contains("void forceSafeOutputs()"));
        assert!(source.contains(
            "else if(frame[6]==PING){sendHello();sendCapabilities();sendFrame(PONG,nullptr,0);}"
        ));
        assert!(!source.contains("DIAGNOSTIC_TRANSPORT_ONLY"));
        assert!(!source.contains("FspTimer.h"));
    }
}
