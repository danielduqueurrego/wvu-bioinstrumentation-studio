# Project decisions

## Product identity

- Application name: **WVU Bioinstrumentation Studio**
- Subtitle: **Firmware, Acquisition, and Calibration for BMEG 420L**
- Executable target: `WVUBioinstrumentationStudio.exe`
- Repository name: `wvu-bioinstrumentation-studio`
- Application identifier: `edu.wvu.bioinstrumentationstudio`
- Primary recording extension: `.bmeg`

## Version 1 platform

- Arduino: UNO R4 WiFi only
- Arduino FQBN: `arduino:renesas_uno:unor4wifi`
- Desktop operating system: Windows 11 first
- Communication: USB only
- Concurrent boards: one
- UNO R4 WiFi recovery: normal handshake retry first; a 1200-bps touch reset is an explicit
  idle-only user action with USB re-enumeration matching and no implicit firmware upload.
- Controlled Phase 1 reference firmware identity: protocol v0.1, build `0x00010001`, and
  UNO R4 device ID `0x554E4F34`. A successful upload is not sufficient evidence; a
  production-parser HELLO/CAPABILITIES/PONG probe must verify these values.
- Recording duration: explicit Timed presets/custom duration or user-selected **Until stopped**.
  Until-stopped recording has no hidden time limit; it streams to disk, warns below 1 GiB
  free space, and finalizes under a 250 MiB controlled-stop guard.
- Internet: not required for normal operation after installation

## Technology stack

- Firmware: Arduino C++
- Desktop backend: Rust
- Desktop framework: Tauri 2
- Frontend: Svelte + TypeScript
- Plotting: uPlot
- Sketch editor: CodeMirror 6, limited to one `.ino` file
- Build/upload integration: Arduino CLI subprocesses with JSON output
- Package manager: npm unless the generated Tauri template requires another documented choice
- Tests: Rust unit/integration tests, frontend tests, protocol tests, simulator tests, and hardware smoke tests

## Functional scope

The application replaces:

- Firmware editing
- Firmware compilation
- Firmware upload
- USB acquisition
- Live visualization
- Calibration workflows
- Recording review
- CSV and JSON metadata export
- Diagnostic logging

Students may edit and compile a single-file Arduino sketch. The approved template must remain recoverable and distinguishable from a modified student copy.

## Lab profiles

Instructor-controlled profiles are required for:

1. ECG & EMG
2. Pulse Oximetry
3. Blood Pressure

Profile files are versioned JSON, validated against a schema, and cannot be silently changed by the student interface.

## Measurement policy

- Default output is raw ADC counts.
- Volts may be displayed as a direct conversion.
- MPXV5100DP may additionally display nominal or calibrated kPa/mmHg.
- XGZP160201S is counts/volts only in the application; student-built bridge conditioning and calibration are handled outside the app.
- No clinical SpO2 estimate.
- No automatic heart-rate, EMG-envelope, blood-pressure, or diagnostic interpretation in v1.

## Branding

- Use the approved WVU/college lockup without redrawing or recoloring it.
- Digital UI colors:
  - WVU Blue: `#002855`
  - WVU Gold: `#EEAA00`
  - Not Quite White: `#F7F7F7`
- The included approved SVG is the primary development asset.
