# Phase 6 lab authoring acceptance — 2026-08-10

Status: **automated implementation verification passed; simulator, UNO smoke, and manual UI acceptance pending.**

## Implemented workflow

- Instructor-only Manage Labs modal within Acquisition; no new primary navigation route.
- Active/history list with immutable revision records, edit, duplicate, blank simultaneous template,
  import/export, retire/restore, and course-default restore.
- Save creates the next patch revision, SHA-256-locks it, activates it for new Student sessions,
  and leaves completed recording snapshots unchanged.
- Simultaneous editor supports one to six unique A0–A5 channels, labels, CSV fields, conversion
  capability, default visibility, plot group, supported 100/200/250/500/1000 Hz rate, 12/14-bit
  ADC, and safe D4–D6 output declaration.
- The firmware/host protocol is v0.3 / build `0x00010003`; CAPABILITIES carries ADC, channel,
  mode, output, and rate limits. A saved offline lab is checked against those limits before
  CONFIGURE.

## Automated evidence — 2026-08-10

- Rust unit tests cover revision activation/persistence, drafts, blank template, resource conflicts,
  built-in retirement/restore, pulse resource checks, capability validation, and dynamic payloads.
- Frontend unit tests cover pulse fixed-phase helpers and client-side resource conflict reporting.
- `cargo fmt --check`, `cargo check`, `cargo test`, and `cargo clippy -- -D warnings` passed;
  Rust totals: **76 passed, 0 failed**.
- `npm run check`, `npm test`, and `npm run build` passed; frontend totals: **42 passed, 0 failed**.
- `npm run tauri build` produced MSI and NSIS packages. The controlled v0.3 reference sketch compiled
  with Arduino CLI: **54,748 bytes (20%) flash; 9,060 bytes (27%) RAM**.

## Pending acceptance

- Explicit controlled-reference firmware upload/identity probe at protocol 0.3 / build
  `0x00010003` (no automatic upload is permitted).
- Simulator and UNO smoke captures of instructor-edited ECG, multi-channel, D4, and pulse-ox labs.
- Full manual Lab Manager/Firmware/Acquisition viewport matrix at actual Windows scaling.
- No Phase 6 source acceptance claim is made until the explicit user-assisted simulator/UNO and UI
  checks below are completed.
