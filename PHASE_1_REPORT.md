# Phase 1 report — 2026-08-05

## Acceptance outcome

**Phase 1 is accepted for normal simulator and Arduino-alone acquisition.** The
production controller, its Tauri command path, recording/export path, and final
60-second hardware evidence pass with credible zero-loss counters. No person or
biomedical accessory was connected.

Physical unplug/replug is **not performed** because it requires the user's manual
participation. Automated terminal-disconnect finalization is covered by test.

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
- Physical unplug/replug: not performed. It must be conducted with the user present;
  no automatic concatenation/restart is implemented.

## Known limitations

- The 60-second controller run was headless; the release app launch was confirmed, but
  visual plot responsiveness was not manually observed during that run. The UI design
  polls only bounded snapshots at 25 Hz and related tests pass.
- Serial reconnection is intentionally explicit rather than automatic in Phase 1.
- The firmware is a teaching communication reference, not a clinical or physiological
  measurement implementation.

## Exact next recommended task

Perform the user-assisted physical unplug/replug acceptance test: unplug the UNO R4
WiFi during a recording, verify incomplete finalization, reconnect/redetect its port,
and require an explicit new acquisition. Then begin Phase 2 only after recording the
result.
