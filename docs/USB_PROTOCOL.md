# USB Protocol

## Current reference

The current WVU reference firmware uses binary USB protocol **0.4**, build `0x00010006`, and device ID `0x554E4F34`. USB CDC is configured for 921600 baud. Multibyte fields are little-endian.

Frames use the `BMEG` magic, protocol major/minor, message type, flags, payload length, packet sequence, payload, and CRC-16/CCITT-FALSE.

## Capabilities

The firmware reports supported ADC resolutions (12 and 14 bit), a maximum of six simultaneous analog channels, acquisition-mode support, controlled outputs D4/D5/D6, and supported rates (100, 200, 250, 500, and 1000 Hz). The desktop validates the active lab against these capabilities before it sends CONFIGURE. A CAPABILITIES payload that cannot be parsed in this layout fails the handshake (and Verify Firmware) as an incompatible firmware, because it cannot prove the board's limits.

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

The fixed phase order is RED ON, DARK 1, IR ON, DARK 2. The firmware waits the configured state dwell after each output transition before sampling both analog inputs. Raw cycle fields are:

```text
red_TX,dark1_TX,ir_TX,dark2_TX,red_RX,dark1_RX,ir_RX,dark2_RX
```

RED and IR are never HIGH together.

## Sample batches and measured timestamps

Protocol 0.4 `SAMPLE_BATCH` payloads retain the original 20-byte batch header and set flag `0x0008` to identify explicit per-record timestamps:

```text
uint32  first_record_sequence
uint64  first_timestamp_us
uint32  nominal_period_us
uint8   field_count
uint8   record_count
uint16  flags
uint32  acquisition_queue_overflows
uint64  timestamps_us[record_count]
uint16  values[record_count][field_count]
```

`first_timestamp_us` must match the first explicit timestamp. Timestamps are monotonic 64-bit microseconds extended by the firmware across the underlying `micros()` wrap. For pulse-ox, each timestamp records completion of the DARK 2 measurements for that cycle. The nominal period describes the requested configuration; it is not substituted for measured timestamps.

The firmware first finalizes records into a bounded raw-batch queue, then constructs protocol packets and CRCs outside the timing-critical sampling path. Serial transmission uses a second bounded queue, reported writable capacity, and partial writes. If backpressure fills the acquisition queue, the firmware drops the new batch, increments the cumulative `acquisition_queue_overflows` value, and reports the overflow flag rather than blocking acquisition or silently corrupting timing.

Batch flag `0x0004` has two meanings. It is set on the next stored batch after the firmware either dropped a batch (also counted in `acquisition_queue_overflows`) or found that one or more sampling deadlines had already passed and skipped them. A skipped deadline produces no record and no sequence gap, and it does not change `acquisition_queue_overflows`. The host therefore counts the records of flagged batches separately as `integrity.overrun_flagged_records` in the session summary and metadata. That count is never added to `firmware_overflows`, which counts each dropped batch once. Every record of a flagged batch also keeps the flag in its BMEG status field, and the skipped interval appears in the ±5% timing-tolerance count.

Build `0x00010006` extends the protocol 0.4 `STATUS` payload while remaining parseable by older v0.4 hosts:

```text
uint8   acquiring
uint8   output_mask
uint32  acquisition_queue_overflows
uint32  phase_intervals_outside_5_percent[4]  // RED, DARK 1, IR, DARK 2
uint32  phase_maximum_absolute_error_us[4]    // RED, DARK 1, IR, DARK 2
```

The host accounts for the cumulative overflow value once even when it appears in both status and batch frames. For pulse-ox recordings it preserves the four per-state timing checks in final metadata. Pulse-state deadlines advance by the configured dwell on one absolute timeline; they are not recalculated from the completion time of ADC, bookkeeping, packet construction, or serial transmission.

The desktop parser continues to accept legacy protocol 0.1–0.3 sample batches, whose record timestamps are expanded from their first timestamp and nominal period. New recordings require the current compatible reference firmware so their authoritative records use measured per-record timestamps.

## Firmware ERROR codes

`ERROR_MESSAGE` (type `0x0A`) carries a one-byte code. The controlled reference firmware sends only these codes:

| Code | Cause | Effect |
| --- | --- | --- |
| 1 | Command header has an unsupported protocol version, or its length does not match | Acquisition stops; configuration cleared; D4/D5/D6 LOW |
| 2 | Command CRC mismatch | Acquisition stops; configuration cleared; D4/D5/D6 LOW |
| 3 | CONFIGURE rejected (mode, channels, pins, ADC bits, rate, dwell, or outputs) | Any acquisition stops; configuration cleared; D4/D5/D6 LOW; no CONFIG_ACK |
| 4 | START without an accepted configuration, or START with a payload | START refused; acquisition never began |
| 5 | Command input buffer overflow | Acquisition stops; configuration cleared; D4/D5/D6 LOW |
| 6 | Command payload length above the protocol maximum | Acquisition stops; configuration cleared; D4/D5/D6 LOW |
| 7 | Host-command watchdog: no command for 5 s while acquiring | Acquisition stops after the final partial batch; D4/D5/D6 LOW |

The host fails a pending CONFIGURE or START wait as soon as a new ERROR arrives (an ERROR received before the handshake finished is logged, not counted against the new recording) and describes each code with its likely cause and next step. Any other code is reported as not used by the reference firmware.

## Safety and compatibility

D4/D5/D6 are forced LOW at startup, idle, Stop, rejected configuration, protocol fault, and watchdog fault. Acquisition requires HELLO, CAPABILITIES, PONG, valid CRC, the expected firmware identity, successful CONFIGURE, and START.

The recording reader retains compatibility with earlier single-channel and synchronized BMEG layouts. Existing files are not relabelled or rewritten. New metadata includes requested and measured rate, interval median/minimum/maximum/p99, the documented ±5% timing-tolerance count, timestamp source, and acquisition-queue overflows.
