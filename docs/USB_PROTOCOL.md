# USB Protocol

This is the binary protocol between WVU Bioinstrumentation Studio and the controlled reference firmware. The sources of truth are `firmware/reference_unor4wifi/reference_unor4wifi.ino` and `src-tauri/src/protocol.rs`; this page describes them and must change with them.

## Current reference

The current WVU reference firmware uses binary USB protocol **0.4**, build `0x00010006`, and device ID `0x554E4F34` (the ASCII letters `UNO4` read as a big-endian number). It runs on the RA4M1's native USB CDC. The sketch calls `Serial.begin(921600)` and the desktop opens the port at 921600 baud, but over native USB CDC that value is nominal: it does not limit the link.

All multibyte integers are **little-endian**.

## Frame layout

Every frame in both directions has a 14-byte header, the payload, and a 2-byte CRC:

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 4 | Magic: ASCII `BMEG` (`42 4D 45 47`) |
| 4 | 1 | Protocol major (`0`) |
| 5 | 1 | Protocol minor (`4`) |
| 6 | 1 | Message type (table below) |
| 7 | 1 | Flags (always `0`; not interpreted) |
| 8 | 2 | Payload length `N` (u16) |
| 10 | 4 | Packet sequence (u32) |
| 14 | `N` | Payload |
| 14 + `N` | 2 | CRC (u16) |

A frame is `16 + N` bytes long.

**CRC:** CRC-16/CCITT-FALSE. The polynomial is `0x1021` and the initial value is `0xFFFF`, with no input or output reflection and no final XOR. The check value for ASCII `123456789` is `0x29B1`, and a host test pins it. The CRC covers bytes `0` to `13 + N` (header and payload) and is sent little-endian.

**Packet sequence:** the firmware numbers every frame it sends with one u32 counter, which wraps. The desktop counts missing, duplicated, and out-of-order packet sequences with serial-number arithmetic. The firmware does not check the sequence numbers of host commands.

**Length limits:**

- The desktop accepts payloads up to 1024 bytes.
- The firmware accepts command payloads up to 1024 bytes and never sends a payload larger than 264 bytes. That is the largest sample batch: `24 + 10 × (8 + 8 × 2)` bytes, so the largest transmitted frame is 280 bytes.

**Resynchronization:** both sides scan for the magic.

- The desktop skips one byte and counts the error when a frame has a bad CRC or a length above 1024. It drops and counts a valid frame with an unsupported version or an unknown type.
- The parser accepts protocol 0.1–0.4. New recordings still require the 0.4 reference firmware (see [Handshake](#handshake-start-and-stop)).

## Message types

| Type | Name | Direction | Payload |
| ---: | --- | --- | --- |
| `0x01` | HELLO | board → desktop | 12 bytes, below |
| `0x02` | CAPABILITIES | board → desktop | 16 bytes, below |
| `0x03` | CONFIGURE | desktop → board | 9–14 bytes (simultaneous) or 11 bytes (pulse ox), below |
| `0x04` | CONFIG_ACK | board → desktop | empty |
| `0x05` | START | desktop → board | empty; START with a payload is refused with ERROR 4 |
| `0x06` | STOP | desktop → board | empty |
| `0x07` | STATUS | board → desktop | 38 bytes, below |
| `0x08` | SAMPLE_BATCH | board → desktop | 24-byte header plus records, below |
| `0x09` | EVENT_MARKER | — | Known to the desktop parser but never sent. Recording markers are desktop metadata and are not sent to the board. |
| `0x0A` | ERROR_MESSAGE | board → desktop | 1-byte code, below |
| `0x0B` | PING | desktop → board | empty |
| `0x0C` | PONG | board → desktop | empty |

### HELLO

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 4 | Firmware build (u32), `0x00010006` |
| 4 | 4 | Device ID (u32), `0x554E4F34` |
| 8 | 4 | The reference firmware sends `01 0E 06 03`. The desktop does not interpret these bytes. |

### CAPABILITIES

| Offset | Size | Field | Reference value |
| ---: | ---: | --- | --- |
| 0 | 1 | Minimum ADC resolution, bits | 12 |
| 1 | 1 | Maximum ADC resolution, bits | 14 |
| 2 | 1 | Maximum analog channels | 6 |
| 3 | 1 | Mode bits: `0x01` simultaneous, `0x02` pulse-ox four-state | `0x03` |
| 4 | 1 | Output bits: `0x01` D4, `0x02` D5, `0x04` D6 | `0x07` |
| 5 | 1 | Rate count `n` | 5 |
| 6 | `2n` | Supported frame rates in Hz (u16 each) | 100, 200, 250, 500, 1000 |

The desktop treats the minimum and maximum resolutions as the only two supported values; resolutions between them do not count. Before it sends CONFIGURE, it checks the active lab's mode, channel count, ADC resolution, rate, and outputs against these capabilities. A CAPABILITIES payload that is not exactly `6 + 2n` bytes, or whose minimum exceeds its maximum, fails the handshake (and Verify Firmware) as incompatible firmware, because it cannot prove the board's limits.

### CONFIGURE: simultaneous analog (mode 0)

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 1 | `mode = 0` |
| 1 | 1 | ADC resolution: 12 or 14 |
| 2 | 4 | Frame rate in Hz (u32): 100, 200, 250, 500, or 1000 |
| 6 | 1 | Channel count `c`, 1–6 |
| 7 | `c` | Analog pin IDs (A0 = 0 … A5 = 5), unique |
| 7 + `c` | 1 | Output mask: `0x01` D4, `0x02` D5, `0x04` D6. Any other bit is rejected. |

The payload is exactly `8 + c` bytes. The board reads the analog channels in this order within each frame.

The output mask lists the pins held HIGH while recording. The firmware rejects a mask that sets **both** D5 and D6, but it accepts D5 alone or D6 alone. The rule that D5 and D6 are never HIGH while recording in this mode is enforced by the desktop: lab validation rejects HIGH-while-recording on D5 or D6, so the desktop only ever sends `0x00` or `0x01` (D4) in this mode. Making the firmware reject any D5/D6 bit here is a deferred firmware change (see [AGENT_HANDOFF](AGENT_HANDOFF.md#deferred-decisions)).

### CONFIGURE: pulse-ox four-state (mode 1)

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 1 | `mode = 1` |
| 1 | 1 | ADC resolution: 12 or 14 |
| 2 | 4 | State dwell in µs (u32), 250–5000 |
| 6 | 1 | Analog input count, must be `2` |
| 7 | 1 | TX pin (0–5 = A0–A5) |
| 8 | 1 | RX pin (0–5), different from TX |
| 9 | 1 | RED output pin (4–6 = D4–D6) |
| 10 | 1 | IR output pin (4–6), different from RED |

The payload is exactly 11 bytes. The fixed phase order is RED ON, DARK 1, IR ON, DARK 2. Each phase lasts one dwell on one absolute timeline: the next deadline is the previous deadline plus the dwell, not the time the last ADC read finished. The board samples both inputs when the phase's dwell has elapsed. The raw cycle fields are:

```text
red_TX,dark1_TX,ir_TX,dark2_TX,red_RX,dark1_RX,ir_RX,dark2_RX
```

Every phase change first drives D4, D5, and D6 LOW, then sets at most one of RED or IR HIGH, so RED and IR are never HIGH together.

### STATUS

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 1 | `acquiring` (0 or 1) |
| 1 | 1 | Output mask read back from the D4/D5/D6 pin levels (`0x01` D4, `0x02` D5, `0x04` D6) |
| 2 | 4 | Cumulative `acquisition_queue_overflows` (u32) |
| 6 | 16 | `phase_intervals_outside_5_percent[4]` (u32 each), in the order RED, DARK 1, IR, DARK 2 |
| 22 | 16 | `phase_maximum_absolute_error_us[4]` (u32 each), in the order RED, DARK 1, IR, DARK 2 |

Build `0x00010006` extended the v0.4 STATUS to this 38-byte layout. Older v0.4 hosts can still parse it.

The firmware sends STATUS in two cases:

- **After START**, with `acquiring = 1`, once the new outputs have been applied.
- **After STOP**, with `acquiring = 0`. This STATUS is queued behind the final partial batch and every batch still waiting, so it is the last frame of the run. STOP while idle also produces this STATUS.

The desktop uses only STATUS frames from the current run for integrity and timing evidence. It accepts the per-phase counters as final only from a terminal (`acquiring = 0`) STATUS.

### SAMPLE_BATCH

Protocol 0.4 batches set flag `0x0008`, which means explicit per-record timestamps. The payload is:

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 4 | `first_record_sequence` (u32) |
| 4 | 8 | `first_timestamp_us` (u64) |
| 12 | 4 | `nominal_period_us` (u32): `1,000,000 / frame rate`, or `4 × dwell` for pulse ox |
| 16 | 1 | `field_count` `f`: the channel count, or 8 for pulse ox |
| 17 | 1 | `record_count` `r`: 10 from the reference firmware, fewer in the final batch of a run |
| 18 | 2 | Flags (u16), below |
| 20 | 4 | Cumulative `acquisition_queue_overflows` (u32) |
| 24 | `8r` | `timestamps_us[r]` (u64 each) |
| 24 + `8r` | `2rf` | `values[r][f]` (u16 ADC counts), record after record |

The payload is exactly `24 + 8r + 2rf` bytes.

| Flag | Meaning |
| --- | --- |
| `0x0001` | Acquiring. Set on every batch from the reference firmware. |
| `0x0004` | Overrun: a batch was dropped or a sampling deadline was skipped (see below). |
| `0x0008` | Explicit per-record timestamps (the 24-byte layout above). |

The desktop rejects a batch, and counts it as an invalid frame, in three cases:

- `timestamps_us[0]` does not equal `first_timestamp_us`.
- The timestamps are not strictly increasing.
- `field_count` does not match the configured lab.

It ignores batches that arrive before the START STATUS, because they belong to an earlier run.

Protocol 0.1–0.3 batches do not set `0x0008`. They use the legacy 20-byte header: the same fields without the overflow counter, and no timestamp array. The desktop expands their record timestamps from the first timestamp and the nominal period. The parser keeps that path for historical data. New recordings require the current reference firmware, so their records carry measured timestamps.

### ERROR_MESSAGE codes

The payload is a one-byte code. The controlled reference firmware sends only these codes:

| Code | Cause | Effect |
| --- | --- | --- |
| 1 | Command header has an unsupported protocol version, or its length does not match | Acquisition stops; configuration cleared; D4/D5/D6 LOW |
| 2 | Command CRC mismatch | Acquisition stops; configuration cleared; D4/D5/D6 LOW |
| 3 | CONFIGURE rejected (mode, channels, pins, ADC bits, rate, dwell, or outputs) | Any acquisition stops; configuration cleared; D4/D5/D6 LOW; no CONFIG_ACK |
| 4 | START without an accepted configuration, or START with a payload | START refused; acquisition never began |
| 5 | Command input buffer overflow | Acquisition stops; configuration cleared; D4/D5/D6 LOW |
| 6 | Command payload length above the protocol maximum | Acquisition stops; configuration cleared; D4/D5/D6 LOW |
| 7 | Host-command watchdog: no valid command for 5 s while acquiring | Acquisition stops after the final partial batch; D4/D5/D6 LOW |

The desktop fails a pending CONFIGURE or START wait as soon as a new ERROR arrives, and explains each code with its likely cause and next step. An ERROR received before the handshake finished is logged but not counted against the new recording. Any other code is reported as not used by the reference firmware.

## Handshake, start, and stop

The desktop follows the same sequence for Start, Verify Firmware, and Retry connection. Verify and Retry stop after the handshake and never configure or start the board; Verify never uploads firmware.

1. **Handshake.**
   1. The desktop opens the port and waits 1.25 s.
   2. It sends PING up to three times, 2 s apart, within an overall 8 s limit. The board answers every PING with HELLO, CAPABILITIES, and PONG. It also sends HELLO and CAPABILITIES once at power-up.
   3. The handshake needs HELLO, a parseable CAPABILITIES, and PONG, all with valid CRCs, and an exact identity match: build `0x00010006` and device ID `0x554E4F34`.
2. **Quiesce.** Before every CONFIGURE, the desktop sends an idle-safe **STOP**. A board still acquiring for an earlier session that ended without STOP therefore stops, drives its outputs LOW, and never applies a new configuration mid-acquisition: the firmware does not stop acquiring on CONFIGURE by itself. The `acquiring = 0` STATUS that answers this STOP does not satisfy the later START wait.
3. **CONFIGURE.** The desktop sends CONFIGURE and waits up to 2 s for a new CONFIG_ACK. An accepted CONFIGURE drives the outputs LOW.
4. **START.**
   1. The desktop creates the `.bmeg.part` file, then sends START.
   2. It waits up to 2 s for a STATUS with `acquiring = 1`. Recording starts with that STATUS.
   3. START resets the firmware's record sequence (to 0), the overflow counter, and the per-phase counters. In simultaneous mode it applies the output mask and samples the first frame at once. In pulse-ox mode it applies the RED phase and samples it one dwell later.
5. **Keepalive.** While acquiring, the firmware stops with ERROR 7 if no valid command arrives for 5 s; any frame that passes the version and CRC checks counts. The desktop sends PING every second during a recording. If no sample arrives for 7 s, the desktop ends the recording as a no-data fault instead of restarting it.
6. **STOP.**
   1. The desktop sends STOP. The firmware closes the partial batch, stops, drives D4/D5/D6 LOW, and sends STATUS (`acquiring = 0`) after all queued batches.
   2. The desktop reads for up to 300 ms until a STATUS newer than STOP reports `acquiring = 0` and output mask 0. Every batch received before it, including the recording's tail, is saved.

If START may have reached the board but its STATUS did not arrive, the desktop sends a best-effort STOP. It removes the `.bmeg.part` file if that file holds no records.

## Sampling and timestamps

- **Record sequence.** It counts records (frames, or pulse-ox cycles) from 0 at START.
- **Timestamps.** They are 64-bit microseconds from the firmware's `micros()`, extended across its 32-bit wrap.
  - In simultaneous mode, a record's timestamp is taken immediately before the frame's first analog read, so it marks the **start** of the frame. The channels are then read one after another in CONFIGURE order.
  - In pulse-ox mode, the cycle timestamp is taken after the DARK 2 reads, so it marks the **completion** of the cycle.
- **Nominal period.** It describes the requested configuration and is never substituted for measured timestamps.

The firmware keeps sampling separate from packet work and USB:

- **Raw-batch queue.** Completed batches go into a bounded queue of 4 slots. One slot is being filled, so at most 3 batches wait.
- **Encoding.** Packets and CRCs are built outside the sampling deadline check.
- **Transmit queue.** It holds 8 frames, and 3 are reserved for control frames. Each loop writes at most 64 bytes, and only as much as the port reports as writable.

If a batch completes while 3 batches are already waiting, the firmware drops that batch, increments `acquisition_queue_overflows`, and sets flag `0x0004` on the next stored batch. The dropped records leave a gap in the record sequence.

When the loop finds that one or more sampling deadlines have already passed, it skips them (advancing the deadline on the same absolute timeline) and also sets flag `0x0004` on the next stored batch. A skipped deadline produces no record and no sequence gap, and it does not change `acquisition_queue_overflows`. Flag `0x0004` therefore has two meanings.

The desktop accounts for this as follows:

- It counts the records of flagged batches separately, as `integrity.overrun_flagged_records`, in the session summary and metadata. That count is never added to `firmware_overflows`, which counts each dropped batch once.
- It accounts for the cumulative overflow value once, even when the value appears in both STATUS and batch frames.
- Every record of a flagged batch keeps the flag in its BMEG status field.
- A skipped interval appears in the ±5% timing-tolerance count.

## Safety and compatibility

The firmware forces D4, D5, and D6 LOW in these cases:

- In `setup()`
- On every loop iteration while not acquiring
- On STOP
- On an accepted CONFIGURE
- Before START applies the new outputs
- On every rejected command (ERROR 1, 2, 3, 5, or 6)
- On the 5 s watchdog (ERROR 7)

RED and IR are never HIGH together. Before `setup()` runs (power-up, reset, the bootloader, and a firmware upload), the sketch does not drive these pins.

Acquisition requires HELLO, CAPABILITIES, PONG, valid CRCs, the expected firmware identity, an accepted CONFIGURE, and a confirmed START.

The recording reader remains compatible with earlier single-channel and synchronized BMEG layouts, and existing files are never relabelled or rewritten. New metadata records:

- Requested and measured rate
- Interval median, minimum, maximum, and p99
- The ±5% timing-tolerance count
- Timestamp source
- Acquisition-queue overflows
- `overrun_flagged_records`
- For pulse ox, the four per-state timing checks

[Calibration and Units](CALIBRATION_AND_UNITS.md#recording-files) describes the BMEG, CSV, and metadata files.
