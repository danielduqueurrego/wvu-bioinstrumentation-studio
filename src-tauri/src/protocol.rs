//! Binary USB protocol. Constants here are the host-side source of truth.

use std::collections::VecDeque;

pub const MAGIC: [u8; 4] = *b"BMEG";
pub const PROTOCOL_MAJOR: u8 = 0;
/// Protocol v0.3 adds instructor-configurable controlled output masks and remappable
/// pulse-ox TX/RX and RED/IR resources while keeping the fixed four-state order.
pub const PROTOCOL_MINOR: u8 = 3;
pub const LEGACY_PROTOCOL_MINOR: u8 = 1;
/// USB CDC configuration used by the controlled v0.3 firmware. This leaves
/// headroom for a six-channel 1 kHz logical-frame stream.
pub const CONTROLLED_SERIAL_BAUD: u32 = 921_600;
pub const HEADER_LEN: usize = 14;
pub const CRC_LEN: usize = 2;
pub const MAX_PAYLOAD_LEN: usize = 1024;

/// Immutable identity expected from the controlled UNO R4 WiFi reference firmware.
/// The matching values are encoded in `firmware/reference_unor4wifi`'s HELLO frame.
pub const REFERENCE_FIRMWARE_BUILD: u32 = 0x0001_0003;
pub const REFERENCE_DEVICE_ID: u32 = 0x554e_4f34;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageType {
    Hello = 0x01,
    Capabilities = 0x02,
    Configure = 0x03,
    ConfigAck = 0x04,
    Start = 0x05,
    Stop = 0x06,
    Status = 0x07,
    SampleBatch = 0x08,
    EventMarker = 0x09,
    ErrorMessage = 0x0a,
    Ping = 0x0b,
    Pong = 0x0c,
}

impl TryFrom<u8> for MessageType {
    type Error = ProtocolError;
    fn try_from(value: u8) -> Result<Self, ProtocolError> {
        use MessageType::*;
        match value {
            1 => Ok(Hello),
            2 => Ok(Capabilities),
            3 => Ok(Configure),
            4 => Ok(ConfigAck),
            5 => Ok(Start),
            6 => Ok(Stop),
            7 => Ok(Status),
            8 => Ok(SampleBatch),
            9 => Ok(EventMarker),
            10 => Ok(ErrorMessage),
            11 => Ok(Ping),
            12 => Ok(Pong),
            _ => Err(ProtocolError::UnknownMessage(value)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub message_type: MessageType,
    pub flags: u8,
    pub sequence: u32,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    UnsupportedVersion(u8, u8),
    InvalidLength(usize),
    CrcMismatch,
    UnknownMessage(u8),
    InvalidPayload(&'static str),
}

pub fn crc16_ccitt_false(bytes: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

pub fn encode_frame(frame: &Frame) -> Result<Vec<u8>, ProtocolError> {
    if frame.payload.len() > MAX_PAYLOAD_LEN {
        return Err(ProtocolError::InvalidLength(frame.payload.len()));
    }
    let mut result = Vec::with_capacity(HEADER_LEN + frame.payload.len() + CRC_LEN);
    result.extend_from_slice(&MAGIC);
    result.extend_from_slice(&[
        PROTOCOL_MAJOR,
        PROTOCOL_MINOR,
        frame.message_type as u8,
        frame.flags,
    ]);
    result.extend_from_slice(&(frame.payload.len() as u16).to_le_bytes());
    result.extend_from_slice(&frame.sequence.to_le_bytes());
    result.extend_from_slice(&frame.payload);
    result.extend_from_slice(&crc16_ccitt_false(&result).to_le_bytes());
    Ok(result)
}

#[derive(Default, Debug)]
pub struct ParserStats {
    pub crc_failures: u64,
    pub invalid_frames: u64,
    pub unsupported_versions: u64,
    pub skipped_noise_bytes: u64,
}
#[derive(Default)]
pub struct FrameParser {
    buffer: VecDeque<u8>,
    pub stats: ParserStats,
}
impl FrameParser {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Frame> {
        self.buffer.extend(bytes);
        let mut frames = Vec::new();
        loop {
            while self.buffer.len() >= 4 && !self.starts_magic() {
                self.buffer.pop_front();
                self.stats.skipped_noise_bytes += 1;
            }
            if self.buffer.len() < HEADER_LEN {
                break;
            }
            let length = u16::from_le_bytes([self.buffer[8], self.buffer[9]]) as usize;
            if length > MAX_PAYLOAD_LEN {
                self.buffer.pop_front();
                self.stats.invalid_frames += 1;
                continue;
            }
            let total = HEADER_LEN + length + CRC_LEN;
            if self.buffer.len() < total {
                break;
            }
            let raw: Vec<u8> = self.buffer.iter().take(total).copied().collect();
            let expected = u16::from_le_bytes([raw[total - 2], raw[total - 1]]);
            if crc16_ccitt_false(&raw[..total - CRC_LEN]) != expected {
                self.buffer.pop_front();
                self.stats.crc_failures += 1;
                continue;
            }
            if raw[4] != PROTOCOL_MAJOR
                || !(LEGACY_PROTOCOL_MINOR..=PROTOCOL_MINOR).contains(&raw[5])
            {
                self.drain(total);
                self.stats.invalid_frames += 1;
                self.stats.unsupported_versions += 1;
                continue;
            }
            let message_type = match MessageType::try_from(raw[6]) {
                Ok(value) => value,
                Err(_) => {
                    self.drain(total);
                    self.stats.invalid_frames += 1;
                    continue;
                }
            };
            let sequence_bytes: [u8; 4] = match raw[10..14].try_into() {
                Ok(value) => value,
                Err(_) => {
                    self.drain(total);
                    self.stats.invalid_frames += 1;
                    continue;
                }
            };
            let frame = Frame {
                message_type,
                flags: raw[7],
                sequence: u32::from_le_bytes(sequence_bytes),
                payload: raw[14..14 + length].to_vec(),
            };
            self.drain(total);
            frames.push(frame);
        }
        frames
    }
    fn starts_magic(&self) -> bool {
        self.buffer.iter().take(4).copied().eq(MAGIC)
    }
    fn drain(&mut self, amount: usize) {
        self.buffer.drain(..amount);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SampleBatch {
    pub first_sample_sequence: u32,
    pub first_timestamp_us: u64,
    pub sample_period_us: u32,
    pub channel_count: u8,
    pub samples: Vec<u16>,
    pub status_flags: u16,
}
impl SampleBatch {
    pub fn from_payload(payload: &[u8]) -> Result<Self, ProtocolError> {
        if payload.len() < 22 {
            return Err(ProtocolError::InvalidPayload("sample batch header"));
        }
        let channels = payload[16];
        let count = payload[17];
        if channels == 0 || count == 0 {
            return Err(ProtocolError::InvalidPayload("zero channels or samples"));
        }
        let expected = 20usize + usize::from(channels) * usize::from(count) * 2;
        if payload.len() != expected {
            return Err(ProtocolError::InvalidPayload("sample batch length"));
        }
        let samples = payload[20..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        Ok(Self {
            first_sample_sequence: u32::from_le_bytes(
                payload[0..4]
                    .try_into()
                    .map_err(|_| ProtocolError::InvalidPayload("sequence"))?,
            ),
            first_timestamp_us: u64::from_le_bytes(
                payload[4..12]
                    .try_into()
                    .map_err(|_| ProtocolError::InvalidPayload("timestamp"))?,
            ),
            sample_period_us: u32::from_le_bytes(
                payload[12..16]
                    .try_into()
                    .map_err(|_| ProtocolError::InvalidPayload("period"))?,
            ),
            channel_count: channels,
            samples,
            status_flags: u16::from_le_bytes([payload[18], payload[19]]),
        })
    }
    pub fn to_payload(&self) -> Result<Vec<u8>, ProtocolError> {
        if self.channel_count == 0
            || self.samples.is_empty()
            || !self
                .samples
                .len()
                .is_multiple_of(usize::from(self.channel_count))
            || self.samples.len() / usize::from(self.channel_count) > u8::MAX as usize
        {
            return Err(ProtocolError::InvalidPayload("sample batch shape"));
        }
        let mut output = Vec::with_capacity(20 + self.samples.len() * 2);
        output.extend_from_slice(&self.first_sample_sequence.to_le_bytes());
        output.extend_from_slice(&self.first_timestamp_us.to_le_bytes());
        output.extend_from_slice(&self.sample_period_us.to_le_bytes());
        output.push(self.channel_count);
        output.push((self.samples.len() / usize::from(self.channel_count)) as u8);
        output.extend_from_slice(&self.status_flags.to_le_bytes());
        for sample in &self.samples {
            output.extend_from_slice(&sample.to_le_bytes());
        }
        Ok(output)
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct IntegrityCounters {
    pub received_packets: u64,
    pub crc_failures: u64,
    pub invalid_frames: u64,
    pub unsupported_versions: u64,
    pub missing_packet_sequences: u64,
    pub duplicate_packets: u64,
    pub out_of_order_packets: u64,
    pub missing_sample_sequences: u64,
    pub duplicate_sample_sequences: u64,
    pub out_of_order_sample_sequences: u64,
    pub firmware_overflows: u64,
    pub host_channel_overflows: u64,
    pub reconnects: u64,
    pub disconnect_events: u64,
}
#[derive(Default)]
pub struct IntegrityMonitor {
    packet_next: Option<u32>,
    sample_next: Option<u32>,
    pub counters: IntegrityCounters,
}
impl IntegrityMonitor {
    pub fn observe_frame(&mut self, frame: &Frame) {
        self.counters.received_packets += 1;
        self.observe_sequence(frame.sequence, false);
    }
    pub fn observe_samples(&mut self, first: u32, count: u32, status_flags: u16) {
        if status_flags & 0b100 != 0 {
            self.counters.firmware_overflows += 1;
        }
        if let Some(next) = self.sample_next {
            observe_sequence_delta(
                first,
                next,
                &mut self.counters.missing_sample_sequences,
                &mut self.counters.duplicate_sample_sequences,
                &mut self.counters.out_of_order_sample_sequences,
            );
        }
        self.sample_next = Some(first.wrapping_add(count));
    }
    fn observe_sequence(&mut self, sequence: u32, sample: bool) {
        let next = &mut self.packet_next;
        if let Some(expected) = *next {
            observe_sequence_delta(
                sequence,
                expected,
                &mut self.counters.missing_packet_sequences,
                &mut self.counters.duplicate_packets,
                &mut self.counters.out_of_order_packets,
            );
        }
        *next = Some(sequence.wrapping_add(1));
        let _ = sample;
    }
}

/// Classify a u32 sequence relative to the next expected value using serial-number arithmetic.
/// Differences below 2^31 are forward, which keeps normal gaps correct across wraparound.
fn observe_sequence_delta(
    received: u32,
    expected: u32,
    missing: &mut u64,
    duplicate: &mut u64,
    out_of_order: &mut u64,
) {
    if received == expected {
        return;
    }
    let forward = received.wrapping_sub(expected);
    if forward < (1 << 31) {
        *missing += u64::from(forward);
    } else if received == expected.wrapping_sub(1) {
        *duplicate += 1;
    } else {
        *out_of_order += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(sequence: u32) -> Frame {
        Frame {
            message_type: MessageType::Ping,
            flags: 0,
            sequence,
            payload: vec![1, 2],
        }
    }
    #[test]
    fn crc_known_vector() {
        assert_eq!(crc16_ccitt_false(b"123456789"), 0x29b1);
    }
    #[test]
    fn partial_and_back_to_back() {
        let a = encode_frame(&frame(1)).unwrap_or_default();
        let b = encode_frame(&frame(2)).unwrap_or_default();
        let mut p = FrameParser::default();
        assert!(p.push(&a[..5]).is_empty());
        let mut rest = a[5..].to_vec();
        rest.extend(b);
        assert_eq!(p.push(&rest).len(), 2);
    }

    #[test]
    fn multichannel_batches_preserve_record_major_synchronized_fields() {
        let batch = SampleBatch {
            first_sample_sequence: u32::MAX - 1,
            first_timestamp_us: 123,
            sample_period_us: 1_000,
            channel_count: 6,
            samples: vec![1, 2, 3, 4, 5, 6, 11, 12, 13, 14, 15, 16],
            status_flags: 1,
        };
        let decoded = SampleBatch::from_payload(
            &batch
                .to_payload()
                .unwrap_or_else(|error| panic!("{error:?}")),
        )
        .unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(decoded.channel_count, 6);
        assert_eq!(decoded.samples[0..6], [1, 2, 3, 4, 5, 6]);
        assert_eq!(decoded.samples[6..12], [11, 12, 13, 14, 15, 16]);
    }

    #[test]
    fn twenty_record_eight_channel_batch_stays_within_the_controlled_payload_limit() {
        let batch = SampleBatch {
            first_sample_sequence: 10,
            first_timestamp_us: 100,
            sample_period_us: 1_000,
            channel_count: 8,
            samples: vec![123; 20 * 8],
            status_flags: 1,
        };
        let payload = batch
            .to_payload()
            .unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(payload.len(), 20 + 20 * 8 * 2);
        assert!(payload.len() <= 1_024);
        let decoded =
            SampleBatch::from_payload(&payload).unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(decoded.samples.len(), 20 * 8);
    }

    #[test]
    fn multichannel_batch_rejects_field_count_mismatch() {
        let mut payload = vec![0; 20];
        payload[16] = 4;
        payload[17] = 1;
        payload.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        assert!(SampleBatch::from_payload(&payload).is_err());
    }
    #[test]
    fn complete_back_to_back_identity_frames_preserve_each_crc() {
        let mut bytes = Vec::new();
        for (sequence, message_type, payload) in [
            (1, MessageType::Hello, vec![0; 12]),
            (2, MessageType::Capabilities, vec![12, 1, 6, 0]),
            (3, MessageType::Pong, vec![]),
        ] {
            bytes.extend(
                encode_frame(&Frame {
                    message_type,
                    flags: 0,
                    sequence,
                    payload,
                })
                .unwrap_or_default(),
            );
        }
        let mut parser = FrameParser::default();
        let frames = parser.push(&bytes);
        assert_eq!(frames.len(), 3);
        assert_eq!(parser.stats.crc_failures, 0);
    }
    #[test]
    fn back_to_back_identity_frames_preserve_the_hello_payload() {
        let hello_payload = vec![0x00, 0x00, 0x01, 0x00, 0x34, 0x4f, 0x4e, 0x55, 1, 12, 1, 0];
        let frames = [
            Frame {
                message_type: MessageType::Hello,
                flags: 0,
                sequence: 8,
                payload: hello_payload.clone(),
            },
            Frame {
                message_type: MessageType::Capabilities,
                flags: 0,
                sequence: 9,
                payload: vec![12, 1, 6, 0],
            },
            Frame {
                message_type: MessageType::Pong,
                flags: 0,
                sequence: 10,
                payload: vec![],
            },
        ];
        let bytes = frames.iter().try_fold(Vec::new(), |mut bytes, frame| {
            bytes.extend(encode_frame(frame)?);
            Ok::<_, ProtocolError>(bytes)
        });
        let mut parser = FrameParser::default();
        let decoded = parser.push(&bytes.unwrap_or_default());
        assert_eq!(decoded[0].payload, hello_payload);
        assert_eq!(decoded[1].message_type, MessageType::Capabilities);
        assert_eq!(decoded[2].message_type, MessageType::Pong);
    }
    #[test]
    fn incomplete_identity_frame_is_rejected_and_parser_resynchronizes() {
        let mut broken_hello = encode_frame(&Frame {
            message_type: MessageType::Hello,
            flags: 0,
            sequence: 1,
            payload: vec![0; 12],
        })
        .unwrap_or_default();
        let _ = broken_hello.pop(); // Regression for the observed dropped terminal CRC byte.
        let pong = encode_frame(&Frame {
            message_type: MessageType::Pong,
            flags: 0,
            sequence: 2,
            payload: vec![],
        })
        .unwrap_or_default();
        broken_hello.extend(pong);
        let mut parser = FrameParser::default();
        let frames = parser.push(&broken_hello);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].message_type, MessageType::Pong);
        assert!(parser.stats.crc_failures >= 1);
    }
    #[test]
    fn noise_length_crc_and_fuzz_are_safe() {
        let mut p = FrameParser::default();
        p.push(&[
            1, 2, 3, b'B', b'M', b'E', b'G', 0, 1, 11, 0, 0xff, 0xff, 0, 0, 0, 0,
        ]);
        assert!(p.stats.invalid_frames > 0);
        let mut broken = encode_frame(&frame(1)).unwrap_or_default();
        broken[14] ^= 1;
        p.push(&broken);
        assert!(p.stats.crc_failures > 0);
        for seed in 0..256u16 {
            let noise = vec![seed as u8; (seed % 31) as usize];
            p.push(&noise);
        }
    }
    #[test]
    fn integrity_gap_duplicate_and_out_of_order() {
        let mut m = IntegrityMonitor::default();
        m.observe_frame(&frame(1));
        m.observe_frame(&frame(3));
        m.observe_frame(&frame(3));
        m.observe_frame(&frame(2));
        assert_eq!(m.counters.missing_packet_sequences, 1);
        assert_eq!(m.counters.duplicate_packets, 1);
        assert_eq!(m.counters.out_of_order_packets, 1);
        m.observe_samples(1, 1, 0);
        m.observe_samples(3, 1, 0);
        assert_eq!(m.counters.missing_sample_sequences, 1);
    }
    #[test]
    fn integrity_sequence_wraparound_is_contiguous() {
        let mut m = IntegrityMonitor::default();
        m.observe_frame(&frame(u32::MAX));
        m.observe_frame(&frame(0));
        m.observe_samples(u32::MAX, 1, 0);
        m.observe_samples(0, 1, 0);
        assert_eq!(m.counters.missing_packet_sequences, 0);
        assert_eq!(m.counters.missing_sample_sequences, 0);
    }
}
