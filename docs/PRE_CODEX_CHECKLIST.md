# Before sending the prompt to Codex

## Required now

- A new writable project folder containing this starter bundle.
- Visual Studio Code with Codex access.
- Git installed.
- Arduino CLI installed and available from PowerShell.
- Arduino UNO R4 WiFi connected with a data-capable USB-C cable.
- The UNO R4 Renesas core installed, or permission for Codex to install it during development.
- No other application holding the board's COM port.
- Permission for Codex to run terminal commands, create files, compile, upload, and run tests.
- No person connected to electrodes, optical probe, cuff, or biomedical board during the first hardware task.

## Desktop-development tools

The selected Tauri stack normally requires:

- Rust stable toolchain (`rustup`, `rustc`, `cargo`)
- Node.js LTS and npm
- Git
- Microsoft C++ Build Tools / Visual Studio Build Tools with Desktop development with C++
- Windows WebView2 runtime

Run `scripts\check_environment.ps1`; Codex should inspect the report and install only missing development dependencies with your approval.

## Helpful but not required for Phase 1

- A jumper from a safe known voltage or potentiometer to A0
- A DMM to confirm the A0 voltage
- A second analog source for later multi-channel tests
- Board/module serial-number labels
- A folder for test recordings
- A low-resource Windows computer for later acceptance testing

## Do not prepare yet

- Human ECG/EMG electrodes
- A finger pulse-oximetry test
- A pressure cuff on a participant
- Clinical reference devices
- Student names or real course records

Phase 1 must prove the software path with the Arduino alone, simulator data, or safe bench signals.
