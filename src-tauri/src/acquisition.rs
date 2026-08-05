//! Shared ingestion path for serial hardware and the simulator.
use crate::{
    protocol::{Frame, FrameParser, IntegrityMonitor, MessageType, SampleBatch},
    recording::RawSample,
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
}
pub struct AcquisitionController {
    pub state: AcquisitionState,
    parser: FrameParser,
    monitor: IntegrityMonitor,
    sender: SyncSender<RawSample>,
    pub sample_count: u64,
    first_timestamp: Option<u64>,
    last_timestamp: Option<u64>,
    last_counts: Option<u16>,
    hello_seen: bool,
    capabilities_seen: bool,
    config_ack_seen: bool,
    pong_seen: bool,
    status_seen: bool,
}
impl AcquisitionController {
    pub fn new(sender: SyncSender<RawSample>) -> Self {
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
            MessageType::Hello => self.hello_seen = true,
            MessageType::Capabilities => self.capabilities_seen = true,
            MessageType::ConfigAck => self.config_ack_seen = true,
            MessageType::Pong => self.pong_seen = true,
            MessageType::Status => self.status_seen = true,
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
        if batch.channel_count != 1 || self.state != AcquisitionState::Running {
            return;
        }
        for (index, counts) in batch.samples.into_iter().enumerate() {
            let sample = RawSample {
                sequence: batch.first_sample_sequence.wrapping_add(index as u32),
                timestamp_us: batch.first_timestamp_us
                    + u64::from(batch.sample_period_us) * index as u64,
                counts,
            };
            match self.sender.try_send(sample) {
                Ok(()) => {
                    self.sample_count += 1;
                    self.first_timestamp.get_or_insert(sample.timestamp_us);
                    self.last_timestamp = Some(sample.timestamp_us);
                    self.last_counts = Some(sample.counts);
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
        }
    }
}
pub fn simulator_stream(seconds: u32, sample_rate_hz: u32) -> Receiver<RawSample> {
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
}
