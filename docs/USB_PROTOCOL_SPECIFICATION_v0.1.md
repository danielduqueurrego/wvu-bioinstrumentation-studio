# USB protocol specification v0.1

Status: legacy Phase 1 implementation contract. New multi-channel capture uses
[`USB_PROTOCOL_SPECIFICATION_v0.2.md`](USB_PROTOCOL_SPECIFICATION_v0.2.md); this document is
retained because v0.1 recordings remain readable.

## 1. Byte order

All multibyte integers use little-endian byte order.

## 2. Frame

```text
offset  size  field
0       4     magic = ASCII "BMEG" (42 4D 45 47)
4       1     protocol major
5       1     protocol minor
6       1     message type
7       1     flags
8       2     payload length
10      4     packet sequence
14      N     payload
14+N    2     CRC-16/CCITT-FALSE over bytes 0 through 13+N
```

Maximum payload length for v0.1: 1024 bytes.

The parser shall resynchronize by scanning for the magic bytes and shall reject invalid lengths, unsupported versions, and CRC failures without crashing.

## 3. Message types

```text
0x01 HELLO
0x02 CAPABILITIES
0x03 CONFIGURE
0x04 CONFIG_ACK
0x05 START
0x06 STOP
0x07 STATUS
0x08 SAMPLE_BATCH
0x09 EVENT_MARKER
0x0A ERROR
0x0B PING
0x0C PONG
```

## 4. HELLO payload

```text
uint32 firmware_build
uint32 device_id
uint8  board_type       // 1 = UNO R4 WiFi
uint8  adc_max_bits
uint8  max_channels
uint8  reserved
```

Human-readable firmware metadata may be requested later; do not add variable strings to the Phase 1 hot path.

## 5. CONFIGURE payload v0.1

```text
uint32 requested_sample_rate_hz
uint8  adc_bits
uint8  channel_count
uint8  analog_pin_ids[channel_count]
uint8  digital_status_count
uint8  digital_status_pin_ids[digital_status_count]
```

Phase 1 may support one analog channel while preserving the extensible decoder.

## 6. SAMPLE_BATCH payload

```text
uint32 first_sample_sequence
uint64 first_timestamp_us
uint32 sample_period_us
uint8  channel_count
uint8  sample_count_per_channel
uint16 status_flags
uint16 samples[sample_count_per_channel][channel_count]
```

Packet sequence and sample sequence are independent.

## 7. Status flags

```text
bit 0  acquisition active
bit 1  ADC clipping observed
bit 2  firmware ring-buffer overflow
bit 3  command timeout
bit 4  LOD+ active
bit 5  LOD- active
bit 6  pulse-ox LED safety fault
bits 7–15 reserved
```

## 8. Timing

- Board timestamps are monotonic microseconds extended to 64 bits.
- Sampling must not be scheduled from the desktop.
- The host reconstructs sample timestamps using the first timestamp and sample period.
- Host receipt time is recorded separately for latency diagnostics.

## 9. Integrity counters

The desktop shall maintain:

- Received packets
- CRC failures
- Invalid frames
- Missing packet sequences
- Duplicate packets
- Out-of-order packets
- Missing sample sequences
- Firmware-reported overflows
- Host-channel overflows
- Reconnects
