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
    "86221c104b8a7cf30bfbb0ac621cfdfebc8b08635ed0710df66db8fa4f5d681d";

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

    fn reference_text() -> &'static str {
        std::str::from_utf8(controlled_reference_source())
            .unwrap_or_else(|error| panic!("reference sketch is not UTF-8: {error}"))
    }

    /// The sketch text from `start` (exclusive) up to `end` (exclusive).
    fn between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
        source
            .split(start)
            .nth(1)
            .and_then(|value| value.split(end).next())
            .unwrap_or_else(|| panic!("missing {start}"))
    }

    #[test]
    fn controlled_reference_preserves_the_protocol_and_safe_output_contract() {
        let source = reference_text();

        assert!(source.contains("PROTOCOL_MAJOR = 0, PROTOCOL_MINOR = 4"));
        assert!(source.contains("FIRMWARE_BUILD = 0x00010007UL"));
        assert!(source.contains("D4_GREEN = 4, D5_RED = 5, D6_IR = 6"));
        assert!(source.contains("void forceSafeOutputs()"));
        assert!(source.contains(
            "else if(frame[6]==PING){sendHello();sendCapabilities();enqueueFrame(PONG,nullptr,0);}"
        ));
        // Both pulse-ox modes start with the RED state and one full dwell.
        assert!(source.contains(
            "if(pulseMode()){applyPulseState(pulseState);lastLedEdgeMicros=micros();nextTickMicros=lastLedEdgeMicros+stateDwellUs;}"
        ));
        assert!(source.contains(
            "bool pulseMode(){return acquisitionMode==PULSEOX_4STATE||acquisitionMode==PULSEOX_SEQUENCED;}"
        ));
        assert!(source.contains("applyPulseState(pulseState);\n  nextTickMicros+=stateDwellUs;"));
        assert!(source.contains("if((int32_t)(now-nextTickMicros)>=0)acquirePulseState();"));
        assert!(source.contains("if((int32_t)(now-nextTickMicros)>=0)acquirePulseSequenced();"));
        assert!(!source.contains("void acquirePulseState(){\n  applyPulseState(pulseState);"));
        assert!(!source.contains("DIAGNOSTIC_TRANSPORT_ONLY"));
        assert!(!source.contains("FspTimer.h"));
        // RED and IR are never HIGH together: every state change first drives
        // all three outputs LOW, then at most one of RED (state 0) or IR (2).
        assert!(source.contains(
            "void applyPulseState(uint8_t state){\n  forceSafeOutputs();\n  writeControlledOutput(redOutputPin, state==0 ? HIGH : LOW);\n  writeControlledOutput(irOutputPin, state==2 ? HIGH : LOW);\n}"
        ));
        // An accepted CONFIGURE leaves the outputs LOW.
        assert!(source.contains(
            "if(!ok){rejectConfiguration(3);return;} forceSafeOutputs();configured=true;"
        ));
    }

    #[test]
    fn build_seven_adds_the_sequenced_mode_and_the_deferred_firmware_fixes() {
        let source = reference_text();

        assert!(source.contains(
            "enum AcquisitionMode : uint8_t { SIMULTANEOUS=0, PULSEOX_4STATE=1, PULSEOX_SEQUENCED=2 };"
        ));
        // HELLO and CAPABILITIES advertise mode bits 0x07.
        assert!(source.contains("p[11]=0x07;enqueueFrame(HELLO,p,sizeof(p));"));
        assert!(source.contains("uint8_t p[]={12,14,6,0x07,0x07,5,100,0,200,0,250,0,244,1,232,3};"));
        // Mode 2 CONFIGURE: [2, bits, dwell u32, n, pins, red, ir], checked
        // completely before any state changes.
        let sequenced_configure = between(
            source,
            "bool configurePulseSequenced(const uint8_t* p,uint16_t length){",
            "void handleFrame(",
        );
        assert!(sequenced_configure.contains("length!=(uint16_t)(9+count)"));
        assert!(sequenced_configure.contains("dwell<1000||dwell>5000"));
        assert!(sequenced_configure.contains(
            "if(!uniquePins(pins,count)||red<4||red>6||ir<4||ir>6||red==ir)return false;"
        ));
        let first_assignment = sequenced_configure
            .find("acquisitionMode=PULSEOX_SEQUENCED")
            .unwrap_or_else(|| panic!("mode 2 is never selected"));
        assert!(sequenced_configure[..first_assignment]
            .rfind("return false;")
            .is_some());
        assert!(!sequenced_configure[first_assignment..].contains("return false;"));
        assert!(
            source.contains("(p[0]==PULSEOX_SEQUENCED&&configurePulseSequenced(p,payloadLength))")
        );
        // C-2: CONFIGURE during acquisition stops it like STOP.
        assert!(source.contains(
            "if(frame[6]==CONFIGURE){\n    // CONFIGURE during acquisition first stops it, exactly as STOP does.\n    if(acquiring){finishActiveBatch();acquiring=false;forceSafeOutputs();terminalStatusPending=true;}"
        ));
        // Each finished batch keeps its own layout for the encoder.
        assert!(source.contains(
            "slot.count=activeBatchCount;slot.fields=recordFields();slot.nominalPeriodUs=nominalPeriodUs();"
        ));
        assert!(source.contains("writeU32(p+12,slot.nominalPeriodUs);"));
        // C-5: simultaneous mode refuses any D5/D6 HIGH.
        assert!(source.contains("if((p[7+count]&0x06)!=0)return false;"));
        assert!(!source.contains("if((p[7+count]&0x06)==0x06)return false;"));
        // C-6: skipped deadlines have their own flag and counter; 0x0004 marks
        // only a dropped batch.
        assert!(source.contains("SAMPLE_FLAG_DEADLINE_SKIPPED = 0x0010;"));
        assert!(source.contains(
            "(overflowObserved?SAMPLE_FLAG_QUEUE_OVERFLOW:0)|(deadlineSkipObserved?SAMPLE_FLAG_DEADLINE_SKIPPED:0)"
        ));
        assert!(source.contains("acquisitionBatchesQueued++;activeBatchCount=0;overflowObserved=false;deadlineSkipObserved=false;"));
        assert_eq!(source.matches("overflowObserved=true").count(), 1);
        assert!(source
            .contains("{deadlineSkipObserved=true;skippedDeadlines++;nextTickMicros+=interval;}"));
        // STATUS grows to 46 bytes.
        assert!(source.contains("uint8_t p[46]={0};"));
        assert!(source.contains("writeU32(p+38,skippedDeadlines);writeU32(p+42,minimumSettleUs);"));
        // C-14: the encoder never delays a sample due soon.
        assert!(source.contains(
            "if(acquiring&&(int32_t)(nextTickMicros-micros())<(int32_t)ENCODE_WINDOW_US)return;"
        ));
        // C-15: bounded command reads and a hardware watchdog.
        assert!(source.contains("for(uint8_t budget=0;budget<COMMAND_BYTES_PER_PASS;budget++){\n    int value=Serial.read();\n    if(value<0)break;"));
        assert!(!source.contains("while(Serial.available())"));
        assert!(source.contains("#include <WDT.h>"));
        let setup = between(source, "void setup(){", "void loop(){");
        assert!(setup.starts_with("buildCrcTable();"), "{setup}");
        let crc_table = setup.find("buildCrcTable()").unwrap_or(usize::MAX);
        let hello = setup.find("sendHello()").unwrap_or(0);
        assert!(crc_table < hello);
        assert!(setup.contains("forceSafeOutputs();"));
        assert!(setup
            .trim_end()
            .ends_with("WDT.begin(HARDWARE_WATCHDOG_MS);}"));
        let main_loop = source
            .split("void loop(){")
            .nth(1)
            .unwrap_or_else(|| panic!("missing loop"));
        assert!(main_loop.trim_start().starts_with("WDT.refresh();"));
        // C-16: the header describes all three modes generically.
        let header = between(source, "/*", "*/");
        assert!(header.contains("Teaching use only; not a medical device."));
        assert!(header.contains("RED and IR are never HIGH"));
        for mode in [
            "0 simultaneous analog",
            "1 legacy pulse-ox 4-state",
            "2 LED-sequenced pulse-ox",
        ] {
            assert!(header.contains(mode), "{mode}");
        }
        assert!(!header.contains("A0 TX/A1 RX"));
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
        let sequenced = between(source, "void acquirePulseSequenced(){", "void setup(){");
        for forbidden in [
            "Serial.write",
            "enqueueFrame",
            "crc16",
            "serviceBatchEncoding",
        ] {
            assert!(!sequenced.contains(forbidden), "{forbidden}");
        }
        assert!(sequenced.contains("applyPulseState(pulseState);\n  nextTickMicros+=stateDwellUs;"));
        assert!(sequenced.contains(
            "lastLedEdgeMicros=micros();\n  scheduleNextPulseDeadline(lastLedEdgeMicros);"
        ));
        // One record per state, [state, ch1..chN]: sequence % 4 == state.
        assert!(sequenced.contains("record.sequence=recordSequence;record.timestamp=timestamp;record.fields[0]=pulseState;"));
        assert!(sequenced.contains("pulseState=(pulseState+1)&3;"));
        let deadline = between(
            source,
            "void scheduleNextPulseDeadline(uint32_t ledEdge){",
            "void acquireSimultaneous(){",
        );
        for forbidden in [
            "Serial.write",
            "enqueueFrame",
            "crc16",
            "serviceBatchEncoding",
            "pulseState",
        ] {
            assert!(!deadline.contains(forbidden), "{forbidden}");
        }
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

    /// Model of the sketch's `scheduleNextPulseDeadline` on wrapping u32
    /// microseconds. The firmware reads `micros()` in every pass; here time
    /// does not advance while scheduling, which the guard does not rely on.
    fn schedule_next_pulse_deadline(
        next_tick: &mut u32,
        now: u32,
        led_edge: u32,
        dwell: u32,
        skipped: &mut u32,
    ) {
        let minimum_settle = dwell / 2;
        while now.wrapping_sub(*next_tick) as i32 >= 0
            || (next_tick.wrapping_sub(led_edge) as i32) < minimum_settle as i32
        {
            *next_tick = next_tick.wrapping_add(dwell);
            *skipped += 1;
        }
    }

    struct XorShift(u64);
    impl XorShift {
        fn below(&mut self, bound: u32) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % u64::from(bound.max(1))) as u32
        }
    }

    #[test]
    fn settle_guard_keeps_half_a_dwell_of_settle_without_reordering_states() {
        let source = reference_text();
        // The model below is this exact loop.
        assert!(source.contains(
            "uint32_t minimumSettle=stateDwellUs/2;\n  while((int32_t)(micros()-nextTickMicros)>=0||(int32_t)(nextTickMicros-ledEdge)<(int32_t)minimumSettle){\n    nextTickMicros+=stateDwellUs;skippedDeadlines++;deadlineSkipObserved=true;}"
        ));
        for (dwell, cycles, seed) in [
            (250u32, 60_000u32, 0x9e37_79b9_7f4a_7c15u64),
            (1_000, 40_000, 0x2545_f491_4f6c_dd1d),
            (5_000, 12_000, 0x0123_4567_89ab_cdef),
        ] {
            let mut random = XorShift(seed);
            // Start just before the u32 micros() wraparound.
            let start = u32::MAX - 7 * dwell;
            // START: applyPulseState(0), lastLedEdgeMicros=micros(),
            // nextTickMicros=lastLedEdgeMicros+stateDwellUs.
            let mut clock = start;
            let mut led_edge = clock;
            let mut next_tick = led_edge.wrapping_add(dwell);
            let mut grid = 1u64;
            let mut state = 0u32;
            let mut skipped = 0u32;
            let mut expected_skipped = 0u64;
            let mut minimum_settle = u32::MAX;
            // `sequence` is the record sequence of each sample (mode 2).
            for sequence in 0..cycles {
                // The loop polls until the deadline, then is sometimes late:
                // usually a little, sometimes up to three whole dwells.
                if (clock.wrapping_sub(next_tick) as i32) < 0 {
                    clock = next_tick;
                }
                let lateness = match random.below(100) {
                    0..=79 => random.below(40),
                    80..=94 => random.below(dwell),
                    _ => random.below(3 * dwell),
                };
                clock = clock.wrapping_add(lateness);
                // notePulseSettle(micros()), then the sample of `state`.
                let settle = clock.wrapping_sub(led_edge);
                minimum_settle = minimum_settle.min(settle);
                assert!(settle >= dwell / 2, "dwell {dwell}: settle {settle}");
                assert_eq!(state, sequence % 4, "a state was skipped or reordered");
                clock = clock.wrapping_add(20 + random.below(150));
                state = (state + 1) & 3;
                // applyPulseState(next); nextTickMicros+=stateDwellUs;
                // lastLedEdgeMicros=micros(); scheduleNextPulseDeadline(...).
                clock = clock.wrapping_add(random.below(8));
                next_tick = next_tick.wrapping_add(dwell);
                grid += 1;
                led_edge = clock;
                schedule_next_pulse_deadline(&mut next_tick, clock, led_edge, dwell, &mut skipped);
                // Independently: the first deadline on the absolute grid that
                // is at least half a dwell after the LED edge.
                let edge = u64::from(led_edge.wrapping_sub(start));
                let needed = (edge + u64::from(dwell / 2))
                    .div_ceil(u64::from(dwell))
                    .max(grid);
                expected_skipped += needed - grid;
                grid = needed;
                assert_eq!(
                    next_tick,
                    start.wrapping_add((grid * u64::from(dwell)) as u32),
                    "dwell {dwell}: the deadline left the absolute timeline"
                );
            }
            assert_eq!(u64::from(skipped), expected_skipped, "dwell {dwell}");
            assert!(
                skipped > 0,
                "dwell {dwell}: the lateness never exercised the guard"
            );
            assert!(minimum_settle >= dwell / 2);
            assert!(
                minimum_settle < dwell,
                "dwell {dwell}: no late sample was modeled"
            );
        }
    }

    /// Model of the sketch's `buildCrcTable` and table-driven `crc16`.
    fn firmware_crc_table() -> [u16; 256] {
        std::array::from_fn(|value| {
            let mut crc = (value as u16) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 {
                    (crc << 1) ^ 0x1021
                } else {
                    crc << 1
                };
            }
            crc
        })
    }

    fn firmware_table_crc16(table: &[u16; 256], bytes: &[u8]) -> u16 {
        bytes.iter().fold(0xffff, |crc, byte| {
            (crc << 8) ^ table[usize::from((crc >> 8) as u8 ^ byte)]
        })
    }

    #[test]
    fn table_driven_crc_equals_the_hosts_crc16_ccitt_false() {
        let source = reference_text();
        assert!(source.contains(
            "void buildCrcTable(){for(uint16_t value=0;value<256;value++){uint16_t crc=(uint16_t)(value<<8);for(uint8_t bit=0;bit<8;bit++)crc=(crc&0x8000)?(uint16_t)((crc<<1)^0x1021):(uint16_t)(crc<<1);crcTable[value]=crc;}}"
        ));
        assert!(source.contains(
            "uint16_t crc16(const uint8_t* bytes, size_t length) { uint16_t crc=0xFFFF; while (length--) crc=(uint16_t)((crc<<8)^crcTable[(uint8_t)((crc>>8)^*bytes++)]); return crc; }"
        ));
        let table = firmware_crc_table();
        assert_eq!(firmware_table_crc16(&table, b"123456789"), 0x29b1);
        assert_eq!(firmware_table_crc16(&table, &[]), 0xffff);
        let mut random = XorShift(0x5851_f42d_4c95_7f2d);
        // Up to the largest frame the sketch sends (14 + 264 payload bytes).
        for length in 0..=278usize {
            let bytes = (0..length)
                .map(|_| random.below(256) as u8)
                .collect::<Vec<_>>();
            assert_eq!(
                firmware_table_crc16(&table, &bytes),
                crate::protocol::crc16_ccitt_false(&bytes),
                "length {length}"
            );
        }
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
