# Phase 1 simulator acceptance — 2026-08-05

## Path exercised

`cargo run --manifest-path src-tauri\Cargo.toml --bin phase1_capture -- simulator 5`

The feature-gated acceptance harness calls the same nonblocking `start_simulator`,
status-polling, parser, integrity, recording, and CSV-export path registered by the
Tauri commands. It does not use the former frontend-only preview path.

## Result: passed

| Metric | Value |
|---|---:|
| Host duration | 5.014298 s |
| Board duration | 4.999 s |
| Validated samples | 5,000 |
| Valid packets | 509 |
| Measured rate | 1000.000 Hz |
| CRC / invalid frames | 0 / 0 |
| Missing / duplicate / out-of-order samples | 0 / 0 / 0 |
| Firmware / host overflows | 0 / 0 |
| Bounded recent display history | <= 1,500 samples (automated test) |

Outputs (ignored as generated recordings):

- `recordings/20260805_153510_Phase1_A0_Run01.bmeg`
- `recordings/20260805_153510_Phase1_A0_Run01.metadata.json`
- `recordings/20260805_153510_Phase1_A0_Run01.csv`

Read-back validation found `BMEGREC1`, 5,000 records and CSV rows, contiguous
sequences, strictly increasing timestamps, correct metadata (`simulator: true`,
`complete`), and zero CRC/missing/overflow counters. Repeated simulator and
mock-disconnect finalization are also covered by Rust tests.
