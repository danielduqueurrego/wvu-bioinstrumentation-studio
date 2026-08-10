# Phase 4 ECG course-capture smoke acceptance — 2026-08-07

UNO R4 WiFi alone on COM12; A0 was floating/uncalibrated. No module, person, electrode, or bench
source was connected.

| Item | Result |
| --- | --- |
| Profile | `wvu.bmeg420l.ecg.course.capture.v1` — ECG — Course Capture |
| Mapping | A0 = ECG; 12 bit; 1000 frames/s |
| Firmware | protocol 0.2; build `0x00010002`; device `0x554E4F34` |
| Host / board duration | 30.044 s / 29.929 s |
| Frames / packets | 29,930 / 3,085 |
| Measured rate | 1000.007 Hz |
| Integrity | CRC, invalid, missing, duplicate, out-of-order, firmware overflow, host overflow, disconnect, reconnect: all 0 |
| Export | BMEG 482,037 B; CSV 557,602 B; metadata 4,277 B; provenance/header/count validation passed |

Ignored temporary capture root:
`C:\Users\dd00055\AppData\Local\Temp\2\wvu_phase4_course_ecg_1786140952239`.
