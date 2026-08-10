# USB protocol specification v0.3

Status: controlled Phase 6 protocol for instructor-configurable BMEG 420L course labs. Protocol
major remains `0`; the desktop reader retains legacy Phase 1–5 file support.

## Transport and identity

USB CDC is configured for **921600 baud**. Multibyte fields are little-endian. Frames retain the
`BMEG` magic, major/minor, message type, flags, payload length, packet sequence, payload, and
CRC-16/CCITT-FALSE described by v0.2. The current controlled reference firmware is protocol
`0.3`, build `0x00010003`, device `0x554E4F34`.

## CAPABILITIES

The controlled v0.3 `CAPABILITIES` payload is:

```text
uint8   first supported ADC resolution        // 12
uint8   second supported ADC resolution       // 14
uint8   maximum simultaneous analog channels // 6
uint8   acquisition-mode bits                // bit 0 simultaneous; bit 1 pulse-ox 4-state
uint8   controlled-output mask                // bit 0 D4, bit 1 D5, bit 2 D6
uint8   supported-rate count
uint16  supported_rate_hz[count]             // 100, 200, 250, 500, 1000
```

The host parses and retains this information during the normal identity handshake. A lab can be
authored offline, but the host validates its requested mode, channel count, ADC resolution, rate,
and output mask against these advertised limits immediately before CONFIGURE. Earlier capability
payloads are readable but do not claim these richer limits.

## CONFIGURE mode 0 — simultaneous analog frame

```text
uint8   mode = 0
uint8   adc_resolution_bits                 // 12 or 14
uint32  frame_rate_hz                       // advertised supported rate
uint8   channel_count                       // 1..6
uint8   analog_pin_ids[count]               // A0=0 through A5=5, unique
uint8   output_mask                         // D4/D5/D6 high-while-recording permission
```

Each record has one logical sequence and timestamp. Reads are in deterministic sequential ADC
order within a frame, not electrically simultaneous conversions. In simultaneous mode D5/D6 are
not permitted HIGH while recording; this preserves the safe pulse-ox LED boundary.

## CONFIGURE mode 1 — fixed pulse-ox four-state cycle

```text
uint8   mode = 1
uint8   adc_resolution_bits                 // 12 or 14
uint32  state_dwell_us                      // 250..5000
uint8   analog_input_count = 2
uint8   tx_pin                              // A0=0 through A5=5
uint8   rx_pin                              // A0=0 through A5=5, distinct
uint8   red_output_pin                      // D4=4 through D6=6
uint8   ir_output_pin                       // D4=4 through D6=6, distinct
```

The fixed phase sequence is RED ON, DARK 1, IR ON, DARK 2. The mapped RED pin is HIGH only in
RED ON; the mapped IR pin is HIGH only in IR ON. They are never HIGH together. The raw cycle
field order is unchanged:

```text
red_TX,dark1_TX,ir_TX,dark2_TX,red_RX,dark1_RX,ir_RX,dark2_RX
```

## Safety and compatibility

The firmware forces D4/D5/D6 LOW at startup, idle, Stop, rejected configuration, protocol fault,
and watchdog fault. STATUS remains `acquiring, digital_output_mask`. A successful upload alone
does not prove identity: acquisition requires HELLO, CAPABILITIES, PONG, a compatible identity,
and successful CONFIGURE/START.

v0.1 single-channel recordings and v0.2/v0.3 synchronized recordings remain readable. Existing
files are not relabelled or rewritten by this protocol update.
