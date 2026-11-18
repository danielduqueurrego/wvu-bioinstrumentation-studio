//! Shared ingestion path for serial hardware and the simulator.
use crate::{
    protocol::{Frame, FrameParser, IntegrityMonitor, MessageType, SampleBatch},
    recording::{AcquisitionTimingDiagnostics, SynchronizedRecord},
};

const TIMING_RESERVOIR_CAPACITY: usize = 65_536;
const TIMING_INTERVAL_TOLERANCE_PERCENT: f64 = 5.0;

#[derive(Default)]
struct TimingAccumulator {
    first_timestamp_us: Option<u64>,
    previous_timestamp_us: Option<u64>,
    last_timestamp_us: Option<u64>,
    interval_count: u64,
    minimum_interval_us: u64,
    maximum_interval_us: u64,
    intervals_outside_tolerance: u64,
    reservoir: Vec<u64>,
    reservoir_state: u64,
}

impl TimingAccumulator {
    fn accepts(&self, timestamp_us: u64) -> bool {
        self.previous_timestamp_us
            .is_none_or(|previous| timestamp_us > previous)
    }

    fn observe(&mut self, timestamp_us: u64, nominal_period_us: u32) {
        self.first_timestamp_us.get_or_insert(timestamp_us);
        if let Some(previous) = self.previous_timestamp_us {
            if timestamp_us <= previous {
                return;
            }
            let interval = timestamp_us - previous;
            self.interval_count += 1;
            if self.minimum_interval_us == 0 || interval < self.minimum_interval_us {
                self.minimum_interval_us = interval;
            }
            self.maximum_interval_us = self.maximum_interval_us.max(interval);
            let tolerance =
                (f64::from(nominal_period_us) * TIMING_INTERVAL_TOLERANCE_PERCENT / 100.0).max(1.0);
            if (interval as f64 - f64::from(nominal_period_us)).abs() > tolerance {
                self.intervals_outside_tolerance += 1;
            }
            self.observe_reservoir(interval);
        }
        self.previous_timestamp_us = Some(timestamp_us);
        self.last_timestamp_us = Some(timestamp_us);
    }

    fn observe_reservoir(&mut self, interval: u64) {
        if self.reservoir.len() < TIMING_RESERVOIR_CAPACITY {
            self.reservoir.push(interval);
            return;
        }
        // A deterministic fixed-size reservoir bounds memory for indefinite
        // recordings while retaining an unbiased interval sample for p50/p99.
        self.reservoir_state = self
            .reservoir_state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let selected = self.reservoir_state % self.interval_count.max(1);
        if selected < TIMING_RESERVOIR_CAPACITY as u64 {
            self.reservoir[selected as usize] = interval;
        }
    }

    fn measured_rate_hz(&self) -> f64 {
        match (
            self.first_timestamp_us,
            self.last_timestamp_us,
            self.interval_count,
        ) {
            (Some(first), Some(last), count) if last > first && count > 0 => {
                count as f64 * 1_000_000.0 / (last - first) as f64
            }
            _ => 0.0,
        }
    }

    fn diagnostics(
        &self,
        requested_rate_hz: u32,
        acquisition_queue_overflows: u64,
        pulse_state_intervals_outside_tolerance: Option<[u32; 4]>,
        pulse_state_maximum_error_us: Option<[u32; 4]>,
    ) -> AcquisitionTimingDiagnostics {
        let mut intervals = self.reservoir.clone();
        intervals.sort_unstable();
        let quantile = |fraction: f64| -> f64 {
            if intervals.is_empty() {
                return 0.0;
            }
            let index = ((intervals.len() as f64 * fraction).ceil() as usize)
                .saturating_sub(1)
                .min(intervals.len() - 1);
            intervals[index] as f64
        };
        AcquisitionTimingDiagnostics {
            requested_rate_hz,
            measured_rate_hz: self.measured_rate_hz(),
            interval_count: self.interval_count,
            median_interval_us: quantile(0.5),
            minimum_interval_us: self.minimum_interval_us,
            maximum_interval_us: self.maximum_interval_us,
            p99_interval_us: quantile(0.99),
            interval_tolerance_percent: TIMING_INTERVAL_TOLERANCE_PERCENT,
            intervals_outside_tolerance: self.intervals_outside_tolerance,
            acquisition_queue_overflows,
            timestamp_source: "firmware_per_record".into(),
            pulse_state_intervals_outside_tolerance,
            pulse_state_maximum_error_us,
        }
    }
}
use std::sync::mpsc::{SyncSender, TrySendError};

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum AcquisitionState {
    Idle,
    Configured,
    /// START has been (or is about to be) sent. Nothing is recorded until the
    /// firmware's acquiring STATUS arrives, because the firmware emits that
    /// STATUS before any batch of the new run; anything earlier belongs to a
    /// previous run.
    Starting,
    Running,
    Disconnected,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct AcquisitionSnapshot {
    pub state: AcquisitionState,
    pub sample_count: u64,
    pub last_counts: Option<u16>,
    pub measured_rate_hz: f64,
    pub integrity: crate::protocol::IntegrityCounters,
    pub hello_seen: bool,
    pub capabilities_seen: bool,
    pub config_ack_seen: bool,
    pub pong_seen: bool,
    pub status_seen: bool,
    /// Monotonic frame counts let the capture worker record when it most
    /// recently observed a keepalive response or firmware status frame without
    /// putting host timing into the protocol model.
    pub pong_count: u64,
    pub status_count: u64,
    /// CONFIG_ACK frames received. Capture waits for this to increase past a
    /// baseline taken before CONFIGURE, so a stale acknowledgement cannot
    /// satisfy a new command.
    pub config_ack_count: u64,
    /// The `acquiring` flag (payload byte 0) of the most recent STATUS frame.
    /// STOP is confirmed only by a STATUS newer than STOP with this false.
    pub last_status_acquiring: Option<bool>,
    /// The controlled firmware sends this when it deliberately leaves its
    /// acquisition state (for example, after its host-command watchdog).
    /// Capture owns the policy for turning it into a terminal session fault.
    pub firmware_error_code: Option<u8>,
    pub firmware_error_count: u64,
    /// Optional diagnostic bytes attached to a firmware ERROR frame. Normal
    /// protocol errors remain one byte; temporary controlled-firmware
    /// experiments may provide a small backwards-compatible detail payload.
    pub firmware_error_payload: Option<Vec<u8>>,
    pub firmware_build: Option<u32>,
    pub firmware_board_id: Option<u32>,
    pub firmware_capabilities: Option<FirmwareCapabilities>,
    pub digital_output_mask: Option<u8>,
    pub skipped_noise_bytes: u64,
    /// Protocol minor version of the most recent accepted frame.
    pub protocol_minor: Option<u8>,
}

/// Capability information advertised by the controlled firmware.  The host
/// deliberately keeps this separate from a lab definition: a lab may be
/// authored offline, but a capture is configured only when the connected
/// firmware has advertised support for its requested resources.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct FirmwareCapabilities {
    pub supported_adc_resolutions: Vec<u8>,
    pub max_analog_channels: u8,
    pub supported_modes: u8,
    pub supported_digital_output_mask: u8,
    pub supported_rates_hz: Vec<u32>,
}

impl FirmwareCapabilities {
    /// v0.4 CAPABILITIES is:
    /// min ADC bits, max ADC bits, max analog channels, mode bits, output
    /// mask, rate count, then little-endian u16 rates.  Earlier capability
    /// payloads remain readable but cannot prove the current configuration limits.
    fn from_payload(payload: &[u8]) -> Option<Self> {
        if payload.len() < 6 {
            return None;
        }
        let rate_count = usize::from(payload[5]);
        if payload.len() != 6 + rate_count * 2 || payload[0] > payload[1] {
            return None;
        }
        let supported_rates_hz = payload[6..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| u32::from(u16::from_le_bytes([bytes[0], bytes[1]])))
            .collect();
        Some(Self {
            supported_adc_resolutions: if payload[0] == payload[1] {
                vec![payload[0]]
            } else {
                vec![payload[0], payload[1]]
            },
            max_analog_channels: payload[2],
            supported_modes: payload[3],
            supported_digital_output_mask: payload[4] & 0x07,
            supported_rates_hz,
        })
    }

    pub fn supports_adc_resolution(&self, bits: u8) -> bool {
        self.supported_adc_resolutions.contains(&bits)
    }

    pub fn supports_rate(&self, rate_hz: u32) -> bool {
        self.supported_rates_hz.contains(&rate_hz)
    }

    pub fn supports_mode(&self, mode_bit: u8) -> bool {
        self.supported_modes & mode_bit != 0
    }
}

pub struct AcquisitionController {
    pub state: AcquisitionState,
    parser: FrameParser,
    monitor: IntegrityMonitor,
    sender: SyncSender<SynchronizedRecord>,
    pub sample_count: u64,
    timing: TimingAccumulator,
    last_counts: Option<u16>,
    hello_seen: bool,
    capabilities_seen: bool,
    config_ack_seen: bool,
    pong_seen: bool,
    status_seen: bool,
    pong_count: u64,
    status_count: u64,
    config_ack_count: u64,
    last_status_acquiring: Option<bool>,
    firmware_error_code: Option<u8>,
    firmware_error_count: u64,
    firmware_error_payload: Option<Vec<u8>>,
    firmware_build: Option<u32>,
    firmware_board_id: Option<u32>,
    firmware_capabilities: Option<FirmwareCapabilities>,
    digital_output_mask: Option<u8>,
    pulse_state_intervals_outside_tolerance: [u32; 4],
    pulse_state_maximum_error_us: [u32; 4],
    /// True only after a terminal (acquiring=0) extended STATUS of this run
    /// was parsed. The START STATUS carries zeroed per-phase counters, so
    /// without the terminal one the per-phase checks are unknown, not zero.
    final_phase_timing_received: bool,
    /// Synchronized fields per record configured for this capture. A batch
    /// with a different channel count is counted as invalid instead of being
    /// handed to the raw writer, which would reject the record shape.
    expected_field_count: Option<usize>,
    /// Frames the host rejected after a valid CRC (unparseable batch payload,
    /// non-increasing timestamp, wrong field count). Kept separately because
    /// `invalid_frames` is refreshed from the parser after every read.
    rejected_frames: u64,
}
impl AcquisitionController {
    pub fn new(sender: SyncSender<SynchronizedRecord>) -> Self {
        Self {
            state: AcquisitionState::Idle,
            parser: FrameParser::default(),
            monitor: IntegrityMonitor::default(),
            sender,
            sample_count: 0,
            timing: TimingAccumulator::default(),
            last_counts: None,
            hello_seen: false,
            capabilities_seen: false,
            config_ack_seen: false,
            pong_seen: false,
            status_seen: false,
            pong_count: 0,
            status_count: 0,
            config_ack_count: 0,
            last_status_acquiring: None,
            firmware_error_code: None,
            firmware_error_count: 0,
            firmware_error_payload: None,
            firmware_build: None,
            firmware_board_id: None,
            firmware_capabilities: None,
            digital_output_mask: None,
            pulse_state_intervals_outside_tolerance: [0; 4],
            pulse_state_maximum_error_us: [0; 4],
            final_phase_timing_received: false,
            expected_field_count: None,
            rejected_frames: 0,
        }
    }
    pub fn configure(&mut self) -> Result<(), &'static str> {
        if self.state != AcquisitionState::Idle {
            return Err("acquisition must be idle before configuration");
        }
        self.state = AcquisitionState::Configured;
        Ok(())
    }
    /// Records immediately. Used by offline simulations that have no START
    /// acknowledgement; a hardware capture uses `arm_start` instead.
    pub fn start(&mut self) -> Result<(), &'static str> {
        if self.state != AcquisitionState::Configured {
            return Err("acquisition must be configured before start");
        }
        self.state = AcquisitionState::Running;
        Ok(())
    }
    /// Prepares for START without recording yet. The controller enters
    /// `Running` only when a STATUS reporting `acquiring=1` arrives, and then
    /// resets the sample-sequence baseline the firmware also resets at START.
    /// Batches, STATUS counters, and per-phase checks seen earlier are not part
    /// of the new recording.
    pub fn arm_start(&mut self) -> Result<(), &'static str> {
        if self.state != AcquisitionState::Configured {
            return Err("acquisition must be configured before start");
        }
        self.state = AcquisitionState::Starting;
        Ok(())
    }
    pub fn set_expected_field_count(&mut self, field_count: usize) {
        self.expected_field_count = Some(field_count);
    }
    pub fn stop(&mut self) {
        self.state = AcquisitionState::Idle;
    }
    pub fn ingest_bytes(&mut self, bytes: &[u8]) {
        for frame in self.parser.push(bytes) {
            self.ingest_frame(frame);
        }
        self.monitor.counters.crc_failures = self.parser.stats.crc_failures;
        self.monitor.counters.invalid_frames =
            self.parser.stats.invalid_frames + self.rejected_frames;
        self.monitor.counters.unsupported_versions = self.parser.stats.unsupported_versions;
    }
    fn ingest_frame(&mut self, frame: Frame) {
        self.monitor.observe_frame(&frame);
        match frame.message_type {
            MessageType::Hello => {
                self.hello_seen = true;
                if frame.payload.len() >= 4 {
                    self.firmware_build = Some(u32::from_le_bytes([
                        frame.payload[0],
                        frame.payload[1],
                        frame.payload[2],
                        frame.payload[3],
                    ]));
                }
                if frame.payload.len() >= 8 {
                    self.firmware_board_id = Some(u32::from_le_bytes([
                        frame.payload[4],
                        frame.payload[5],
                        frame.payload[6],
                        frame.payload[7],
                    ]));
                }
            }
            MessageType::Capabilities => {
                self.capabilities_seen = true;
                self.firmware_capabilities = FirmwareCapabilities::from_payload(&frame.payload);
            }
            MessageType::ConfigAck => {
                self.config_ack_seen = true;
                self.config_ack_count += 1;
            }
            MessageType::Pong => {
                self.pong_seen = true;
                self.pong_count += 1;
            }
            MessageType::Status => {
                self.status_seen = true;
                self.status_count += 1;
                let acquiring = frame.payload.first().map(|flag| *flag != 0);
                self.last_status_acquiring = acquiring;
                // The output mask always reflects the board's latest report,
                // including a stale one, because it describes physical pins.
                if frame.payload.len() >= 2 {
                    self.digital_output_mask = Some(frame.payload[1] & 0x07);
                }
                if self.state == AcquisitionState::Starting && acquiring == Some(true) {
                    // The firmware resets its record sequence and overflow
                    // counter at START and sends this STATUS before any batch
                    // of the new run.
                    self.monitor.reset_sample_baseline();
                    self.final_phase_timing_received = false;
                    self.state = AcquisitionState::Running;
                }
                // Counters from a STATUS of a previous run are not this
                // recording's integrity or timing evidence.
                if self.state == AcquisitionState::Running {
                    if frame.payload.len() >= 6 {
                        self.monitor
                            .observe_firmware_queue_overflows(u32::from_le_bytes([
                                frame.payload[2],
                                frame.payload[3],
                                frame.payload[4],
                                frame.payload[5],
                            ]));
                    }
                    if frame.payload.len() >= 38 {
                        for state in 0..4 {
                            let violations = 6 + state * 4;
                            let maximum_error = 22 + state * 4;
                            self.pulse_state_intervals_outside_tolerance[state] =
                                u32::from_le_bytes([
                                    frame.payload[violations],
                                    frame.payload[violations + 1],
                                    frame.payload[violations + 2],
                                    frame.payload[violations + 3],
                                ]);
                            self.pulse_state_maximum_error_us[state] = u32::from_le_bytes([
                                frame.payload[maximum_error],
                                frame.payload[maximum_error + 1],
                                frame.payload[maximum_error + 2],
                                frame.payload[maximum_error + 3],
                            ]);
                        }
                        self.final_phase_timing_received = acquiring == Some(false);
                    }
                }
            }
            MessageType::ErrorMessage => {
                self.firmware_error_count += 1;
                self.firmware_error_code = frame.payload.first().copied();
                self.firmware_error_payload = Some(frame.payload.clone());
            }
            _ => {}
        }
        if frame.message_type != MessageType::SampleBatch {
            return;
        }
        let Ok(batch) = SampleBatch::from_payload(&frame.payload) else {
            self.rejected_frames += 1;
            return;
        };
        if self.state != AcquisitionState::Running {
            // A batch before the START acknowledgement belongs to a previous
            // run (for example one that ended without STOP). It is neither
            // recorded nor allowed to seed this recording's sequence checks.
            return;
        }
        if self
            .expected_field_count
            .is_some_and(|expected| expected != usize::from(batch.channel_count))
        {
            self.rejected_frames += 1;
            return;
        }
        let count = batch.samples.len() / usize::from(batch.channel_count);
        self.monitor.observe_samples(
            batch.first_sample_sequence,
            count as u32,
            batch.status_flags,
            batch.acquisition_queue_overflows,
        );
        for index in 0..count {
            let first = index * usize::from(batch.channel_count);
            let record = SynchronizedRecord {
                sequence: batch.first_sample_sequence.wrapping_add(index as u32),
                timestamp_us: batch.timestamps_us[index],
                status_flags: batch.status_flags,
                counts: batch.samples[first..first + usize::from(batch.channel_count)].to_vec(),
            };
            if !self.timing.accepts(record.timestamp_us) {
                self.rejected_frames += 1;
                continue;
            }
            let timestamp_us = record.timestamp_us;
            let first_counts = record.counts.first().copied();
            match self.sender.try_send(record) {
                Ok(()) => {
                    self.sample_count += 1;
                    self.timing.observe(timestamp_us, batch.sample_period_us);
                    self.last_counts = first_counts;
                }
                Err(TrySendError::Full(_)) => self.monitor.counters.host_channel_overflows += 1,
                Err(TrySendError::Disconnected(_)) => {
                    self.state = AcquisitionState::Disconnected;
                }
            }
        }
    }
    pub fn snapshot(&self) -> AcquisitionSnapshot {
        let measured_rate_hz = self.timing.measured_rate_hz();
        AcquisitionSnapshot {
            state: self.state,
            sample_count: self.sample_count,
            last_counts: self.last_counts,
            measured_rate_hz,
            integrity: self.monitor.counters.clone(),
            hello_seen: self.hello_seen,
            capabilities_seen: self.capabilities_seen,
            config_ack_seen: self.config_ack_seen,
            pong_seen: self.pong_seen,
            status_seen: self.status_seen,
            pong_count: self.pong_count,
            status_count: self.status_count,
            config_ack_count: self.config_ack_count,
            last_status_acquiring: self.last_status_acquiring,
            firmware_error_code: self.firmware_error_code,
            firmware_error_count: self.firmware_error_count,
            firmware_error_payload: self.firmware_error_payload.clone(),
            firmware_build: self.firmware_build,
            firmware_board_id: self.firmware_board_id,
            firmware_capabilities: self.firmware_capabilities.clone(),
            digital_output_mask: self.digital_output_mask,
            skipped_noise_bytes: self.parser.stats.skipped_noise_bytes,
            protocol_minor: self.parser.stats.last_protocol_minor,
        }
    }

    /// Per-phase pulse-ox checks are reported only when the terminal STATUS
    /// of this run was received. Otherwise (fault, disconnect, simulator) they
    /// are `None`, meaning unavailable, rather than a false claim of zero.
    pub fn timing_diagnostics(
        &self,
        requested_rate_hz: u32,
        include_pulse_state_timing: bool,
    ) -> AcquisitionTimingDiagnostics {
        let per_phase = include_pulse_state_timing && self.final_phase_timing_received;
        self.timing.diagnostics(
            requested_rate_hz,
            self.monitor.counters.firmware_overflows,
            per_phase.then_some(self.pulse_state_intervals_outside_tolerance),
            per_phase.then_some(self.pulse_state_maximum_error_us),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::sync_channel;
    #[test]
    fn simulator_uses_protocol_path_and_state_transitions() {
        let (tx, rx) = sync_channel(64);
        let mut c = AcquisitionController::new(tx);
        assert!(c.start().is_err());
        assert!(c.configure().is_ok());
        assert!(c.start().is_ok());
        let b = SampleBatch {
            first_sample_sequence: 0,
            sample_period_us: 1000,
            channel_count: 1,
            timestamps_us: vec![1_000, 2_015],
            samples: vec![10, 11],
            status_flags: crate::protocol::SAMPLE_STATUS_ACQUIRING,
            acquisition_queue_overflows: Some(0),
        };
        let f = Frame {
            message_type: MessageType::SampleBatch,
            flags: 0,
            sequence: 0,
            payload: b.to_payload().unwrap_or_default(),
        };
        c.ingest_bytes(&crate::protocol::encode_frame(&f).unwrap_or_default());
        let records = rx.try_iter().collect::<Vec<_>>();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].timestamp_us, 1_000);
        assert_eq!(records[1].timestamp_us, 2_015);
        c.stop();
        assert_eq!(c.state, AcquisitionState::Idle);
    }

    #[test]
    fn identity_frames_can_arrive_after_a_late_host_ping() {
        let (tx, _rx) = sync_channel(4);
        let mut c = AcquisitionController::new(tx);
        for message_type in [
            MessageType::Hello,
            MessageType::Capabilities,
            MessageType::Pong,
        ] {
            let frame = Frame {
                message_type,
                flags: 0,
                sequence: 1,
                payload: vec![],
            };
            c.ingest_bytes(&crate::protocol::encode_frame(&frame).unwrap_or_default());
        }
        let snapshot = c.snapshot();
        assert!(snapshot.hello_seen && snapshot.capabilities_seen && snapshot.pong_seen);
    }

    #[test]
    fn timing_diagnostics_use_firmware_timestamps_and_report_interval_distribution() {
        let (tx, rx) = sync_channel(16);
        let mut controller = AcquisitionController::new(tx);
        controller
            .configure()
            .unwrap_or_else(|error| panic!("{error}"));
        controller.start().unwrap_or_else(|error| panic!("{error}"));
        let batch = SampleBatch {
            first_sample_sequence: 0,
            sample_period_us: 4_000,
            channel_count: 1,
            timestamps_us: vec![4_000, 8_000, 12_000, 16_600, 20_600],
            samples: vec![1, 2, 3, 4, 5],
            status_flags: crate::protocol::SAMPLE_STATUS_ACQUIRING,
            acquisition_queue_overflows: Some(0),
        };
        let frame = Frame {
            message_type: MessageType::SampleBatch,
            flags: 0,
            sequence: 0,
            payload: batch
                .to_payload()
                .unwrap_or_else(|error| panic!("{error:?}")),
        };
        controller.ingest_bytes(
            &crate::protocol::encode_frame(&frame).unwrap_or_else(|error| panic!("{error:?}")),
        );
        assert_eq!(
            rx.try_iter()
                .map(|record| record.timestamp_us)
                .collect::<Vec<_>>(),
            [4_000, 8_000, 12_000, 16_600, 20_600]
        );
        let timing = controller.timing_diagnostics(250, false);
        assert_eq!(timing.interval_count, 4);
        assert_eq!(timing.minimum_interval_us, 4_000);
        assert_eq!(timing.maximum_interval_us, 4_600);
        assert_eq!(timing.median_interval_us, 4_000.0);
        assert_eq!(timing.p99_interval_us, 4_600.0);
        assert_eq!(timing.intervals_outside_tolerance, 1);
    }

    #[test]
    fn sixty_second_pulseox_timestamps_meet_the_nominal_rate_without_batch_spikes() {
        let mut timing = TimingAccumulator::default();
        for cycle in 0..60 * 250u64 {
            timing.observe((cycle + 1) * 4_000, 4_000);
        }
        let diagnostics = timing.diagnostics(250, 0, None, None);
        assert!((diagnostics.measured_rate_hz - 250.0).abs() / 250.0 < 0.01);
        assert_eq!(diagnostics.median_interval_us, 4_000.0);
        assert_eq!(diagnostics.maximum_interval_us, 4_000);
        assert_eq!(diagnostics.intervals_outside_tolerance, 0);
    }

    fn encoded(message_type: MessageType, sequence: u32, payload: Vec<u8>) -> Vec<u8> {
        crate::protocol::encode_frame(&Frame {
            message_type,
            flags: 0,
            sequence,
            payload,
        })
        .unwrap_or_else(|error| panic!("{error:?}"))
    }

    fn extended_status(
        acquiring: u8,
        mask: u8,
        overflows: u32,
        violations: [u32; 4],
        maximum_errors: [u32; 4],
    ) -> Vec<u8> {
        let mut payload = vec![0; 38];
        payload[0] = acquiring;
        payload[1] = mask;
        payload[2..6].copy_from_slice(&overflows.to_le_bytes());
        for (state, value) in violations.into_iter().enumerate() {
            let offset = 6 + state * 4;
            payload[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (state, value) in maximum_errors.into_iter().enumerate() {
            let offset = 22 + state * 4;
            payload[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        payload
    }

    fn batch_payload(first: u32, timestamps: Vec<u64>, channels: u8) -> Vec<u8> {
        let samples = (0..timestamps.len() * usize::from(channels))
            .map(|value| value as u16)
            .collect();
        SampleBatch {
            first_sample_sequence: first,
            sample_period_us: 1_000,
            channel_count: channels,
            timestamps_us: timestamps,
            samples,
            status_flags: crate::protocol::SAMPLE_STATUS_ACQUIRING,
            acquisition_queue_overflows: Some(0),
        }
        .to_payload()
        .unwrap_or_else(|error| panic!("{error:?}"))
    }

    #[test]
    fn extended_status_preserves_per_phase_timing_diagnostics() {
        let (tx, _rx) = sync_channel(4);
        let mut controller = AcquisitionController::new(tx);
        controller
            .configure()
            .unwrap_or_else(|error| panic!("{error}"));
        controller
            .arm_start()
            .unwrap_or_else(|error| panic!("{error}"));
        // The START STATUS carries zeroed per-phase counters; on its own it
        // must not be reported as "zero violations".
        controller.ingest_bytes(&encoded(
            MessageType::Status,
            0,
            extended_status(1, 0x02, 0, [0; 4], [0; 4]),
        ));
        assert_eq!(controller.state, AcquisitionState::Running);
        let before_stop = controller.timing_diagnostics(250, true);
        assert_eq!(before_stop.pulse_state_intervals_outside_tolerance, None);
        assert_eq!(before_stop.pulse_state_maximum_error_us, None);

        controller.ingest_bytes(&encoded(
            MessageType::Status,
            1,
            extended_status(0, 0, 3, [0, 1, 0, 2], [12, 61, 8, 74]),
        ));
        let timing = controller.timing_diagnostics(250, true);
        assert_eq!(timing.acquisition_queue_overflows, 3);
        assert_eq!(
            timing.pulse_state_intervals_outside_tolerance,
            Some([0, 1, 0, 2])
        );
        assert_eq!(timing.pulse_state_maximum_error_us, Some([12, 61, 8, 74]));
        assert_eq!(controller.snapshot().last_status_acquiring, Some(false));
        let analog_timing = controller.timing_diagnostics(1_000, false);
        assert_eq!(analog_timing.pulse_state_intervals_outside_tolerance, None);
        assert_eq!(analog_timing.pulse_state_maximum_error_us, None);
    }

    #[test]
    fn frames_before_the_start_status_belong_to_a_previous_run() {
        let (tx, rx) = sync_channel(64);
        let mut controller = AcquisitionController::new(tx);
        controller.set_expected_field_count(1);
        controller
            .configure()
            .unwrap_or_else(|error| panic!("{error}"));
        // A terminal STATUS and a batch left over from a run that ended without STOP.
        controller.ingest_bytes(&encoded(
            MessageType::Status,
            40,
            extended_status(0, 0x01, 9, [5; 4], [500; 4]),
        ));
        controller.ingest_bytes(&encoded(
            MessageType::SampleBatch,
            41,
            batch_payload(900, vec![900_000, 901_000], 1),
        ));
        controller
            .arm_start()
            .unwrap_or_else(|error| panic!("{error}"));
        controller.ingest_bytes(&encoded(
            MessageType::SampleBatch,
            42,
            batch_payload(902, vec![902_000, 903_000], 1),
        ));
        assert_eq!(controller.state, AcquisitionState::Starting);
        assert_eq!(rx.try_iter().count(), 0);

        controller.ingest_bytes(&encoded(
            MessageType::Status,
            43,
            extended_status(1, 0, 0, [0; 4], [0; 4]),
        ));
        assert_eq!(controller.state, AcquisitionState::Running);
        controller.ingest_bytes(&encoded(
            MessageType::SampleBatch,
            44,
            batch_payload(0, vec![1_000, 2_000, 3_000], 1),
        ));
        let records = rx.try_iter().collect::<Vec<_>>();
        assert_eq!(
            records
                .iter()
                .map(|record| record.sequence)
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.sample_count, 3);
        assert_eq!(snapshot.integrity.missing_sample_sequences, 0);
        assert_eq!(snapshot.integrity.out_of_order_sample_sequences, 0);
        assert_eq!(snapshot.integrity.duplicate_sample_sequences, 0);
        assert_eq!(snapshot.integrity.firmware_overflows, 0);
        assert_eq!(snapshot.integrity.invalid_frames, 0);
        // The stale terminal STATUS cannot stand in for this run's checks.
        assert_eq!(
            controller
                .timing_diagnostics(250, true)
                .pulse_state_intervals_outside_tolerance,
            None
        );
    }

    #[test]
    fn a_batch_with_the_wrong_field_count_is_invalid_instead_of_recorded() {
        let (tx, rx) = sync_channel(64);
        let mut controller = AcquisitionController::new(tx);
        controller.set_expected_field_count(2);
        controller
            .configure()
            .unwrap_or_else(|error| panic!("{error}"));
        controller.start().unwrap_or_else(|error| panic!("{error}"));
        controller.ingest_bytes(&encoded(
            MessageType::SampleBatch,
            0,
            batch_payload(0, vec![1_000, 2_000], 3),
        ));
        assert_eq!(rx.try_iter().count(), 0);
        assert_eq!(controller.snapshot().integrity.invalid_frames, 1);
        controller.ingest_bytes(&encoded(
            MessageType::SampleBatch,
            1,
            batch_payload(0, vec![1_000, 2_000], 2),
        ));
        assert_eq!(rx.try_iter().count(), 2);
    }

    #[test]
    fn config_ack_and_status_frames_are_counted_for_command_baselines() {
        let (tx, _rx) = sync_channel(4);
        let mut controller = AcquisitionController::new(tx);
        controller.ingest_bytes(&encoded(MessageType::ConfigAck, 0, vec![]));
        controller.ingest_bytes(&encoded(MessageType::Status, 1, vec![1, 0]));
        controller.ingest_bytes(&encoded(MessageType::Status, 2, vec![0, 0]));
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.config_ack_count, 1);
        assert_eq!(snapshot.status_count, 2);
        assert_eq!(snapshot.last_status_acquiring, Some(false));
        // Not armed: an acquiring STATUS alone never starts recording.
        assert_eq!(snapshot.state, AcquisitionState::Idle);
    }

    #[test]
    fn hello_identity_uses_the_documented_payload_offsets() {
        let (tx, _rx) = sync_channel(4);
        let mut controller = AcquisitionController::new(tx);
        let hello = Frame {
            message_type: MessageType::Hello,
            flags: 0,
            sequence: 0,
            payload: vec![0x01, 0x00, 0x01, 0x00, 0x34, 0x4f, 0x4e, 0x55, 1, 12, 1, 0],
        };
        controller.ingest_bytes(&crate::protocol::encode_frame(&hello).unwrap_or_default());
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.firmware_build, Some(0x0001_0001));
        assert_eq!(snapshot.firmware_board_id, Some(0x554e_4f34));
    }

    #[test]
    fn capabilities_are_parsed_for_configuration_checks() {
        let (tx, _rx) = sync_channel(4);
        let mut controller = AcquisitionController::new(tx);
        let capabilities = Frame {
            message_type: MessageType::Capabilities,
            flags: 0,
            sequence: 0,
            payload: vec![
                12, 14, 6, 0x03, 0x07, 5, 100, 0, 200, 0, 250, 0, 244, 1, 232, 3,
            ],
        };
        controller.ingest_bytes(&crate::protocol::encode_frame(&capabilities).unwrap_or_default());
        let parsed = controller
            .snapshot()
            .firmware_capabilities
            .expect("capabilities");
        assert_eq!(parsed.supported_adc_resolutions, vec![12, 14]);
        assert_eq!(parsed.max_analog_channels, 6);
        assert!(parsed.supports_mode(0x01));
        assert!(parsed.supports_mode(0x02));
        assert!(parsed.supports_rate(1_000));
        assert_eq!(parsed.supported_digital_output_mask, 0x07);
    }

    #[test]
    fn legacy_capabilities_are_seen_but_do_not_claim_current_limits() {
        let (tx, _rx) = sync_channel(4);
        let mut controller = AcquisitionController::new(tx);
        let legacy = Frame {
            message_type: MessageType::Capabilities,
            flags: 0,
            sequence: 0,
            payload: vec![12, 1, 6, 0],
        };
        controller.ingest_bytes(&crate::protocol::encode_frame(&legacy).unwrap_or_default());
        let snapshot = controller.snapshot();
        assert!(snapshot.capabilities_seen);
        assert!(snapshot.firmware_capabilities.is_none());
    }

    #[test]
    fn firmware_error_frame_is_retained_for_capture_fault_handling() {
        let (tx, _rx) = sync_channel(4);
        let mut controller = AcquisitionController::new(tx);
        let error = Frame {
            message_type: MessageType::ErrorMessage,
            flags: 0,
            sequence: 7,
            payload: vec![7],
        };
        controller.ingest_bytes(&crate::protocol::encode_frame(&error).unwrap_or_default());
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.firmware_error_code, Some(7));
        assert_eq!(snapshot.firmware_error_count, 1);
        assert_eq!(snapshot.firmware_error_payload, Some(vec![7]));
    }

    #[test]
    fn protocol_v2_status_preserves_controlled_output_mask() {
        let (tx, _rx) = sync_channel(4);
        let mut controller = AcquisitionController::new(tx);
        let active = Frame {
            message_type: MessageType::Status,
            flags: 0,
            sequence: 0,
            payload: vec![1, 0b001],
        };
        controller.ingest_bytes(&crate::protocol::encode_frame(&active).unwrap_or_default());
        assert_eq!(controller.snapshot().digital_output_mask, Some(0b001));
        let stopped = Frame {
            message_type: MessageType::Status,
            flags: 0,
            sequence: 1,
            payload: vec![0, 0],
        };
        controller.ingest_bytes(&crate::protocol::encode_frame(&stopped).unwrap_or_default());
        assert_eq!(controller.snapshot().digital_output_mask, Some(0));
    }
}
