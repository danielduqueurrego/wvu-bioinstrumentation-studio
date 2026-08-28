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
    "f3eb3fec0166189f66a8ddfeb058172a8d97a3c60d95d574b3234eecbd144aab";

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

        assert!(source.contains("PROTOCOL_MAJOR = 0, PROTOCOL_MINOR = 4"));
        assert!(source.contains("FIRMWARE_BUILD = 0x00010006UL"));
        assert!(source.contains("D4_GREEN = 4, D5_RED = 5, D6_IR = 6"));
        assert!(source.contains("void forceSafeOutputs()"));
        assert!(source.contains(
            "else if(frame[6]==PING){sendHello();sendCapabilities();enqueueFrame(PONG,nullptr,0);}"
        ));
        assert!(source.contains(
            "if(acquisitionMode==PULSEOX_4STATE){applyPulseState(pulseState);nextTickMicros=micros()+stateDwellUs;}"
        ));
        assert!(source.contains("applyPulseState(pulseState);\n  nextTickMicros+=stateDwellUs;"));
        assert!(source.contains("if((int32_t)(now-nextTickMicros)>=0)acquirePulseState();"));
        assert!(!source.contains("void acquirePulseState(){\n  applyPulseState(pulseState);"));
        assert!(!source.contains("DIAGNOSTIC_TRANSPORT_ONLY"));
        assert!(!source.contains("FspTimer.h"));
    }

    #[test]
    fn timing_critical_acquisition_never_writes_to_serial() {
        let source = std::str::from_utf8(controlled_reference_source())
            .unwrap_or_else(|error| panic!("reference sketch is not UTF-8: {error}"));
        let simultaneous = source
            .split("void acquireSimultaneous(){")
            .nth(1)
            .and_then(|value| value.split("void acquirePulseState(){").next())
            .unwrap_or_else(|| panic!("missing simultaneous acquisition function"));
        let pulseox = source
            .split("void acquirePulseState(){")
            .nth(1)
            .and_then(|value| value.split("void setup(){").next())
            .unwrap_or_else(|| panic!("missing pulse-ox acquisition function"));
        assert!(!simultaneous.contains("Serial.write"));
        assert!(!pulseox.contains("Serial.write"));
        assert!(!simultaneous.contains("enqueueFrame"));
        assert!(!pulseox.contains("enqueueFrame"));
        assert!(!simultaneous.contains("crc16"));
        assert!(!pulseox.contains("crc16"));
        assert!(!simultaneous.contains("serviceBatchEncoding"));
        assert!(!pulseox.contains("serviceBatchEncoding"));
        assert!(pulseox.contains("nextTickMicros+=stateDwellUs"));
        assert!(!pulseox.contains("nextTickMicros=micros()+stateDwellUs"));
        assert!(source.contains("Serial.availableForWrite()"));
        assert!(source.contains("frame.offset+=(uint16_t)written"));
        assert!(source.contains("acquisitionQueueOverflows++"));
        assert!(source.contains("SAMPLE_FLAG_EXPLICIT_TIMESTAMPS"));
    }

    #[test]
    fn simulated_backpressure_at_every_tenth_cycle_does_not_shift_sampling_deadlines() {
        const DWELL_US: u64 = 1_000;
        const CYCLES: usize = 60 * 250;
        let mut phase_deadline = DWELL_US;
        let mut simulated_clock = 0u64;
        let mut cycle_timestamps = Vec::with_capacity(CYCLES);
        let mut phase_timestamps: [Vec<u64>; 4] =
            std::array::from_fn(|_| Vec::with_capacity(CYCLES));
        let mut phase = 0usize;
        let mut queued_batches = 0usize;
        while cycle_timestamps.len() < CYCLES {
            simulated_clock = simulated_clock.max(phase_deadline);
            let now = simulated_clock;
            phase_timestamps[phase].push(now);
            // Model ADC and ordinary bookkeeping on every phase. Every tenth
            // completed cycle also makes a raw batch available for later
            // encoding. Neither cost changes the absolute next deadline.
            simulated_clock += 30;
            phase = (phase + 1) % 4;
            if phase == 0 {
                cycle_timestamps.push(now);
                if cycle_timestamps.len().is_multiple_of(10) {
                    simulated_clock += 400;
                    queued_batches += 1;
                }
            }
            phase_deadline += DWELL_US;
        }
        let intervals = cycle_timestamps
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .collect::<Vec<_>>();
        assert_eq!(queued_batches, CYCLES / 10);
        assert!(intervals.iter().all(|interval| *interval == 4_000));
        for timestamps in phase_timestamps {
            assert!(timestamps.windows(2).all(|pair| pair[1] - pair[0] == 4_000));
        }
        for remainder in 0..10 {
            assert!(intervals
                .iter()
                .skip(remainder)
                .step_by(10)
                .all(|interval| *interval == 4_000));
        }
        let measured_rate = (CYCLES - 1) as f64 * 1_000_000.0
            / (cycle_timestamps[CYCLES - 1] - cycle_timestamps[0]) as f64;
        assert!((measured_rate - 250.0).abs() / 250.0 < 0.01);
    }

    #[test]
    fn extended_micros_model_is_monotonic_across_u32_wraparound() {
        let values = [u32::MAX - 500, u32::MAX - 1, 250, 1_250];
        let mut last = 0u32;
        let mut high = 0u64;
        let extended = values.map(|now| {
            if now < last {
                high += 1u64 << 32;
            }
            last = now;
            high | u64::from(now)
        });
        assert!(extended.windows(2).all(|pair| pair[1] > pair[0]));
        assert_eq!(extended[2] - extended[1], 252);
    }
}
