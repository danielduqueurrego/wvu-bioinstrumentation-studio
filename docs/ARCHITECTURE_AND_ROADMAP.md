# Architecture and implementation roadmap

## 1. High-level architecture

```text
UNO R4 hardware timer / ADC
        |
Firmware sample ring buffer
        |
Binary packet encoder
        |
USB serial
        |
Rust serial reader thread
        |
Packet parser and integrity monitor
        +-------------------------+
        |                         |
Raw recording writer         Display decimator
        |                         |
.bmeg + metadata             Batched Tauri events
                                  |
                           Svelte + uPlot
```

Acquisition, disk writing, plotting, and UI interaction must not share one blocking loop.

## 2. Desktop modules

Suggested Rust modules:

```text
src-tauri/src/
  app_state.rs
  arduino_cli.rs
  board_discovery.rs
  serial/
    mod.rs
    reader.rs
    reconnect.rs
  protocol/
    mod.rs
    framing.rs
    messages.rs
    crc.rs
  acquisition/
    mod.rs
    controller.rs
    ring_buffer.rs
    statistics.rs
  recording/
    mod.rs
    bmeg_writer.rs
    csv_export.rs
    metadata.rs
  profiles/
    mod.rs
    validation.rs
  simulator/
    mod.rs
    device.rs
```

Suggested frontend areas:

```text
src/
  lib/
    components/
    stores/
    plotting/
    profiles/
    api/
  routes or views/
    Home
    Firmware
    Setup
    Acquisition
    Calibration
    Review
    Diagnostics
    Settings
```

## 3. Threading and data flow

- One blocking serial reader thread is sufficient initially.
- Decode and validate packets in Rust.
- Batch frontend updates at 20–30 Hz.
- Write raw data to disk before display downsampling.
- Use bounded channels/ring buffers and explicit overflow counters.
- Do not emit one Tauri event per ADC sample.

## 4. Arduino CLI integration

Initial integration uses subprocesses:

- `arduino-cli version`
- `arduino-cli board list --format json`
- `arduino-cli core list --format json`
- `arduino-cli compile --fqbn arduino:renesas_uno:unor4wifi --format json`
- `arduino-cli upload --fqbn arduino:renesas_uno:unor4wifi --port <COM>`

Store exact command, exit code, stdout, and stderr in the diagnostic log. Do not assume a fixed Arduino CLI installation path; discover it and allow an instructor override.

## 5. Firmware organization

Student-visible firmware is one `.ino` file. Codex may generate internal code before packaging, but the approved student template must remain a single-file sketch.

The firmware must:

- Initialize all optical LED outputs low.
- Report firmware and protocol identity.
- Accept configuration before starting.
- Use deterministic sampling.
- Batch samples.
- Include sequence numbers and timestamps.
- Stop safely on command timeout or reset.
- Support simulator-compatible message semantics.

## Phase 2 firmware-workspace flow

```text
CodeMirror editor -> explicit Save -> single-file project.json model
                         |
                  Arduino CLI compile (argument array)
                         |
                current-source build artifact
                         |
explicit confirmed upload -> release shared SessionController serial handle
                         |
 Arduino CLI touch reset / upload -> serial-number-first port rediscovery
                         |
    declared non-WVU sketch                 declared WVU reference
      -> Acquisition disabled          -> production HELLO/CAPABILITIES/PONG
                                               + identity verification
                                                     |
                                           Acquisition re-enabled
```

`FirmwareWorkflow` coordinates one active compile/upload job and shares the same
`SessionController` clone held by Tauri application state. It never exposes a serial handle to
the frontend and keeps a job visible until its terminal result and diagnostic log are published.

## 6. Development phases

### Phase 3A — locked raw ECG/EMG profile foundation

- Versioned validated profile packages and SHA-256 integrity checks
- Student selection and locally acknowledged instructor draft/finalize workflow
- Frozen profile snapshot in BMEG/metadata/CSV provenance
- Bench-only ECG/EMG A0 raw acquisition at 12-bit / 1000 Hz
- No human authorization, module characterization, physiological interpretation, leads-off,
  optional notch channel, or EMG 2000 Hz work

### Phase 1 — tested vertical slice

- Repository and toolchain
- Tauri/Svelte scaffold
- WVU branding shell
- Board discovery
- Minimal reference firmware
- Binary protocol v0.1
- One analog channel
- Start/stop acquisition
- Bounded live plot
- Short `.bmeg` recording
- CSV/JSON export
- Device simulator
- Automated tests
- Hardware report

### Phase 2 — firmware workspace

- CodeMirror editor
- Approved template and working copy
- Compile/upload UX
- Error navigation
- Restore/compare template
- Profile validation and pin mapper

### Phase 3 — ECG & EMG

- ECG/EMG locked profile
- Optional notch channel
- Leads-off status
- 1000/2000 samples/s validation
- Ten-minute recording
- One-hour stress test

### Phase 4 — Pulse Oximetry

- Active-high LED sequencer
- Dark/red/IR and optional green modes
- Reflection/transmission acquisition
- Manual DIP-gain entry
- Preflight and clipping guidance
- Raw and dark-corrected export

### Phase 5 — Blood Pressure

- MPXV nominal conversion
- MPXV calibration workflow
- XGZP raw counts/volts
- Event markers and pressure-specific metadata

### Phase 6 — packaging and classroom hardening

- Offline dependency bundle
- Windows installer and portable build
- WebView runtime strategy
- Low-resource tests
- Accessibility review
- Recovery/reconnect testing
- Instructor documentation
