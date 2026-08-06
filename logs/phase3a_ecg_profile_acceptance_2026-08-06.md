# Phase 3A ECG locked-profile acceptance — 2026-08-06

## Safety

UNO R4 WiFi alone; A0 was floating and explicitly uncalibrated. No ECG module, electrode, or
person was connected. The selected `ECG Module — Raw Output` profile was acknowledged as
bench-validation only and provides raw counts/Arduino input volts only.

## Production-controller run: passed

`cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase3a_profile_capture -- hardware ecg 30`

| Metric | Result |
| --- | ---: |
| Board / port / serial | UNO R4 WiFi / COM12 / `48CA4360243C` |
| Firmware identity | protocol 0.1; build `0x00010001`; device `0x554E4F34` |
| Profile | `wvu.bmeg420l.ecg.raw.v1` 1.0.0; SHA-256 validated; acknowledgement true |
| Configuration | A0, 12 bit, 1000 samples/s |
| Host / board duration | 30.142 / 29.929 s |
| Samples / valid packets | 29,930 / 3,085 |
| Measured rate | 1000.000 Hz |
| CRC / invalid / missing packets / missing samples | 0 / 0 / 0 / 0 |
| Duplicate/out-of-order / firmware-host overflow / reconnect-disconnect | 0/0 / 0/0 / 0/0 |
| Completion / stop reason | complete / timed_complete |

Ignored outputs: `C:\Users\dd00055\AppData\Local\Temp\2\wvu_phase3a_ecg_1786039936711\20260806_141218_Phase3A_ecg_Run01.{bmeg,metadata.json,csv}`.
