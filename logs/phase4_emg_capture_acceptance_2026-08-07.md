# Phase 4 EMG + Force course-capture smoke acceptance — 2026-08-07

UNO R4 WiFi alone on COM12; A0–A3 were floating/uncalibrated. No module, person, electrode, or
bench source was connected.

| Item | Result |
| --- | --- |
| Profile | `wvu.bmeg420l.emg.force.course.capture.v1` |
| Synchronized mapping | A0 raw EMG; A1 analog rectified EMG; A2 EMG envelope; A3 pressure/force surrogate |
| Format | 12 bit; 1000 logical frames/s; four fields in every record |
| Host / board duration | 30.043 s / 29.929 s |
| Frames / packets | 29,930 / 3,085 |
| Measured rate | 999.986 Hz |
| Integrity | CRC, invalid, missing, duplicate, out-of-order, firmware overflow, host overflow, disconnect, reconnect: all 0 |
| Export | BMEG 662,098 B; CSV 1,036,534 B; metadata 5,054 B; header and four-field record/count validation passed |

Ignored temporary capture root:
`C:\Users\dd00055\AppData\Local\Temp\2\wvu_phase4_course_emg_force_1786141000023`.
