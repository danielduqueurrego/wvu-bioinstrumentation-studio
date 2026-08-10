# Phase 4 Blood Pressure + PPG course-capture smoke acceptance — 2026-08-07

UNO R4 WiFi alone on COM12; A0–A2 were floating/uncalibrated. No pressure hardware, person, or
bench source was connected.

| Item | Result |
| --- | --- |
| Profile | `wvu.bmeg420l.blood_pressure.ppg.course.capture.v1` |
| Synchronized mapping | A0 PPG; A1 MPXV/reference pressure; A2 XGZP/instrumented pressure |
| Format | 12 bit; 200 logical frames/s; three fields in every record |
| Host / board duration | 30.030 s / 29.895 s |
| Frames / packets | 5,980 / 690 |
| Measured rate | 200.001 Hz |
| Integrity | CRC, invalid, missing, duplicate, out-of-order, firmware overflow, host overflow, disconnect, reconnect: all 0 |
| D4 safety | v0.2 STATUS confirmed D4 active while acquiring (`mask=1`) and D4/D5/D6 LOW after Stop (`mask=0`) in the follow-up 10-second production capture |
| Export | BMEG 123,160 B; CSV 172,368 B; metadata 4,934 B; header and three-field record/count validation passed |

Ignored temporary capture roots:
`C:\Users\dd00055\AppData\Local\Temp\2\wvu_phase4_course_blood_pressure_1786141043908`
and `C:\Users\dd00055\AppData\Local\Temp\2\wvu_phase4_course_blood_pressure_1786141334302`.
