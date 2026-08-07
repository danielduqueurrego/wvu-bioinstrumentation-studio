# Phase 1 report — 2026-08-05

## Acceptance outcome

**Phase 1 is accepted for normal simulator and Arduino-alone acquisition.** The
production controller, its Tauri command path, recording/export path, and final
60-second hardware evidence pass with credible zero-loss counters. No person or
biomedical accessory was connected.

Physical unplug/replug was later verified on 2026-08-07 with the user's manual participation;
the original normal-acquisition acceptance remains unchanged. See
`logs/phase1_physical_disconnect_verification_2026-08-06.md`.

## Environment

| Component | Measured value |
|---|---|
| Windows user-facing identity | Microsoft Windows 11 Enterprise, 10.0.26200 |
| Compatibility API identity | Windows 10 Enterprise / 2009 / build 26200 |
| Git | 2.55.0.windows.3 |
| Rust host / rustc / cargo | x86_64-pc-windows-msvc / 1.97.1 / 1.97.1 |
| Node / npm | 24.19.0 / 11.17.0 |
| Arduino CLI | 1.5.2-rc.1, `C:\arduino-cli\arduino-cli.exe` |
| UNO R4 core | arduino:renesas_uno 1.6.0 |
| Board | Arduino UNO R4 WiFi, COM12, serial 48CA4360243C |
| FQBN | `arduino:renesas_uno:unor4wifi` |
| Native prerequisites | VS Community 2026 / MSVC x64/x86 toolset / Windows SDK 10.0.26100.0 |
| WebView2 | Evergreen runtime present |

Rust is installed at `C:\Users\dd00055\.cargo\bin`; the persistent PowerShell PATH
still should be repaired, although all verification commands prepended it for the
current process.

## Implemented vertical slice

- One Rust-owned `SessionController` with explicit disconnected, connecting, connected,
  configured, acquiring, stopping, and faulted states.
- Blocking serial work and disk I/O execute in a cancellable worker; Tauri status and
  bounded recent-data calls take only short locks.
- Tauri exposes board/port discovery, combined connect/handshake/configure/start,
  stop, disconnect, status, bounded display data, and CSV-path export commands.
- The Svelte Acquisition view selects hardware/simulator, polls at 25 Hz, displays
  integrity/state/file paths, and plots a bounded raw counts-or-volts history with uPlot.
- Serial and deterministic simulator transports share protocol framing, parser,
  integrity, recording, metadata, CSV export, and state behavior.
- `.bmeg` is written continuously to a temporary file then finalized; CSV is streamed
  from finalized BMEG without retaining the recording in RAM.
- Firmware remains UNO R4 WiFi-only, one A0–A5 / 12-bit / 1000 Hz channel. D4–D6 are
  initialized LOW and continually forced LOW. No physiological function was added.

Two measured firmware defects were fixed and regression-tested: late host opens can
re-request HELLO/CAPABILITIES/PONG via PING, and incomplete back-to-back identity
frames are rejected/resynchronized. Board timestamps now advance from the scheduled
1000 Hz clock rather than USB-transmit jitter.

## Verification results

| Command | Result |
|---|---|
| `cargo fmt --manifest-path src-tauri\Cargo.toml -- --check` | Passed |
| `cargo check --manifest-path src-tauri\Cargo.toml` | Passed |
| `cargo test --manifest-path src-tauri\Cargo.toml` | Passed: 19 tests |
| `cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets -- -D warnings` | Passed |
| `npm run check` | Passed: 0 errors, 0 warnings |
| `npm test` | Passed: 1 frontend test |
| `npm run build` | Passed |
| `npm run tauri build` | Passed: MSI and NSIS bundles |
| Release executable launch | Passed: running after 5 seconds |
| Firmware compile/upload | Passed: 53,508 bytes flash; 7,940 bytes RAM; COM12 |

## Simulator acceptance

Five seconds through the nonblocking production controller produced 5,000 validated
samples at 1000.000 Hz, 509 valid packets, and zero CRC, frame, sequence, or overflow
errors. BMEG/metadata/CSV read-back passed. See
`logs/phase1_simulator_acceptance_2026-08-05.md`.

## Hardware acceptance

The final Arduino-alone floating-A0 run used the same `start_serial` and status-poll
path used by Tauri commands. It produced 60,850 samples over 60.849 board seconds;
the measured board rate was exactly 1000.000 Hz (0.000% error). All CRC, invalid,
missing, duplicate, out-of-order, firmware-overflow, host-overflow, reconnect, and
disconnect counters were zero. BMEG had 60,850 records; CSV had 60,850 data rows;
metadata matched. See `logs/phase1_hardware_acceptance_2026-08-05.md` and
`logs/phase1_export_validation_2026-08-05.md`.

The final generated files are intentionally ignored:

- `recordings/20260805_154043_Phase1_A0_Run01.bmeg`
- `recordings/20260805_154043_Phase1_A0_Run01.metadata.json`
- `recordings/20260805_154043_Phase1_A0_Run01.csv`

## Disconnect/reconnect status

- Automated terminal-disconnect finalization: passed. The session finalizes a readable
  `disconnected` recording, increments the disconnect counter, faults visibly, and
  requires an explicit new start.
- Physical unplug/replug: passed on 2026-08-07. The app faulted visibly without crashing,
  finalized a readable incomplete 30,690-sample recording with stop reason `disconnect`,
  rediscovered COM12 after reconnect, reverified the controlled identity, and required a
  separate successful 9,970-sample recording. No automatic concatenation/restart is implemented.
  See `logs/phase1_physical_disconnect_verification_2026-08-06.md`.

## Known limitations

- The 60-second controller run was headless; the release app launch was confirmed, but
  visual plot responsiveness was not manually observed during that run. The UI design
  polls only bounded snapshots at 25 Hz and related tests pass.
- Serial reconnection is intentionally explicit rather than automatic in Phase 1.
- The firmware is a teaching communication reference, not a clinical or physiological
  measurement implementation.

## Exact next recommended task

The physical unplug/replug acceptance test is complete. The next remaining hardware work is
separate Phase 3B ECG/EMG bench validation with documented safe sources; no human-connected work
is authorized.

## Phase 1.1 follow-up status — 2026-08-06

The accepted Phase 1 evidence above is preserved. Phase 1.1 adds explicit `timed` and
`until_stopped` duration modes, bounded long-recording storage safeguards, controlled
application-close finalization, structured handshake diagnostics, controlled firmware identity
checking, and a responsive Acquisition layout with a `ResizeObserver`-driven uPlot resize path.
It does not add biomedical interpretation, optical sequencing, or calibration workflow.

### Controlled firmware identity and normal handshake

An analog ASCII test sketch had replaced the reference firmware. The user initially restored the
repository sketch through Arduino IDE, but a production-parser probe then reproduced a source
defect in the original HELLO payload: the firmware build and device ID were written at the same
offset. The controlled v0.1 correction writes the device ID at byte offset four and uses build
`0x00010001`. Arduino CLI compiled (53,508 bytes flash; 7,940 bytes RAM) and uploaded that exact
controlled binary to rediscovered COM12. Independent production-parser proof after the final
upload: HELLO/CAPABILITIES/PONG true; CRC/invalid/noise counts zero; protocol 0.1; build
`0x00010001`; device `0x554E4F34`. The normal production handshake passed in 1261 ms with 64
bytes and three valid frames. Details: `logs/phase1_1_reference_firmware_restore_2026-08-06.md`.

### Simulator and hardware acceptance

The latest production-controller simulator Until-stopped run manually stopped cleanly with 18,760
samples, 1,899 valid packets, 1000.000 Hz, zero integrity failures, a readable BMEG/CSV/metadata
triplet, and bounded display history. The deterministic 900,000-sample 15-minute-equivalent soak
also passed with its fixed 1,500-point display history.

The required Arduino-alone floating-A0 Until-stopped hardware run passed through the production
session controller. It ran for 121.495 host seconds (121.119 seconds of board timestamps), then
manually finalized 121,120 samples and 12,480 valid packets at exactly 1000.000 Hz. CRC failures,
invalid/unsupported frames, missing/duplicate/out-of-order sequences, firmware/host overflows,
disconnects, and reconnects were all zero. BMEG/metadata/CSV streaming validation passed: 121,120
records/rows, contiguous 0–121119 sequences, monotonic timestamps, correct direct voltage
conversion, and metadata `until_stopped` / `user` / `complete`. Details:
`logs/phase1_1_indefinite_recording_acceptance_2026-08-06.md` and
`logs/phase1_1_export_validation_2026-08-06.md`.

### Verification results

| Command | Result |
|---|---|
| `cargo fmt --manifest-path src-tauri\Cargo.toml -- --check` | Passed |
| `cargo check --manifest-path src-tauri\Cargo.toml` | Passed |
| `cargo test --manifest-path src-tauri\Cargo.toml` | Passed: 33 Rust tests |
| `cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets -- -D warnings` | Passed |
| `npm run check` | Passed: zero errors and warnings |
| `npm test` | Passed: 6 frontend tests in 3 files |
| `npm run build` | Passed |
| `npm run tauri build` | Passed: current MSI and NSIS bundles |
| Release application launch | Passed; launched and closed cleanly, no serial session opened |

### Separately reported follow-ups

- Reset/retry hardware recovery remains unresolved. Its 1200-bps touch rediscovered COM12 but
  later received zero protocol bytes; re-uploading the controlled binary restored normal protocol
  operation. This does not affect the credible normal-acquisition counters. See
  `logs/phase1_1_touch_reset_characterization_2026-08-06.md`.
- Physical unplug/replug passed on 2026-08-07; see
  `logs/phase1_physical_disconnect_verification_2026-08-06.md`.
- The requested manual window-size and Windows-scaling matrix has not been visually observed in
  this noninteractive agent desktop. It is documented as pending, not passed, in
  `logs/phase1_1_responsive_ui_verification_2026-08-06.md`.

Phase 1.1 normal recording/data-integrity implementation is committed as `cdc2c8e`.
The accurately recorded broad Phase 1.1 visual matrix remains a separate pending manual
follow-up; physical disconnect is now represented as passed above.

## Phase 2 firmware-workspace implementation note — 2026-08-06

Phase 2 adds the single-file CodeMirror 6 firmware workspace, versioned local project model,
Arduino CLI compile/upload workflow, serial-number-first returning-port selection, and explicit
controlled reference restore. It retains the Phase 1 session controller as the sole serial owner:
the workflow releases that controller before upload and verifies the reference through the same
production parser/session handshake afterward.

Controller-level hardware evidence passed on UNO R4 WiFi COM12 / serial `48CA4360243C` / core
1.6.0. The workflow compiled and uploaded the declared non-WVU A0 ASCII template (52,032 bytes
flash, 6,740 bytes RAM), correctly disabled Acquisition, compiled/restored the controlled
reference (53,508 bytes flash, 7,940 bytes RAM), and verified protocol 0.1, build `0x00010001`,
device `0x554E4F34`, three valid frames, and zero CRC failures. A following Arduino-alone
30-second A0 recording yielded 29,930 contiguous samples in 29.929 board seconds at exactly
1000.000 Hz; all CRC, invalid-frame, sequence, overflow, disconnect, and reconnect counters
were zero. BMEG/CSV/metadata streaming validation passed with 29,930 records/rows and direct
voltage conversion checks. See the Phase 2 logs for paths and exact CLI arguments.

This is not a new Phase 1 acceptance claim. The post-fix Firmware-page inspection passed at
900 × 650 and wide/maximized windows: the editor expands into the available workspace, the
environment panel remains readable, the console is accessible, and neither page-level horizontal
overflow nor clipping/overlap was reported. The exact numeric Windows scaling and separate
1024 × 768, 1366 × 768, and 1920 × 1080 observations were not recorded and remain a clearly
labeled Phase 2 documentation follow-up. See the Phase 2 workspace acceptance log.
