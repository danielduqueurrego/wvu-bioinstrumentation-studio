# Phase 3A profile framework acceptance — 2026-08-06

## Scope

Bench-only teaching/engineering work. No person, electrodes, ECG/EMG module, biomedical board,
optical hardware, or pressure hardware was connected. The UNO R4 WiFi reference continues to
force D4, D5, and D6 LOW.

## Result: accepted

- Embedded version-controlled locked profiles loaded and SHA-256-validated: General A0,
  ECG Module — Raw Output, and EMG Module — Raw Output.
- Schema validates ID/version/UNO/FQBN, controlled firmware identity, A0–A5, 12-bit/1000 Hz
  Phase 3A capabilities, duration settings, volts range, bench-only safety, export settings, and
  locked integrity.
- Student mode is default. Instructor mode requires the explicit local acknowledgement, is logged,
  and is documented as a workflow guard rather than authentication.
- Automated draft duplication, description edit, finalization to version 1.0.1, SHA-256 validation,
  and retirement passed. Built-in profiles cannot be overwritten or retired.
- A recording captures an immutable profile snapshot before session start; it cannot be affected by
  later selection/draft changes.
- Post-fix manual UI acceptance passed at 3440 × 1392 and Windows scaling 100%. The Student ↔
  Instructor radio state, badge, acknowledgement policy, lock display, acknowledgement gate,
  draft/finalize/hash/retire controls, and focused Acquisition layout all passed. Untested
  viewport/scaling rows remain explicitly pending in the manual verification log.
- Final verification: `cargo fmt --check`, `cargo check`, 49 Rust tests, Clippy with
  `-D warnings`, Svelte check, 17 frontend tests (including DOM checked-state coverage), frontend
  production build, and Tauri MSI/NSIS build all passed.

The exact schema and integrity/authentication distinction are in
`docs/ACQUISITION_PROFILE_SCHEMA_v1.md`.
