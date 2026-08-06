# Phase 3B validation framework acceptance — 2026-08-06

## Scope and result

**Phase 3B implementation complete. Simulator workflow accepted. Physical ECG and EMG validation
pending. Manual Validation-page viewport/scaling verification pending. Overall Phase 3B
acceptance: pending.** This work is strictly bench-validation engineering. No person, electrode
system, ECG/EMG module connected to a person, or biomedical accessory was used. The simulator
acceptance did not open a serial port.

The app now has a versioned validation-evidence model with draft, finalized, and retired states;
canonical SHA-256 integrity; profile ID/version/hash and firmware build/device matching; raw-data
retention; instructor-defined criteria; and manifest-hashed package import/export. SHA-256 detects
changes; it is not authorship authentication or human-use authorization.

## Production-path simulator evidence

Command:

```powershell
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase3b_validation_capture -- "$env:TEMP\wvu_phase3b_simulator_acceptance_20260806_final2"
```

Result: passed. Validation ID `wvu.bmeg420l.ecg.interface.validation.simulator.001`; finalized
evidence SHA-256
`47e0a006d308861f978a6483089445438386194a84ddda187a7c2675b5f68659`.

Seven separate 10-second sessions exercised the same parser, integrity counters, bounded display,
raw BMEG writer, metadata writer, CSV exporter, metric calculation, evidence finalization, and
package import path used by the application:

| Test | Runs | Samples/run | Packets/run | Measured rate | Integrity result |
| --- | ---: | ---: | ---: | ---: | --- |
| Zero-input/baseline | 1 | 10,000 | 1,014 | 1000.000 Hz | all counters zero |
| DC operating-range sweep (2.5 V) | 1 | 10,000 | 1,014 | 1000.000 Hz | all counters zero |
| Known sine (50 Hz, 1.0 Vpp) | 1 | 10,000 | 1,014 | 1000.000 Hz | all counters zero |
| Saturation-margin exercise | 1 | 10,000 | 1,014 | 1000.000 Hz | all counters zero |
| Repeatability | 3 | 10,000 each | 1,014 each | 1000.000 Hz | all counters zero |

Across every run: CRC failures, invalid frames, missing/duplicate/out-of-order packet and sample
sequences, firmware/host overflows, disconnects, and reconnects were all zero.

Measured simulator metrics were transparent and retained separately from raw samples: DC mean
2.500611 V with 0.000611 V absolute error; sine 50.000 Hz, 1.000000 Vpp, 0.000 Hz frequency
error; saturation exercise 100.000% rail clipping; and identical 2.500611 V repeatability means.
All seven local, explicitly configured criteria passed. These are simulator assertions, not module
performance claims.

## Simulator versus physical status

A finalized simulator evidence record deliberately leaves the profile status **Unvalidated** with
the explanation: “Finalized simulator evidence is available, but it does not establish physical
bench validation.” This regression is covered by Rust test
`finalized_simulator_evidence_does_not_claim_physical_bench_validation`.

Physical ECG and EMG interface evidence remains required before a matching profile may appear
Bench validated. It needs an instructor, module identity, safe 0–5 V source/module output,
equipment metadata, and explicit local criteria. No human-connected recording is authorized.

## Automated verification

The following completed after the Phase 3B implementation:

- `cargo fmt --manifest-path src-tauri\Cargo.toml -- --check`
- `cargo check --manifest-path src-tauri\Cargo.toml`
- `cargo test --manifest-path src-tauri\Cargo.toml` — **58 passed**
- `cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets -- -D warnings`
- `npm run check` — 0 errors, 0 warnings
- `npm test` — **21 passed** across 8 files
- `npm run build` — passed (one existing bundle-size warning only)
- `npm run tauri build` — passed; MSI and NSIS artifacts produced but remain ignored

## Artifacts

Simulator artifacts are temporary and intentionally not committed:

`C:\Users\dd00055\AppData\Local\Temp\2\wvu_phase3b_simulator_acceptance_20260806_final2`

It contains seven BMEG/CSV/metadata triplets (BMEG 143,127–143,183 bytes; CSV
1,607,957–1,747,957 bytes; metadata 4,169–4,236 bytes) and an imported package.

## Deferred acceptance items

- Separate physical ECG and EMG bench-interface test sets.
- Instructor-driven criteria and profile association using actual matching hardware evidence.
- Manual Validation-page viewport/scaling/accessibility matrix.
