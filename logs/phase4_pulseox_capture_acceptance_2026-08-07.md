# Phase 4 pulse-ox TX + RX raw-capture smoke acceptance — 2026-08-07

UNO R4 WiFi alone on COM12; A0/A1 were unconnected. No optical probe, finger, person, or other
biomedical accessory was used.

| Item | Result |
| --- | --- |
| Profile | `wvu.bmeg420l.pulseox.txrx.raw.course.capture.v1` |
| Mapping | A0 TX TIA; A1 RX TIA; D5 RED; D6 IR |
| Cycle | RED, DARK 1, IR, DARK 2; 1000 us/state; 14 bit; target 250 cycles/s |
| Raw field order | `red_TX,dark1_TX,ir_TX,dark2_TX,red_RX,dark1_RX,ir_RX,dark2_RX` |
| Host / board duration | 30.045 s / 29.917 s |
| Cycles / packets | 7,480 / 840 |
| Measured cycle rate | 249.988 Hz |
| Integrity | CRC, invalid, missing, duplicate, out-of-order, firmware overflow, host overflow, disconnect, reconnect: all 0 |
| LED safety | Firmware state machine permits D5 only in RED, D6 only in IR, never both; v0.2 STOP STATUS in the follow-up production capture confirmed output mask 0 after Stop |
| Export | BMEG 227,676 B; CSV 410,369 B; metadata 4,401 B; eight-field header/count validation passed |

Ignored temporary capture roots:
`C:\Users\dd00055\AppData\Local\Temp\2\wvu_phase4_course_pulseox_1786141098702`
and `C:\Users\dd00055\AppData\Local\Temp\2\wvu_phase4_course_pulseox_1786141358837`.
