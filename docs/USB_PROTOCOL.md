# USB Protocol

## Current reference

The current WVU reference firmware uses binary USB protocol **0.3**, build `0x00010003`, and device ID `0x554E4F34`. USB CDC is configured for 921600 baud. Multibyte fields are little-endian.

Frames use the `BMEG` magic, protocol major/minor, message type, flags, payload length, packet sequence, payload, and CRC-16/CCITT-FALSE.

## Capabilities

The firmware reports supported ADC resolutions (12 and 14 bit), a maximum of six simultaneous analog channels, acquisition-mode support, controlled outputs D4/D5/D6, and supported rates (100, 200, 250, 500, and 1000 Hz). The desktop validates the active lab against these capabilities before it sends CONFIGURE.

## Simultaneous analog configuration

```text
uint8   mode = 0
uint8   adc_resolution_bits
uint32  frame_rate_hz
uint8   channel_count              // 1..6
uint8   analog_pin_ids[count]      // A0=0 through A5=5, unique
uint8   output_mask                // D4/D5/D6 high-while-recording permission
```

Each record has a logical sequence and timestamp. Analog reads occur in deterministic sequential order inside a frame. In this mode D5/D6 cannot be HIGH while recording.

## Pulse-ox four-state configuration

```text
uint8   mode = 1
uint8   adc_resolution_bits
uint32  state_dwell_us             // 250..5000
uint8   analog_input_count = 2
uint8   tx_pin                     // A0 through A5
uint8   rx_pin                     // A0 through A5, distinct
uint8   red_output_pin             // D4 through D6
uint8   ir_output_pin              // D4 through D6, distinct
```

The fixed phase order is RED ON, DARK 1, IR ON, DARK 2. Raw cycle fields are:

```text
red_TX,dark1_TX,ir_TX,dark2_TX,red_RX,dark1_RX,ir_RX,dark2_RX
```

RED and IR are never HIGH together.

## Safety and compatibility

D4/D5/D6 are forced LOW at startup, idle, Stop, rejected configuration, protocol fault, and watchdog fault. Acquisition requires HELLO, CAPABILITIES, PONG, valid CRC, the expected firmware identity, successful CONFIGURE, and START.

The recording reader retains compatibility with earlier single-channel and synchronized BMEG layouts. Existing files are not relabelled or rewritten.
