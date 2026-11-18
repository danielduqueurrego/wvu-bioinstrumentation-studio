//! Binary USB protocol. Constants here are the host-side source of truth.

use std::collections::VecDeque;

pub const MAGIC: [u8; 4] = *b"BMEG";
pub const PROTOCOL_MAJOR: u8 = 0;
/// Protocol v0.4 adds explicit timestamps for every synchronized record and a
/// cumulative firmware acquisition-queue overflow count. The parser retains
/// v0.1-v0.3 support for diagnostics and historical protocol fixtures.
pub const PROTOCOL_MINOR: u8 = 4;
pub const LEGACY_PROTOCOL_MINOR: u8 = 1;
/// USB CDC configuration used by the controlled v0.4 firmware. This leaves
/// headroom for a six-channel 1 kHz logical-frame stream.
pub const CONTROLLED_SERIAL_BAUD: u32 = 921_600;
pub const HEADER_LEN: usize = 14;
pub const CRC_LEN: usize = 2;
pub const MAX_PAYLOAD_LEN: usize = 1024;

/// Immutable identity expected from the controlled UNO R4 WiFi reference firmware.
/// The matching values are encoded in `firmware/reference_unor4wifi`'s HELLO frame.
pub const REFERENCE_FIRMWARE_BUILD: u32 = 0x0001_0006;
pub const REFERENCE_DEVICE_ID: u32 = 0x554e_4f34;
pub const SAMPLE_STATUS_ACQUIRING: u16 = 0x0001;
pub const SAMPLE_STATUS_QUEUE_OVERFLOW: u16 = 0x0004;
pub const SAMPLE_STATUS_EXPLICIT_TIMESTAMPS: u16 = 0x0008;

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
    /// Protocol minor version of the most recent accepted frame. The parser
    /// accepts v0.1-v0.4, so diagnostics report what the board actually sent.
    pub last_protocol_minor: Option<u8>,
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
            self.stats.last_protocol_minor = Some(raw[5]);
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
    pub sample_period_us: u32,
    pub channel_count: u8,
    /// One firmware-measured timestamp per synchronized record. Protocol v0.4
    /// carries these explicitly; legacy v0.1-v0.3 payloads are expanded here
    /// solely so historical fixtures remain readable by the common API.
    pub timestamps_us: Vec<u64>,
    pub samples: Vec<u16>,
    pub status_flags: u16,
    /// Cumulative packet-queue overflow events reported by protocol v0.4.
    /// Legacy batches have no cumulative counter and therefore use `None`.
    pub acquisition_queue_overflows: Option<u32>,
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
        let status_flags = u16::from_le_bytes([payload[18], payload[19]]);
        let explicit_timestamps = status_flags & SAMPLE_STATUS_EXPLICIT_TIMESTAMPS != 0;
        let timestamp_bytes = if explicit_timestamps {
            usize::from(count) * 8
        } else {
            0
        };
        let v4_header_bytes = if explicit_timestamps { 24 } else { 20 };
        let expected =
            v4_header_bytes + timestamp_bytes + usize::from(channels) * usize::from(count) * 2;
        if payload.len() != expected {
            return Err(ProtocolError::InvalidPayload("sample batch length"));
        }
        let first_timestamp_us = u64::from_le_bytes(
            payload[4..12]
                .try_into()
                .map_err(|_| ProtocolError::InvalidPayload("timestamp"))?,
        );
        let sample_period_us = u32::from_le_bytes(
            payload[12..16]
                .try_into()
                .map_err(|_| ProtocolError::InvalidPayload("period"))?,
        );
        let timestamps_us = if explicit_timestamps {
            let timestamps = payload[24..24 + timestamp_bytes]
                .as_chunks::<8>()
                .0
                .iter()
                .map(|bytes| u64::from_le_bytes(*bytes))
                .collect::<Vec<_>>();
            if timestamps.first().copied() != Some(first_timestamp_us)
                || timestamps.windows(2).any(|pair| pair[1] <= pair[0])
            {
                return Err(ProtocolError::InvalidPayload(
                    "sample timestamps must be strictly monotonic",
                ));
            }
            timestamps
        } else {
            (0..u64::from(count))
                .map(|index| first_timestamp_us + u64::from(sample_period_us) * index)
                .collect()
        };
        let samples_offset = v4_header_bytes + timestamp_bytes;
        let samples = payload[samples_offset..]
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
            sample_period_us,
            channel_count: channels,
            timestamps_us,
            samples,
            status_flags,
            acquisition_queue_overflows: explicit_timestamps
                .then(|| u32::from_le_bytes([payload[20], payload[21], payload[22], payload[23]])),
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
            || self.timestamps_us.len() != self.samples.len() / usize::from(self.channel_count)
            || self.timestamps_us.is_empty()
            || self.timestamps_us.windows(2).any(|pair| pair[1] <= pair[0])
        {
            return Err(ProtocolError::InvalidPayload("sample batch shape"));
        }
        let mut output =
            Vec::with_capacity(24 + self.timestamps_us.len() * 8 + self.samples.len() * 2);
        output.extend_from_slice(&self.first_sample_sequence.to_le_bytes());
        output.extend_from_slice(&self.timestamps_us[0].to_le_bytes());
        output.extend_from_slice(&self.sample_period_us.to_le_bytes());
        output.push(self.channel_count);
        output.push((self.samples.len() / usize::from(self.channel_count)) as u8);
        output.extend_from_slice(
            &(self.status_flags | SAMPLE_STATUS_EXPLICIT_TIMESTAMPS).to_le_bytes(),
        );
        output.extend_from_slice(
            &self
                .acquisition_queue_overflows
                .unwrap_or_default()
                .to_le_bytes(),
        );
        for timestamp in &self.timestamps_us {
            output.extend_from_slice(&timestamp.to_le_bytes());
        }
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
    /// Records in received sample batches that carry
    /// `SAMPLE_STATUS_QUEUE_OVERFLOW`. The controlled firmware sets that flag
    /// on the next batch after it skipped one or more sampling deadlines (no
    /// record and no sequence gap) or dropped a full batch. Skipped deadlines
    /// appear in no other counter, so this is counted separately and never
    /// added to `firmware_overflows`, which would double count real queue
    /// overflows. Every record of such a batch stores the flag in its BMEG
    /// status field. Metadata written before this field existed reads as 0.
    #[serde(default)]
    pub overrun_flagged_records: u64,
}
#[derive(Default)]
pub struct IntegrityMonitor {
    packet_next: Option<u32>,
    sample_next: Option<u32>,
    firmware_queue_overflows: Option<u32>,
    pub counters: IntegrityCounters,
}
impl IntegrityMonitor {
    fn queue_overflow_delta(&mut self, current: u32) -> u64 {
        let delta = self.firmware_queue_overflows.map_or(current, |previous| {
            current.checked_sub(previous).unwrap_or(current)
        });
        self.firmware_queue_overflows = Some(current);
        u64::from(delta)
    }

    pub fn observe_frame(&mut self, frame: &Frame) {
        self.counters.received_packets += 1;
        self.observe_sequence(frame.sequence, false);
    }
    pub fn observe_samples(
        &mut self,
        first: u32,
        count: u32,
        status_flags: u16,
        acquisition_queue_overflows: Option<u32>,
    ) {
        if let Some(current) = acquisition_queue_overflows {
            self.counters.firmware_overflows += self.queue_overflow_delta(current);
        } else if status_flags & SAMPLE_STATUS_QUEUE_OVERFLOW != 0 {
            self.counters.firmware_overflows += 1;
        }
        if status_flags & SAMPLE_STATUS_QUEUE_OVERFLOW != 0 {
            self.counters.overrun_flagged_records += u64::from(count);
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

    pub fn observe_firmware_queue_overflows(&mut self, current: u32) {
        self.counters.firmware_overflows += self.queue_overflow_delta(current);
    }

    /// Forgets the sample-sequence and cumulative-overflow baselines. The
    /// firmware restarts both at START, so a batch left over from a previous
    /// run must not make the first record of a new recording look missing,
    /// duplicated, or out of order. Packet-sequence tracking is unaffected.
    pub fn reset_sample_baseline(&mut self) {
        self.sample_next = None;
        self.firmware_queue_overflows = None;
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
            sample_period_us: 1_000,
            channel_count: 6,
            timestamps_us: vec![123, 1_137],
            samples: vec![1, 2, 3, 4, 5, 6, 11, 12, 13, 14, 15, 16],
            status_flags: SAMPLE_STATUS_ACQUIRING,
            acquisition_queue_overflows: Some(0),
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
        assert_eq!(decoded.timestamps_us, [123, 1_137]);
    }

    #[test]
    fn twenty_record_eight_channel_batch_stays_within_the_controlled_payload_limit() {
        let batch = SampleBatch {
            first_sample_sequence: 10,
            sample_period_us: 1_000,
            channel_count: 8,
            timestamps_us: (0..20).map(|index| 100 + index * 1_000).collect(),
            samples: vec![123; 20 * 8],
            status_flags: SAMPLE_STATUS_ACQUIRING,
            acquisition_queue_overflows: Some(2),
        };
        let payload = batch
            .to_payload()
            .unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(payload.len(), 24 + 20 * 8 + 20 * 8 * 2);
        assert!(payload.len() <= 1_024);
        let decoded =
            SampleBatch::from_payload(&payload).unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(decoded.samples.len(), 20 * 8);
        assert_eq!(decoded.acquisition_queue_overflows, Some(2));
    }

    #[test]
    fn legacy_uniform_batch_payloads_remain_parseable() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&7u32.to_le_bytes());
        payload.extend_from_slice(&10_000u64.to_le_bytes());
        payload.extend_from_slice(&1_000u32.to_le_bytes());
        payload.push(1);
        payload.push(3);
        payload.extend_from_slice(&SAMPLE_STATUS_ACQUIRING.to_le_bytes());
        for sample in [11u16, 12, 13] {
            payload.extend_from_slice(&sample.to_le_bytes());
        }
        let decoded =
            SampleBatch::from_payload(&payload).unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(decoded.timestamps_us, [10_000, 11_000, 12_000]);
        assert_eq!(decoded.acquisition_queue_overflows, None);
    }

    #[test]
    fn cumulative_queue_overflow_counter_is_counted_once_across_batch_and_status() {
        let mut monitor = IntegrityMonitor::default();
        monitor.observe_samples(0, 10, SAMPLE_STATUS_QUEUE_OVERFLOW, Some(2));
        monitor.observe_firmware_queue_overflows(2);
        monitor.observe_samples(20, 10, SAMPLE_STATUS_QUEUE_OVERFLOW, Some(3));
        assert_eq!(monitor.counters.firmware_overflows, 3);
        assert_eq!(monitor.counters.missing_sample_sequences, 10);
    }

    #[test]
    fn skipped_deadline_flags_are_counted_separately_from_queue_overflows() {
        let mut monitor = IntegrityMonitor::default();
        // A skipped deadline sets the flag without changing the cumulative
        // queue-overflow counter or leaving a sequence gap.
        monitor.observe_samples(0, 10, SAMPLE_STATUS_ACQUIRING, Some(0));
        monitor.observe_samples(
            10,
            10,
            SAMPLE_STATUS_ACQUIRING | SAMPLE_STATUS_QUEUE_OVERFLOW,
            Some(0),
        );
        monitor.observe_samples(20, 4, SAMPLE_STATUS_ACQUIRING, Some(0));
        assert_eq!(monitor.counters.overrun_flagged_records, 10);
        assert_eq!(monitor.counters.firmware_overflows, 0);
        assert_eq!(monitor.counters.missing_sample_sequences, 0);
    }

    #[test]
    fn integrity_counters_written_before_the_flag_counter_still_read() {
        let legacy = serde_json::to_value(IntegrityCounters::default())
            .map(|mut value| {
                if let Some(object) = value.as_object_mut() {
                    object.remove("overrun_flagged_records");
                }
                value
            })
            .unwrap_or_else(|error| panic!("{error}"));
        let counters: IntegrityCounters =
            serde_json::from_value(legacy).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(counters.overrun_flagged_records, 0);
    }

    #[test]
    fn parser_reports_the_protocol_minor_version_it_accepted() {
        let mut legacy = encode_frame(&frame(1)).unwrap_or_default();
        legacy[5] = LEGACY_PROTOCOL_MINOR;
        let crc_at = legacy.len() - CRC_LEN;
        let crc = crc16_ccitt_false(&legacy[..crc_at]).to_le_bytes();
        legacy[crc_at..].copy_from_slice(&crc);
        let mut parser = FrameParser::default();
        assert_eq!(parser.stats.last_protocol_minor, None);
        assert_eq!(parser.push(&legacy).len(), 1);
        assert_eq!(
            parser.stats.last_protocol_minor,
            Some(LEGACY_PROTOCOL_MINOR)
        );
        parser.push(&encode_frame(&frame(2)).unwrap_or_default());
        assert_eq!(parser.stats.last_protocol_minor, Some(PROTOCOL_MINOR));
    }

    #[test]
    fn queue_overflow_counter_reset_at_start_does_not_create_a_wrapping_spike() {
        let mut monitor = IntegrityMonitor::default();
        monitor.observe_firmware_queue_overflows(4);
        monitor.observe_firmware_queue_overflows(0);
        monitor.observe_firmware_queue_overflows(2);
        assert_eq!(monitor.counters.firmware_overflows, 6);
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
        m.observe_samples(1, 1, 0, None);
        m.observe_samples(3, 1, 0, None);
        assert_eq!(m.counters.missing_sample_sequences, 1);
    }
    #[test]
    fn integrity_sequence_wraparound_is_contiguous() {
        let mut m = IntegrityMonitor::default();
        m.observe_frame(&frame(u32::MAX));
        m.observe_frame(&frame(0));
        m.observe_samples(u32::MAX, 1, 0, None);
        m.observe_samples(0, 1, 0, None);
        assert_eq!(m.counters.missing_packet_sequences, 0);
        assert_eq!(m.counters.missing_sample_sequences, 0);
    }
}
