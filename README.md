# WVU Bioinstrumentation Studio

Windows teaching/engineering software for BMEG 420L. It is **not a medical device**.
Phase 1 uses only the simulator, the UNO R4 WiFi alone, or a safe 0–5 V bench signal
on A0. Do not connect a person or enable optical LEDs.

## Reproduce checks

```powershell
$env:Path = 'C:\Users\dd00055\.cargo\bin;' + $env:Path
npm install
cargo fmt --manifest-path src-tauri\Cargo.toml -- --check
cargo check --manifest-path src-tauri\Cargo.toml
cargo test --manifest-path src-tauri\Cargo.toml
cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets -- -D warnings
npm run check
npm test
npm run build
npm run tauri build
```

Run the desktop app with `npm run tauri dev`. Its Acquisition page uses the shared Rust
controller, bounded 25 Hz display polling, uPlot, continuous BMEG recording, metadata,
and CSV export.

## Recording duration and storage safeguards

The Acquisition page has an explicit **Timed** mode (10 seconds, 30 seconds, 60 seconds,
5 minutes, 10 minutes, or a validated custom whole-second value of at least 10 seconds) and
an explicit **Until stopped** mode. Until stopped has no hidden time limit: it ends only when
the user presses Stop recording, the transport disconnects, a fault occurs, the application
performs a controlled close, or the storage guard triggers. Raw samples stream directly to the
temporary BMEG file; the bounded live plot never retains the complete recording. The controller
warns below 1 GiB free space and performs a controlled incomplete finalization below 250 MiB.

Final metadata records duration mode, requested duration when applicable, actual duration,
stop reason, completion status, and initial/final observed free disk space. CSV continues to be
streamed from finalized BMEG rather than assembled in memory.

## UNO R4 WiFi connection recovery

Normal hardware connection uses a bounded startup grace and up to three CRC-valid PING retries.
If an idle, discovered UNO R4 WiFi opens but returns no protocol frames, the Acquisition page
shows structured diagnostics and offers **Retry handshake** first. **Reset board and retry** is a
separate, explicit user action: it closes the selected session, performs a 1200-bps touch on that
identified UNO only, polls for its returning USB port (which may have a different COM number),
and repeats the normal handshake. It never uploads firmware, cannot run during recording, and
never concatenates sessions across a reset. See
`logs/phase1_1_touch_reset_characterization_2026-08-06.md` for measured limitations.

The controlled reference identity is protocol v0.1, firmware build `0x00010001`, and device
ID `0x554E4F34`. Upload success alone is not proof of identity. After a controlled upload,
verify it with the production-parser probe shown below. The current manual Arduino CLI upload
path is verified; an in-app firmware installer is not part of the Phase 1.1 release. A known
hardware limitation remains: the explicit 1200-bps reset/retry action can return COM12 but leave
the protocol silent, requiring an explicit controlled firmware recovery rather than an automatic
upload.

## Firmware

```powershell
arduino-cli compile --fqbn arduino:renesas_uno:unor4wifi firmware\reference_unor4wifi
arduino-cli upload --fqbn arduino:renesas_uno:unor4wifi --port <CURRENT_UNO_PORT> firmware\reference_unor4wifi
```

Rediscover the port with `arduino-cli board list`; do not assume COM12. The safe Phase 1
sketch forces D4, D5, and D6 LOW.

## Phase 2 firmware workspace

The **Firmware** view is a one-file Arduino workspace for the UNO R4 WiFi. A project is a
student-selected folder containing exactly:

```text
<ProjectName>/
  <ProjectName>.ino
  project.json
```

`project.json` records the project schema, target/FQBN, timestamps, template origin,
optional notes and remembered COM port, and the last successful compile/upload identity.
Project names must begin with an ASCII letter and contain only letters, digits, and underscores;
the sketch file always matches the project folder name. Saves use a temporary sibling file then
rename where Windows permits it. Existing non-empty project folders are never overwritten.

The workspace supplies five version-controlled templates: blank UNO R4 WiFi, an ASCII A0
example, the byte-identical WVU protocol reference, a D4/D5/D6-LOW digital-output example,
and an ASCII serial diagnostic. Templates are copied into student projects; the controlled
reference source in `firmware/reference_unor4wifi/` is never edited by the workspace.

Arduino CLI is located from `C:\\arduino-cli\\arduino-cli.exe`, the current `PATH`, or the
instructor-controlled `BMEG_ARDUINO_CLI` environment variable. Editing and saving stay
available if the CLI or `arduino:renesas_uno` core is absent; the Firmware environment panel
shows the exact missing prerequisite and install command. Compile and upload use argument arrays,
capture command/output/duration/exit code, and write JSON workflow logs under the application
data directory—not the student project folder.

Upload is deliberate: the current saved source must have compiled successfully, a single
detected UNO R4 WiFi must be selected, acquisition must be stopped, and the user must confirm the
board/port warning. Arduino CLI performs the 1200-bps reset/upload transition; the workflow
rediscovers the returning application port using the UNO serial number where available and never
chooses an unrelated port. A non-WVU sketch is a successful upload but disables Acquisition.
**Restore WVU reference firmware** is a separate confirmed action that compiles the repository
reference, uploads it, and requires HELLO, CAPABILITIES, PONG, protocol v0.1,
build `0x00010001`, device `0x554E4F34`, and zero CRC failures before re-enabling Acquisition.

For a reproducible controller-level hardware sequence (A0 ASCII upload, compatibility block,
reference restore, protocol verification, and 30-second raw A0 recording), run:

```powershell
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase2_firmware_capture
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase2_firmware_capture -- validate <recording.bmeg>
```

The harness uses the same workspace, firmware workflow, and serial session controller as the
Tauri commands. It creates only a temporary student project and ignored temporary recordings.

## Phase 3A locked ECG and EMG profiles

Phase 3A adds the **General A0 — Development**, **ECG Module — Raw Output**, and **EMG Module —
Raw Output** locked profiles. All are UNO R4 WiFi / A0 / 12-bit / 1000 samples/s and display only
raw ADC counts or direct Arduino input volts (`counts * 5.0 / 4095.0`). ECG and EMG are explicitly
bench-validation profiles: **not a medical device and no human-connected recording is authorized**.
They require a session-local acknowledgement before recording.

Student mode is the default and can select valid locked profiles only. Instructor authoring mode
requires an explicit local acknowledgement, is not authentication, and creates new finalized
versions from drafts rather than editing a locked package. Every recording freezes the selected
profile snapshot into BMEG/metadata/CSV provenance. Legacy BMEG files remain readable and are
shown as general/legacy data, never inferred to be ECG or EMG. See
`docs/ACQUISITION_PROFILE_SCHEMA_v1.md` for the schema, SHA-256 integrity behavior, and built-in
profile hashes.

Bench-only controller captures can be reproduced with:

```powershell
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase3a_profile_capture -- simulator development 10
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase3a_profile_capture -- hardware ecg 30
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase3a_profile_capture -- hardware emg 30
```

## Phase 2 firmware workspace

The **Firmware** view is a one-file Arduino workspace for the UNO R4 WiFi. A project is a
student-selected folder containing exactly:

```text
<ProjectName>/
  <ProjectName>.ino
  project.json
```

`project.json` records the project schema, target/FQBN, timestamps, template origin,
optional notes and remembered COM port, and the last successful compile/upload identity.
Project names must begin with an ASCII letter and contain only letters, digits, and underscores;
the sketch file always matches the project folder name. Saves use a temporary sibling file then
rename where Windows permits it. Existing non-empty project folders are never overwritten.

The workspace supplies five version-controlled templates: blank UNO R4 WiFi, an ASCII A0
example, the byte-identical WVU protocol reference, a D4/D5/D6-LOW digital-output example,
and an ASCII serial diagnostic. Templates are copied into student projects; the controlled
reference source in `firmware/reference_unor4wifi/` is never edited by the workspace.

Arduino CLI is located from `C:\arduino-cli\arduino-cli.exe`, the current `PATH`, or the
instructor-controlled `BMEG_ARDUINO_CLI` environment variable. Editing and saving stay
available if the CLI or `arduino:renesas_uno` core is absent; the Firmware environment panel
shows the exact missing prerequisite and install command. Compile and upload use argument arrays,
capture command/output/duration/exit code, and write JSON workflow logs under the application
data directory—not the student project folder.

Upload is deliberate: the current saved source must have compiled successfully, a single
detected UNO R4 WiFi must be selected, acquisition must be stopped, and the user must confirm the
board/port warning. Arduino CLI performs the 1200-bps reset/upload transition; the workflow
rediscovers the returning application port using the UNO serial number where available and never
chooses an unrelated port. A non-WVU sketch is a successful upload but disables Acquisition.
**Restore WVU reference firmware** is a separate confirmed action that compiles the repository
reference, uploads it, and requires HELLO, CAPABILITIES, PONG, protocol v0.1,
build `0x00010001`, device `0x554E4F34`, and zero CRC failures before re-enabling Acquisition.

For a reproducible controller-level hardware sequence (A0 ASCII upload, compatibility block,
reference restore, protocol verification, and 30-second raw A0 recording), run:

```powershell
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase2_firmware_capture
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase2_firmware_capture -- validate <recording.bmeg>
```

The harness uses the same workspace, firmware workflow, and serial session controller as the
Tauri commands. It creates only a temporary student project and ignored temporary recordings.

## Acceptance harness

The feature-gated harness calls the same nonblocking session start/status path as Tauri
commands, without the frontend. It is useful for controlled engineering acceptance:

```powershell
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase1_capture -- simulator 10
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase1_capture -- simulator until 20
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase1_capture -- hardware 61
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase1_capture -- probe
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase1_capture -- verify
```

For the harness only, `until <seconds>` requests Until stopped and issues the production Stop
path after the supplied observation time; the desktop application's Stop recording button remains
the normal user control.

`probe` sends a CRC-valid PING and prints raw bytes plus frames decoded by the production parser.
`verify` runs the bounded production handshake and enforces the controlled firmware identity.
`validate <recording.bmeg>` streams the BMEG and CSV, deserializes metadata, and checks row
counts, monotonic sequences/timestamps, voltage conversion, and Until-stopped finalization fields.

The Phase 1 BMEG layout is `BMEGREC1`, a little-endian `u16` JSON-header length, UTF-8
metadata JSON, followed by little-endian `(u32 sample_sequence, u64 timestamp_us,
u16 counts)` records. CSV is streamed from BMEG and uses direct conversion
`volts = counts * 5.0 / 4095.0`. Generated recordings are ignored by Git.

See [PHASE_1_REPORT.md](PHASE_1_REPORT.md) and the dated files under `logs/` for
measured acceptance results.
