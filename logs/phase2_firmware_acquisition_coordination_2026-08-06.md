# Phase 2 firmware/acquisition coordination — 2026-08-06

## Preconditions

The Phase 2 controller sequence had just restored and independently verified the controlled
reference firmware on the UNO R4 WiFi. The board was alone; A0 was floating and explicitly
treated as uncalibrated raw engineering data.

## 30-second result

| Field | Measured value |
|---|---:|
| Port / serial | COM12 / `48CA4360243C` |
| Configuration | A0, 12-bit, 1000 samples/s |
| Host duration | 30.0364743 s |
| Board duration | 29.929 s |
| Valid samples | 29,930 |
| Valid packets | 3,085 |
| Measured rate | 1000.000 Hz |
| CRC / invalid frames | 0 / 0 |
| Missing packet/sample sequences | 0 / 0 |
| Duplicate/out-of-order packets | 0 / 0 |
| Firmware/host overflow | 0 / 0 |
| Disconnects/reconnects | 0 / 0 |

## Files and streaming validation

The real recordings are intentionally outside the repository:

- `C:\Users\dd00055\AppData\Local\Temp\2\wvu_phase2_acceptance_1786036462343\recordings\20260806_131501_Phase1_A0_Run01.bmeg` — 420,355 bytes
- `C:\Users\dd00055\AppData\Local\Temp\2\wvu_phase2_acceptance_1786036462343\recordings\20260806_131501_Phase1_A0_Run01.metadata.json` — 1,670 bytes
- `C:\Users\dd00055\AppData\Local\Temp\2\wvu_phase2_acceptance_1786036462343\recordings\20260806_131501_Phase1_A0_Run01.csv` — 1,262,284 bytes

The `phase2_firmware_capture -- validate` streaming read-back passed: BMEG, CSV, and metadata
each report 29,930 samples; sequences and timestamps are monotonic/contiguous; first/last board
timestamps are 6,317,362 / 36,246,362 us; the timestamp-derived rate is 1000.000 Hz; every CSV
voltage value matches `counts * 5.0 / 4095.0`; metadata reports `timed`, 30 seconds,
`timed_complete`, and `complete`; all tested integrity counters are zero.

## Result

Passed. The non-WVU upload disabled Acquisition, and only the independently verified controlled
reference re-enabled it before the successful production-session recording.
