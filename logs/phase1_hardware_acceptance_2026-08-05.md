# Phase 1 hardware acceptance — 2026-08-05

## Safety and controller path

No person, electrode, optical probe, biomedical board, cuff, or bench instrument was
connected. This was an Arduino-alone, raw floating-A0 communication test. D4, D5, and
D6 remain LOW in the uploaded Phase 1 firmware.

The feature-gated acceptance harness invoked the same `start_serial` worker and
status-polling controller used by Tauri commands; it is not the prior PowerShell
observer. It ran A0, 12-bit, 1000 samples/s for 61 host seconds.

## Identity and build

| Field | Measured value |
|---|---|
| Board | Arduino UNO R4 WiFi |
| Port / serial | COM12 / 48CA4360243C |
| FQBN | `arduino:renesas_uno:unor4wifi` |
| UNO R4 core | 1.6.0 |
| Arduino CLI | 1.5.2-rc.1 |
| Firmware / protocol | `0x00010000` / 0.1 |
| Compile/upload | Passed; 53,508 bytes flash, 7,940 bytes RAM, COM12 upload |

The host PING verified CRC-valid HELLO, CAPABILITIES, and PONG before configuration.

## Result: passed

| Metric | Value |
|---|---:|
| Host duration | 61.058294 s |
| Board timestamp duration | 60.849 s |
| Validated samples | 60,850 |
| Requested / measured rate | 1000 / 1000.000 Hz |
| Board-rate error | 0.000% |
| Valid packets | 6,270 |
| CRC failures / invalid frames | 0 / 0 |
| Missing packets / samples | 0 / 0 |
| Duplicate / out-of-order packets | 0 / 0 |
| Firmware / host overflows | 0 / 0 |
| Reconnect / disconnect events | 0 / 0 |
| Longest uninterrupted interval | 60.849 s |
| Representative host working set | 8,658,944 bytes (8.26 MiB at 10 s) |
| CPU observation | 0.15625 CPU s at 10 s (~1.6% of one core) |

The packet count includes sample batches plus periodic PING responses; each response
contains HELLO, CAPABILITIES, and PONG. The recorded sample sequences are contiguous
from 2,997 to 63,846 and timestamp values are strictly monotonic.

Outputs (ignored as generated recordings):

- `recordings/20260805_154043_Phase1_A0_Run01.bmeg` — 853,063 bytes
- `recordings/20260805_154043_Phase1_A0_Run01.metadata.json` — 1,455 bytes
- `recordings/20260805_154043_Phase1_A0_Run01.csv` — 2,599,636 bytes

The release app itself was also launch-checked: `wvu_bioinstrumentation_studio.exe`
was running after five seconds. Visual plot responsiveness was not manually observed
during this headless controller acceptance run; the UI polls a bounded snapshot at
25 Hz and the bounded-display automated test passed.
