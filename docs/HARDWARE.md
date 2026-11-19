# Hardware

## Supported board

The application supports one Arduino UNO R4 WiFi at a time, running the repository-controlled WVU reference firmware (build `0x00010007`) over the RA4M1's native USB. **Restore WVU Firmware** is refused while more than one UNO R4 WiFi is connected.

## Analog inputs

Simultaneous-analog course labs can use A0–A5 (one to six unique inputs) at 12- or 14-bit resolution and 100, 200, 250, 500, or 1000 frames/s. Each logical frame has one sequence number and one firmware-measured timestamp. The timestamp is taken immediately before the frame's first analog read, so it marks the **start** of the frame. The UNO ADC then reads the channels one after another in the lab's channel order; the channels are not sampled electrically simultaneously.

**Read order and timing.** The firmware reads one pin per `analogRead` call: each call selects that pin in the ADC's scan mask, converts it, and returns before the next pin is selected. So in every mode with a channel list, channel *k* of the lab (counting the first as 0) is read roughly *k* × 20 µs after the record timestamp. The 20 µs per read is an estimate for the RA4M1 at these settings, not a measured specification; measuring it is on the hardware checklist. Treat the channels of one record as a few tens of microseconds apart, not as simultaneous.

## Controlled outputs

| Pin | Course use | Active behavior in the course labs |
| --- | --- | --- |
| D4 | Blood pressure + PPG green LED | HIGH while recording only |
| D5 | Pulse-ox red LED | HIGH only during the RED state |
| D6 | Pulse-ox IR LED | HIGH only during the IR state |

All three outputs are active HIGH. The firmware drives them LOW in these cases:

- At startup
- On every loop iteration while idle
- On Stop
- On an accepted or rejected configuration, and on a configuration that arrives during a recording (which stops the recording first)
- On a malformed command: a bad version, length, or CRC, or an input overflow
- On the host-command watchdog, which fires 5 s without a host command while recording

Before the sketch's `setup()` runs (power-up, reset, the bootloader, and a firmware upload), the firmware does not drive D4–D6.

**Hardware watchdog.** The firmware also runs the RA4M1's hardware watchdog, started at the end of `setup()` and refreshed at the start of every loop pass. The sketch asks for 1000 ms; the core's `WDT` library rounds that up to the next hardware timeout, which on the UNO R4 (PCLKB 24 MHz, divide-by-2048, 16,384 counts) is about 1.4 s (1,398 ms). If the firmware goes about 1.4 s without a loop pass, the board resets. During the reset the pins are inputs (not driven HIGH), and `setup()` then drives them LOW again. The board's native USB port disappears during the reset, so the desktop ends the recording within about a second, as a disconnection (or as a serial-connection fault if the board has already reappeared). The seven-second no-data safeguard applies only if the port stays open but silent. The raw recording is finalized either way.

**RED and IR are never HIGH together**, and the firmware itself enforces that rule:

- In both pulse-ox modes, every state change first drives all three outputs LOW and then sets at most one of the RED or IR pins HIGH.
- In simultaneous mode, the firmware refuses any configuration that would hold D5 or D6 HIGH; only D4 can be HIGH while recording. The desktop's lab validation applies the same rule. (Firmware build `0x00010006` refused only D5 and D6 together.)

In pulse-ox labs the instructor may assign RED and IR to any two different pins of D4–D6; the built-in lab uses D5 (red) and D6 (IR).

## Pulse oximetry

The LED sequence is RED, DARK 1, IR, DARK 2. Each state lasts one dwell. State deadlines advance on one absolute schedule (`next deadline += dwell`), so ADC reads, bookkeeping, packet construction, and USB service do not add up to a periodic cycle penalty. The firmware has two pulse-ox modes:

| | LED-sequenced (built-in lab 1.1.0) | Legacy 4-state (built-in lab 1.0.0 and older local labs) |
| --- | --- | --- |
| Inputs | 1–6 unique pins of A0–A5 | Two pins: TX and RX |
| Dwell | 1000–5000 µs (built-in lab: 1000 µs) | 250–5000 µs (lab 1.0.0: 1000 µs) |
| Record | One per LED state: the LED state, then every input | One per cycle: the 8 raw TX and RX values of the four states |
| Rate | `round(1,000,000 / dwell)` records/s (1000 at 1000 µs); each LED state at a quarter of that | `round(1,000,000 / (4 × dwell))` cycles/s (250 at 1000 µs) |
| Timestamp | Just before the state's first read (the **start** of the reads) | After the DARK 2 reads (the **completion** of the cycle) |

**LED-sequenced timing.** At the end of each state's dwell the board timestamps the record and reads every input in the lab's channel order while that state's LEDs are unchanged, so input *k* is read roughly *k* × 20 µs after the timestamp (see [Read order and timing](#analog-inputs)). With six inputs the reads take on the order of 0.1 ms, well inside the 1 ms minimum dwell. Only then does the firmware switch to the next state. Every input therefore sees the same LED state, and a photodetector on A0 can share a record with other signals on A1–A5.

**Minimum settle.** If the loop runs late, the next deadline could fall almost immediately after an LED change. The firmware then lengthens the state that just started by whole dwells until its sample is at least half a dwell after its LED edge. States are never shortened below that, skipped, or reordered. The skipped deadlines are counted and flagged, and the shortest settle the board measured is reported at Stop (`minimum_pulse_settle_us` in the recording metadata).

**USB timing limit (firmware build `0x00010007`).** While the computer collects a USB packet, the board's USB driver can hold up the sampling loop, so the first record after a batch of ten is sometimes read late. The next record is on schedule again, because the deadlines are absolute. Late records keep their measured timestamp and their LED state, and the minimum-settle guard still applies, so no data are lost or mislabeled. Measured on hardware (60 s recordings, firmware build `0x00010007`):

| Lab | Intervals outside ±5% | Late records | Skipped deadlines | Minimum settle |
| --- | --- | --- | --- | --- |
| Built-in pulse ox: 2 channels, 1000 µs | 70-91 of 60,000 | about one per second, up to 0.26 ms late (once 0.5 ms in a shorter run) | 0 (once 1) | 677-683 µs |
| 4 channels, 1000 µs | 7,474 of 60,000 | at most batch boundaries, up to 0.62 ms late | 5 | 525 µs |
| 6 channels, 2000 µs | 8 of 30,000 | 4 in 60 s, up to 0.26 ms late | 0 | 1560 µs |

More channels make each batch longer, so it needs more USB packets. **For LED-sequenced labs with more than two channels, use a dwell of at least 2000 µs.** Simultaneous labs at 1000 frames/s show the same isolated late records.

The application generates no processed optical result: it does not subtract ambient light or calculate SpO2 or heart rate. Bounded raw-batch and transmit queues keep encoding and USB backpressure outside the timing-critical sampling path. Overflow and skipped deadlines are reported rather than hidden (see [USB Protocol](USB_PROTOCOL.md#sampling-and-timestamps)).

Teaching use only — not a medical device.
