//! Shared ingestion path for serial hardware and the simulator.
use crate::{
    protocol::{Frame, FrameParser, IntegrityMonitor, MessageType, SampleBatch},
    recording::SynchronizedRecord,
};
use std::{
    sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError},
    thread,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum AcquisitionState {
    Idle,
    Configured,
    Running,
    Stopping,
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
    /// v0.3 CAPABILITIES is:
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
    first_timestamp: Option<u64>,
    last_timestamp: Option<u64>,
    last_counts: Option<u16>,
    hello_seen: bool,
    capabilities_seen: bool,
    config_ack_seen: bool,
    pong_seen: bool,
    status_seen: bool,
    pong_count: u64,
    status_count: u64,
    firmware_error_code: Option<u8>,
    firmware_error_count: u64,
    firmware_error_payload: Option<Vec<u8>>,
    firmware_build: Option<u32>,
    firmware_board_id: Option<u32>,
    firmware_capabilities: Option<FirmwareCapabilities>,
    digital_output_mask: Option<u8>,
}
impl AcquisitionController {
    pub fn new(sender: SyncSender<SynchronizedRecord>) -> Self {
        Self {
            state: AcquisitionState::Idle,
            parser: FrameParser::default(),
            monitor: IntegrityMonitor::default(),
            sender,
            sample_count: 0,
            first_timestamp: None,
            last_timestamp: None,
            last_counts: None,
            hello_seen: false,
            capabilities_seen: false,
            config_ack_seen: false,
            pong_seen: false,
            status_seen: false,
            pong_count: 0,
            status_count: 0,
            firmware_error_code: None,
            firmware_error_count: 0,
            firmware_error_payload: None,
            firmware_build: None,
            firmware_board_id: None,
            firmware_capabilities: None,
            digital_output_mask: None,
        }
    }
    pub fn configure(&mut self) -> Result<(), &'static str> {
        if self.state != AcquisitionState::Idle {
            return Err("acquisition must be idle before configuration");
        }
        self.state = AcquisitionState::Configured;
        Ok(())
    }
    pub fn start(&mut self) -> Result<(), &'static str> {
        if self.state != AcquisitionState::Configured {
            return Err("acquisition must be configured before start");
        }
        self.state = AcquisitionState::Running;
        Ok(())
    }
    pub fn stop(&mut self) {
        if self.state == AcquisitionState::Running {
            self.state = AcquisitionState::Stopping;
        }
        self.state = AcquisitionState::Idle;
    }
    pub fn ingest_bytes(&mut self, bytes: &[u8]) {
        for frame in self.parser.push(bytes) {
            self.ingest_frame(frame);
        }
        self.monitor.counters.crc_failures = self.parser.stats.crc_failures;
        self.monitor.counters.invalid_frames = self.parser.stats.invalid_frames;
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
            MessageType::ConfigAck => self.config_ack_seen = true,
            MessageType::Pong => {
                self.pong_seen = true;
                self.pong_count += 1;
            }
            MessageType::Status => {
                self.status_seen = true;
                self.status_count += 1;
                if frame.payload.len() >= 2 {
                    self.digital_output_mask = Some(frame.payload[1] & 0x07);
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
            self.monitor.counters.invalid_frames += 1;
            return;
        };
        let count = batch.samples.len() / usize::from(batch.channel_count);
        self.monitor.observe_samples(
            batch.first_sample_sequence,
            count as u32,
            batch.status_flags,
        );
        if self.state != AcquisitionState::Running {
            return;
        }
        for index in 0..count {
            let first = index * usize::from(batch.channel_count);
            let record = SynchronizedRecord {
                sequence: batch.first_sample_sequence.wrapping_add(index as u32),
                timestamp_us: batch.first_timestamp_us
                    + u64::from(batch.sample_period_us) * index as u64,
                status_flags: batch.status_flags,
                counts: batch.samples[first..first + usize::from(batch.channel_count)].to_vec(),
            };
            match self.sender.try_send(record.clone()) {
                Ok(()) => {
                    self.sample_count += 1;
                    self.first_timestamp.get_or_insert(record.timestamp_us);
                    self.last_timestamp = Some(record.timestamp_us);
                    self.last_counts = record.counts.first().copied();
                }
                Err(TrySendError::Full(_)) => self.monitor.counters.host_channel_overflows += 1,
                Err(TrySendError::Disconnected(_)) => {
                    self.state = AcquisitionState::Disconnected;
                }
            }
        }
    }
    pub fn snapshot(&self) -> AcquisitionSnapshot {
        let measured_rate_hz = match (self.first_timestamp, self.last_timestamp, self.sample_count)
        {
            (Some(first), Some(last), n) if last > first && n > 1 => {
                (n - 1) as f64 * 1_000_000.0 / (last - first) as f64
            }
            _ => 0.0,
        };
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
            firmware_error_code: self.firmware_error_code,
            firmware_error_count: self.firmware_error_count,
            firmware_error_payload: self.firmware_error_payload.clone(),
            firmware_build: self.firmware_build,
            firmware_board_id: self.firmware_board_id,
            firmware_capabilities: self.firmware_capabilities.clone(),
            digital_output_mask: self.digital_output_mask,
            skipped_noise_bytes: self.parser.stats.skipped_noise_bytes,
        }
    }
}
pub fn simulator_stream(seconds: u32, sample_rate_hz: u32) -> Receiver<SynchronizedRecord> {
    // Ten seconds at 1000 samples/s fits below this fixed bound; no unbounded queue is used.
    let (out_tx, out_rx) = sync_channel(16_384);
    thread::spawn(move || {
        let (sample_tx, sample_rx) = sync_channel(16_384);
        let mut controller = AcquisitionController::new(sample_tx);
        if controller.configure().is_err() || controller.start().is_err() {
            return;
        }
        let period = 1_000_000 / sample_rate_hz;
        let total = seconds.saturating_mul(sample_rate_hz);
        let mut packet_sequence = 0u32;
        let mut first = 0u32;
        while first < total {
            let take = (total - first).min(20);
            let samples = (0..take)
                .map(|i| {
                    let phase =
                        (first + i) as f64 * std::f64::consts::TAU * 2.0 / sample_rate_hz as f64;
                    (2048.0 + phase.sin() * 1200.0).clamp(0.0, 4095.0) as u16
                })
                .collect();
            let batch = SampleBatch {
                first_sample_sequence: first,
                first_timestamp_us: u64::from(first) * u64::from(period),
                sample_period_us: period,
                channel_count: 1,
                samples,
                status_flags: 1,
            };
            let payload = match batch.to_payload() {
                Ok(p) => p,
                Err(_) => return,
            };
            let frame = Frame {
                message_type: MessageType::SampleBatch,
                flags: 0,
                sequence: packet_sequence,
                payload,
            };
            let encoded = match crate::protocol::encode_frame(&frame) {
                Ok(v) => v,
                Err(_) => return,
            };
            for fragment in encoded.chunks(7) {
                controller.ingest_bytes(fragment);
            }
            packet_sequence = packet_sequence.wrapping_add(1);
            first += take;
        }
        controller.stop();
        for sample in sample_rx.try_iter() {
            if out_tx.send(sample).is_err() {
                break;
            }
        }
    });
    out_rx
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simulator_uses_protocol_path_and_state_transitions() {
        let (tx, rx) = sync_channel(64);
        let mut c = AcquisitionController::new(tx);
        assert!(c.start().is_err());
        assert!(c.configure().is_ok());
        assert!(c.start().is_ok());
        let b = SampleBatch {
            first_sample_sequence: 0,
            first_timestamp_us: 0,
            sample_period_us: 1000,
            channel_count: 1,
            samples: vec![10, 11],
            status_flags: 1,
        };
        let f = Frame {
            message_type: MessageType::SampleBatch,
            flags: 0,
            sequence: 0,
            payload: b.to_payload().unwrap_or_default(),
        };
        c.ingest_bytes(&crate::protocol::encode_frame(&f).unwrap_or_default());
        assert_eq!(rx.try_iter().count(), 2);
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
