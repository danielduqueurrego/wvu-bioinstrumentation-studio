# Codex start prompt — Phase 1 vertical slice

You are working in the root of a new project for **WVU Bioinstrumentation Studio**, a Windows desktop application for BMEG 420L.

Read these files before changing anything:

- `README_FIRST.md`
- `docs/PROJECT_DECISIONS.md`
- `docs/SOFTWARE_REQUIREMENTS_SPECIFICATION_v1.0.md`
- `docs/ARCHITECTURE_AND_ROADMAP.md`
- `docs/HARDWARE_INTERFACE_SPECIFICATION.md`
- `docs/USB_PROTOCOL_SPECIFICATION_v0.1.md`
- `docs/ACCEPTANCE_TEST_PLAN.md`
- `docs/PRE_CODEX_CHECKLIST.md`
- all JSON files under `profiles/` and `schemas/`

The approved logo is under `assets/branding/`. Do not redraw, recolor, stretch, crop, or modify the logo artwork.

## Safety and scope

This is teaching and engineering equipment, not a medical device.

For this task:

- Do not perform any test on a person.
- Do not request electrodes, a finger pulse-ox test, or a cuff measurement.
- Use the connected Arduino UNO R4 WiFi alone, simulator data, or a safe bench signal on A0.
- Keep all pulse-ox LED pins low in any firmware created in Phase 1.
- Do not implement clinical SpO2, automatic blood pressure, heart rate, or EMG interpretation.

## Environment assumptions

- Windows 11
- Visual Studio Code
- Arduino CLI is already installed
- One Arduino UNO R4 WiFi is connected by USB
- The user has authorized compile/upload tests on that board
- Other required development tools may or may not be installed

First inspect the environment. Read `environment-report.txt` if present, then independently verify relevant versions and paths. Do not guess. If a required development dependency is missing, report the exact missing dependency and the smallest installation action needed. Do not silently install large system components without approval.

## Phase 1 goal

Create and test a minimal but production-shaped vertical slice that proves:

1. Windows desktop scaffold using Rust + Tauri 2 + Svelte + TypeScript.
2. uPlot-based bounded live plotting.
3. Arduino CLI board discovery, compile, and upload.
4. A safe single-file UNO R4 WiFi reference sketch.
5. The versioned binary USB protocol in `docs/USB_PROTOCOL_SPECIFICATION_v0.1.md`.
6. One analog channel sampled at 1000 samples/s from configurable A0–A5.
7. Explicit start and stop commands.
8. Rust packet parsing, CRC validation, sequence-gap detection, and integrity counters.
9. Batched frontend updates at approximately 20–30 Hz, never one UI event per ADC sample.
10. Continuous raw recording to a simple documented `.bmeg` file.
11. CSV and JSON metadata export.
12. A simulator that exercises the same host acquisition path without hardware.
13. Automated tests.
14. A measured Phase 1 report.

Do not implement the full sketch editor, pulse-ox sequence, MPXV calibration wizard, or offline installer in this task. Create clean extension points for those later phases.

## Required workflow

### 1. Inspect and plan

- Record Windows, Git, Arduino CLI, UNO R4 core, Rust/Cargo, Node/npm, and WebView2 status.
- Run `arduino-cli board list` and identify the connected UNO R4 WiFi COM port.
- Confirm the FQBN `arduino:renesas_uno:unor4wifi`.
- Inspect the starter documents and profiles.
- Write `PHASE_1_PLAN.md` with concrete steps, risks, and proposed repository layout before implementation.
- Use current stable project templates available in the installed toolchain, and pin resulting dependencies with lockfiles. Do not invent package versions.

### 2. Initialize repository

- Initialize Git if needed.
- Create a sensible `.gitignore`.
- Create `README.md` with reproducible commands.
- Preserve the starter documents in the repository.
- Use clear commits when practical, or at minimum provide a commit-ready change summary.

### 3. Scaffold desktop app

- Create the Tauri 2 + Svelte + TypeScript project.
- Use npm unless the scaffold requires and documents a different package manager.
- Apply a restrained WVU visual shell:
  - WVU Blue `#002855`
  - WVU Gold `#EEAA00`
  - Not Quite White `#F7F7F7`
- Use the approved logo without modification.
- Include a visible teaching/not-medical-device notice in About or Help.
- Add primary views/placeholders: Home, Firmware, Acquisition, Diagnostics.
- Keep Phase 1 UI simple and functional.

### 4. Implement Arduino CLI adapter

Implement a Rust adapter that:

- Finds the Arduino CLI executable.
- Runs version, board-list, compile, and upload commands.
- Captures exact command, exit code, stdout, stderr, and duration.
- Parses JSON where supported.
- Allows a configurable CLI path.
- Never shells untrusted user text directly into a command string; pass arguments separately.

### 5. Implement reference firmware

Create a single `.ino` sketch under a clear firmware template directory.

Requirements:

- UNO R4 WiFi only.
- All candidate pulse-ox LED pins initialized LOW and never driven HIGH in Phase 1.
- Protocol HELLO/capabilities.
- CONFIGURE for one analog channel and 1000 samples/s.
- START and STOP.
- Deterministic board-side sampling.
- 12-bit ADC.
- Sample batches with monotonic sample sequence and extended 64-bit microsecond timestamp.
- Packet sequence and CRC-16/CCITT-FALSE.
- Safe stop on command timeout or malformed configuration.
- No human measurement assumptions.
- Comments sufficient for future student-facing simplification.

Compile and upload it using Arduino CLI. Save logs.

### 6. Implement protocol and serial acquisition

In Rust:

- Implement incremental frame parsing.
- Resynchronize after noise/corruption.
- Reject invalid lengths and CRC.
- Track packet/sample integrity counters.
- Use a blocking serial reader thread or another simple justified approach.
- Keep acquisition independent from UI and disk writing.
- Support clean cancellation and reconnect.
- Do not crash if the port disappears.

### 7. Implement simulator

Create a simulator that generates the same protocol messages and a configurable waveform. It must use the same parser/acquisition path as hardware, not a separate UI-only shortcut.

### 8. Implement plotting

- Use uPlot.
- Keep a bounded time window.
- Batch updates at 20–30 Hz.
- Show current sample rate, connected device, acquisition state, and integrity counters.
- Provide counts and volts views using a clearly documented 0–5 V, 12-bit conversion.
- Do not apply hidden filtering.

### 9. Implement recording/export

Create a simple documented `.bmeg` format adequate for Phase 1.

Minimum outputs:

- Raw `.bmeg`
- `.metadata.json`
- `.csv`

Metadata must include:

- UTC/local start time
- Board and COM port
- FQBN
- Arduino CLI version
- UNO R4 core version
- Firmware build/protocol version
- Analog pin
- ADC bits
- Requested/measured sample rate
- Packet/sample integrity counters
- App version
- Simulator versus hardware

Write raw samples continuously. Do not accumulate the complete recording in RAM.

### 10. Test

Automated tests must cover:

- CRC known vectors.
- Valid/partial/back-to-back frames.
- Noise before magic.
- Invalid length.
- CRC failure.
- Sequence gap/duplicate/out-of-order detection.
- Profile loading.
- Filename sanitization.
- Simulator acquisition.
- Start/stop state transitions.
- Recording and CSV round trip.
- Parser must not panic on arbitrary byte sequences.

Hardware smoke test:

- Detect board.
- Compile.
- Upload.
- Read HELLO.
- Configure A0 at 1000 samples/s.
- Acquire at least 60 seconds.
- Stop cleanly.
- Save and export.
- Report requested/measured sample rate, sample count, packet count, missing samples, CRC failures, reconnects, app memory observation, and CPU observation if practical.

If A0 is floating, that is acceptable for communication testing; clearly label the signal as uncalibrated/floating. Do not claim physiological validity.

### 11. Report

Create `PHASE_1_REPORT.md` containing:

- Environment and exact versions.
- Repository structure.
- Commands run.
- Files created/changed.
- Tests and results.
- Hardware COM port and board identity.
- Firmware build/upload result.
- 60-second acquisition metrics.
- Known limitations.
- Deviations from the specification and why.
- Exact next recommended task.

Also create `KNOWN_ISSUES.md`.

## Quality rules

- Prefer simple, explicit code over clever abstractions.
- Use Rust error types and actionable user messages.
- No silent data loss.
- No unbounded queues.
- No sample-by-sample frontend events.
- No `unwrap()`/`expect()` in production paths unless impossibility is proven and documented.
- Format and lint Rust and frontend code.
- Keep protocol constants in one shared documented source location; add consistency tests where practical.
- Do not delete the starter requirements or replace them with a shorter interpretation.
- If hardware behavior conflicts with the documents, stop and report the measured conflict rather than hiding it.

Begin by inspecting the repository and environment, then write `PHASE_1_PLAN.md`.
