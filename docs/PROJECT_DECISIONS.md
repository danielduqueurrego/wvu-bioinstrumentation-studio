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

## Phase 2 firmware workspace decisions

- A student firmware project contains exactly one Arduino source file and `project.json` in a
  matching project folder. Multi-file Arduino projects are deferred.
- Project source remains local to the student-selected folder. Controlled templates are
  version-controlled read-only sources; creating a project copies a template and never writes it.
- CodeMirror 6 is the embedded C++/Arduino editor. It performs no hidden source transformation.
- Compile and upload are separate deliberate operations. Upload needs a successful compile of the
  current saved source, an explicit confirmation, one discovered UNO R4 WiFi, and no active
  acquisition session.
- Arduino CLI upload/reset behavior is treated as a bounded re-enumeration workflow. Returning
  boards match by serial number first and never by an unrelated COM port; without a serial number,
  only the original port is accepted.
- A declared non-WVU student sketch may upload successfully but disables Acquisition. The
  controlled reference firmware is restored only through its explicit confirmation action and is
  protocol/identity verified before Acquisition becomes available.
- Compile/upload logs are application diagnostic data rather than student source files. They
  record project/source hashes, selected-board identifiers, CLI/core versions, exact argument
  arrays, stages, output, exit status, and verification evidence.

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

## Phase 3A ECG/EMG profile decisions

- Phase 3A includes locked General A0, ECG raw-output, and EMG raw-output packages. All bind A0,
  12-bit ADC, 1000 samples/s, direct 0–5 V Arduino input conversion, the controlled v0.1
  firmware identity, and timed/Until-stopped duration rules.
- SHA-256 covers deterministic canonical profile serialization excluding the hash field. It is
  integrity detection only—not a signature, authentication system, or human-use authorization.
- Student mode is default. Instructor authoring is a local acknowledged workflow guard; drafts
  must be finalized to a new locked version and cannot mutate built-ins.
- ECG/EMG profile selection, metadata, CSV, and BMEG provenance always say bench-only/not medical
  device/no human-connected recording authorized. No physiological quantity or interpretation is
  created by Phase 3A.

## Phase 3B bench-validation evidence decisions

- Validation evidence is a separate versioned JSON model, linked to a locked profile ID/version/
  canonical hash and exact controlled firmware build/device identity. It never mutates the
  profile snapshot embedded in a recording.
- Draft/finalized/retired evidence states are instructor-only workflow operations. Finalized
  evidence is immutable and SHA-256 integrity protected; the hash detects changes but is not an
  authorship signature, authentication mechanism, or authorization for human use.
- Every validation run uses the production session controller and is recorded as a separate raw
  BMEG/metadata/CSV session with a compact validation context. Metrics are computed from retained
  raw samples without hidden filtering.
- Bench validation remains strictly no-person/no-electrode/not-medical-device work even when an
  evidence record matches a profile. No physical module result is inferred from simulator evidence.

## Measurement policy

- Default output is raw ADC counts.
- Volts may be displayed as a direct conversion.
- MPXV5100DP may additionally display nominal or calibrated kPa/mmHg.
- XGZP160201S is counts/volts only in the application; student-built bridge conditioning and calibration are handled outside the app.
- No clinical SpO2 estimate.
- No automatic heart-rate, EMG-envelope, blood-pressure, or diagnostic interpretation in v1.

## Phase 4 multi-channel course-capture decisions

- Course development prioritizes synchronized raw variables, pin mapping, timing, and provenance;
  formal Phase 3B physical analog characterization remains optional and does not block normal lab
  capture. It remains a separate no-person engineering workflow.
- Protocol v0.2 retains major zero and moves controlled USB CDC configuration to 921600 baud to
  leave headroom for six 16-bit fields at 1000 logical frames/s. v0.1 recordings remain readable.
- Two firmware acquisition modes are deliberately fixed: simultaneous 1–6 channel frames and the
  pulse-ox RED/DARK/IR/DARK four-state cycle. The firmware is not a generic arbitrary sequencer.
- D4 is the active-HIGH green control for the BP/PPG profile; D5 and D6 are active-HIGH RED/IR.
  All three are driven LOW on startup, idle, Stop, malformed configuration, timeout, and fault.
- Course profiles are locked at their prescribed maps. Instructor drafts may make a unique A0–A5
  general-development map, but no built-in profile is edited in place.
- Course capture stores raw counts as authoritative data. Counts-to-Arduino-input-volts display is
  explicit. No SpO2, heart rate, pressure estimate, calibration fit, EMG activation/fatigue, or
  automatic filter is added in Phase 4.

## Branding

- Use the approved WVU/college lockup without redrawing or recoloring it.
- Digital UI colors:
  - WVU Blue: `#002855`
  - WVU Gold: `#EEAA00`
  - Not Quite White: `#F7F7F7`
- The included approved SVG is the primary development asset.
