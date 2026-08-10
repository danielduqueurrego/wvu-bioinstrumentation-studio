# USB protocol specification v0.2

Status: controlled Phase 4 protocol for synchronized multi-channel course capture. Protocol major
remains `0`; v0.1 readers/recordings remain supported by the desktop reader.

## Transport and frame

The controlled UNO R4 WiFi uses USB CDC configured at **921600 baud**. All multibyte values are
little-endian. A frame is:

```text
0..3       ASCII "BMEG"
4          protocol major (0)
5          protocol minor (2)
6          message type
7          flags
8..9       payload length
10..13     packet sequence
14..       payload
last 2     CRC-16/CCITT-FALSE over all preceding frame bytes
```

The maximum payload is 1024 bytes. The host resynchronizes on magic, rejects bad lengths,
unsupported protocol versions, and invalid CRCs without crashing.

## Identity and capabilities

`HELLO` is `uint32 firmware_build`, `uint32 device_id`, `uint8 board_type`, `uint8 adc_max_bits`,
`uint8 max_analog_channels`, `uint8 mode_flags`. The Phase 4 controlled firmware identity is
protocol `0.2`, build `0x00010002`, device `0x554E4F34`, maximum six analog channels, and
12-/14-bit support.

`CAPABILITIES` advertises six analog channels, simultaneous-frame and fixed pulse-ox-cycle modes,
and the supported frame/cycle rates. Application code must use the profile and capabilities rather
than guessing an arbitrary configuration.

## CONFIGURE payloads

### Mode 0: simultaneous analog frame

```text
uint8   mode = 0
uint8   adc_resolution_bits       // 12 or 14
uint32  frame_rate_hz             // 1..1000
uint8   channel_count             // 1..6
uint8   analog_pin_ids[count]     // A0=0 through A5=5, unique
uint8   output_flags              // bit 0 permits D4 green while acquiring
```

One logical record contains a sequentially read value for every configured pin. They share the
logical record sequence/timestamp, but are not electrically simultaneous ADC conversions.

### Mode 1: fixed pulse-ox four-state cycle

```text
uint8   mode = 1
uint8   adc_resolution_bits = 14
uint32  state_dwell_us = 1000
uint8   analog_input_count = 2
uint8   tx_pin = 0               // A0
uint8   rx_pin = 1               // A1
uint8   reserved = 0
```

The firmware repeats RED, DARK 1, IR, DARK 2. D5 is HIGH only in RED; D6 is HIGH only in IR;
they are never HIGH together. D4, D5, and D6 are LOW before configuration, after Stop, on a
watchdog/protocol/configuration fault, and while idle.

## SAMPLE_BATCH

```text
uint32  first_record_sequence
uint64  first_timestamp_us
uint32  logical_record_period_us
uint8   field_count
uint8   record_count
uint16  status_flags
uint16  values[record_count][field_count]  // record-major
```

Normal course profiles use `field_count` equal to their configured analog channels. Pulse-ox uses
eight fields in this fixed order:

```text
red_TX, dark1_TX, ir_TX, dark2_TX, red_RX, dark1_RX, ir_RX, dark2_RX
```

`status_flags` bit 0 means valid acquisition data; bit 1 indicates ADC clipping observed; bit 2
indicates the firmware could not service its schedule; bit 6 is reserved for a pulse-ox LED safety
fault. The host exposes CRC, invalid frame, packet/sample sequence, firmware-overflow, and
bounded-host-queue counters separately.

## STATUS payload

v0.2 STATUS payload is two bytes: `acquiring` (`0` or `1`) and `digital_output_mask`. Mask bit 0
is D4, bit 1 is D5, and bit 2 is D6. This lets the host verify that the green output was active
during the BP course capture and that all controlled outputs are LOW after Stop. A pulse cycle is
verified by its raw field order and firmware state machine; STATUS is not a per-millisecond LED
trace.

## Compatibility

v0.1 single-channel BMEG files remain readable. The current host accepts protocol minor 1 for
legacy parsing and minor 2 for controlled Phase 4 capture. Existing v0.1 files are never
relabelled as course-profile recordings.
