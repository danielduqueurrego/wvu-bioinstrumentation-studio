# Phase 1.1 indefinite-recording acceptance — 2026-08-06

## Simulator results

The following production-controller command was run. `until` maps to the explicit
`RecordingDuration::UntilStopped` request; after 20 seconds the harness issued the same
Rust `request_stop()` path as the Stop recording control.

```powershell
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase1_capture -- simulator until 20
```

Result: passed.

| Metric | Value |
|---|---:|
| Host duration | 20.019962 s |
| Board timestamp duration | 19.999 s |
| Validated samples / CSV rows | 20,000 / 20,000 |
| Valid packets | 2,024 |
| Measured rate | 1000.000 Hz |
| CRC / invalid frames | 0 / 0 |
| Missing packet / sample sequences | 0 / 0 |
| Duplicate / out-of-order packets | 0 / 0 |
| Firmware / host buffer overflows | 0 / 0 |
| Duration mode / stop reason / completion | until_stopped / user / complete |
| Initial / final observed free disk | 477,584,879,616 / 477,592,121,344 bytes |

Read-back validation confirmed contiguous sequences 0 through 19,999, timestamp range
0 through 19,999,000 microseconds, matching metadata counts, and BMEG/CSV/metadata sizes
of 281,231 / 844,620 / 1,562 bytes. The recording files are intentionally Git-ignored:
`recordings/20260806_094520_Phase1_A0_Run01.{bmeg,csv,metadata.json}`.

An accelerated deterministic simulator soak also passed through the protocol parser,
bounded recent history, and streaming BMEG writer:

```powershell
cargo test --manifest-path src-tauri\Cargo.toml accelerated_fifteen_minute_equivalent_simulator_soak_preserves_bounds -- --nocapture
```

It wrote and validated 900,000 samples (the 15-minute equivalent at 1000 samples/s),
with a fixed 1,500-sample display history, zero CRC/loss/host-overflow counters, and no
complete-recording collection.

## Hardware result

Passed through the production Rust `SessionController` used by the Tauri commands. The UNO R4
WiFi alone was rediscovered as COM12 (USB serial `48CA4360243C`, FQBN
`arduino:renesas_uno:unor4wifi`, Renesas core 1.6.0). A0 was left floating and is explicitly an
uncalibrated engineering communication signal; no person or biomedical accessory was connected.

The harness sent the explicit `UntilStopped` request, remained active for 125 seconds, and invoked
the same `request_stop()` finalization path as Stop recording. The recording crossed the required
two-minute threshold before manual stop; no hidden duration limit fired.

| Metric | Value |
|---|---:|
| Firmware identity / protocol | `0x00010001` / `0x554E4F34` / v0.1 |
| Host elapsed time | 121.494971 s |
| Board timestamp duration | 121.119 s |
| Validated samples / CSV rows | 121,120 / 121,120 |
| Valid packets | 12,480 |
| Requested / measured rate | 1000 / 1000.000 Hz |
| Sample-rate error | 0.000% |
| CRC / invalid / unsupported frames | 0 / 0 / 0 |
| Missing packet / sample sequences | 0 / 0 |
| Duplicate / out-of-order packets | 0 / 0 |
| Firmware / host buffer overflows | 0 / 0 |
| Disconnects / reconnects | 0 / 0 |
| Duration mode / stop reason / completion | until_stopped / user / complete |
| Initial / final observed free disk | 477,523,996,672 / 477,512,380,416 bytes |

The CSV, BMEG, and metadata validation is recorded separately in
`phase1_1_export_validation_2026-08-06.md`. The release executable launched and closed cleanly
after this run (about 31 MiB working set at launch). Interactive resize/plot observation remains
separately pending because this execution environment cannot observe the Tauri desktop surface.
