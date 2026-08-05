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
- Normal recording duration: up to 10 minutes
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
