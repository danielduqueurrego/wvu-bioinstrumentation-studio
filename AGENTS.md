# Working on WVU Bioinstrumentation Studio

Read [docs/AGENT_HANDOFF.md](docs/AGENT_HANDOFF.md) first when resuming this project on a new computer or without the previous conversation. Then read the relevant developer, protocol, hardware, or release documentation for the requested task.

## Before changing anything

- Inspect `git status --short`, the current branch, recent commits, and the actual source. The handoff contains a dated snapshot, not a permanent assertion about HEAD.
- Preserve existing uncommitted work, local instructor labs, recordings, calibration presets, and the maintainer-supplied Arduino runtime.
- Follow the current user's request. Commit, push, merge, retag, or publish only when that request authorizes it. An earlier release operation is not standing permission to rewrite releases.

## Project invariants

- This is Windows teaching software for Arduino UNO R4 WiFi. Retain the teaching-use/not-a-medical-device language.
- Raw BMEG samples are authoritative. Plot grouping, trace visibility, units, legends, endpoint labels, and live-window decimation must remain independent of acquisition and raw storage.
- Duration is explicitly `Timed` or `UntilStopped`. Keep storage protection, protocol integrity checks, serial-fault cleanup, and the seven-second no-data safeguard.
- D4 green, D5 red, and D6 IR are active HIGH and safe LOW on startup/idle/stop/fault/watchdog. Pulse ox remains raw TX/RX RED/DARK 1/IR/DARK 2; RED and IR must never be HIGH together.
- Preserve native RA4M1 USB, its pinned-core compatibility shim, absolute sampling deadlines, measured record timestamps, bounded queues, and partial/nonblocking transmit service.
- Board recovery enablement is independent of firmware readiness. Verification must not upload firmware. Failed operations must release serial ownership and busy state.
- Preserve `CREATE_NO_WINDOW`, the Windows GUI subsystem, and explicit selection of the main application binary.
- Keep Tauri command argument names and nested Rust/serde payload names aligned with their frontend callers.

## Build and verification

- Follow [docs/DEVELOPER_GUIDE.md](docs/DEVELOPER_GUIDE.md). Build the frontend before running frontend tests: a release-config test requires `build/index.html`.
- `npm run tauri build` is the production desktop pipeline. A plain Cargo release build is insufficient to package the frontend.
- `scripts/build_student_release.ps1` is the distribution pipeline. It requires the real hash-pinned `src-tauri/resources/arduino-runtime.zip`, reviewed notices, and matching firmware/release metadata.
- CI's placeholder ZIP satisfies source-check resource validation only. Never use it for hardware or release packaging.
- Use one application/serial owner during hardware testing. Do not alter the ESP32-S3 connectivity firmware as part of an ordinary WVU reference-sketch restore.
