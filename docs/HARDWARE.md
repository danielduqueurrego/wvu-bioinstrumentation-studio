# Hardware

## Supported board

The application supports one Arduino UNO R4 WiFi at a time, running the repository-controlled WVU reference firmware over the RA4M1's native USB. **Restore WVU Firmware** is refused while more than one UNO R4 WiFi is connected.

## Analog inputs

Simultaneous-analog course labs can use A0–A5 (one to six unique inputs) at 12- or 14-bit resolution and 100, 200, 250, 500, or 1000 frames/s. Each logical frame has one sequence number and one firmware-measured timestamp. The timestamp is taken immediately before the frame's first analog read, so it marks the **start** of the frame. The UNO ADC then reads the channels one after another in the lab's channel order; the channels are not sampled electrically simultaneously.

## Controlled outputs

| Pin | Course use | Active behavior in the course labs |
| --- | --- | --- |
| D4 | Blood pressure + PPG green LED | HIGH while recording only |
| D5 | Pulse-ox red LED | HIGH only during the RED phase |
| D6 | Pulse-ox IR LED | HIGH only during the IR phase |

All three outputs are active HIGH. The firmware drives them LOW in these cases:

- At startup
- On every loop iteration while idle
- On Stop
- On an accepted or rejected configuration
- On a malformed command: a bad version, length, or CRC, or an input overflow
- On the host-command watchdog, which fires 5 s without a host command while recording

Before the sketch's `setup()` runs (power-up, reset, the bootloader, and a firmware upload), the firmware does not drive D4–D6.

**RED and IR are never HIGH together**, and the firmware itself enforces that rule:

- In pulse-ox mode, every phase change first drives all three outputs LOW and then sets at most one of the RED or IR pins HIGH.
- In simultaneous mode, the firmware rejects a configuration that would hold D5 and D6 HIGH together.

Simultaneous-mode configurations that hold D5 alone or D6 alone HIGH while recording are refused by the desktop's lab validation, which allows HIGH-while-recording only on D4; the current firmware would accept such a configuration if another program sent it. In pulse-ox labs the instructor may assign RED and IR to any two different pins of D4–D6; the factory lab uses D5 (red) and D6 (IR).

## Pulse oximetry

The fixed cycle is RED, DARK 1, IR, DARK 2. Each state lasts one dwell (250–5000 µs; the factory lab uses 1000 µs, about 250 cycles/s). State deadlines advance on one absolute schedule (`next deadline += dwell`), so ADC reads, bookkeeping, packet construction, and USB service do not add up to a periodic cycle penalty.

Each cycle stores the raw TX and RX values for all four states and one timestamp taken after the DARK 2 reads, marking the cycle's **completion**. The application generates no processed optical result: it does not subtract ambient light or calculate SpO2 or heart rate. Bounded raw-batch and transmit queues keep encoding and USB backpressure outside the timing-critical sampling path. Overflow and skipped deadlines are reported rather than hidden (see [USB Protocol](USB_PROTOCOL.md#sampling-and-timestamps)).

Teaching use only — not a medical device.
