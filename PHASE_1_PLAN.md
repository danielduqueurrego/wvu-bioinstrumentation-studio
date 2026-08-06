# Phase 1 plan — tested vertical slice

## Measured baseline

- Windows reports `Windows 10 Enterprise`, version `2009`, build `26200`, 64-bit (not the stated Windows 11 assumption).
- Git `2.55.0.windows.3`; Arduino CLI `1.5.2-rc.1` at `C:\arduino-cli\arduino-cli.exe`.
- Board: UNO R4 WiFi on `COM12`, serial `48CA4360243C`, FQBN `arduino:renesas_uno:unor4wifi`; core `arduino:renesas_uno` 1.5.3.
- Rust/cargo 1.97.1 are installed in `C:\Users\dd00055\.cargo\bin` but omitted from the current `PATH`; Node 24.19.0 and npm 11.17.0 are available; WebView2 Evergreen is present.

## Dependency risk

`cl.exe` and Visual Studio Build Tools are not installed. Tauri with the installed MSVC Rust target requires Visual Studio Build Tools 2022 with **Desktop development with C++**, including MSVC x64/x86 tools and a Windows SDK. This is the smallest missing dependency; it will not be installed without approval.

## Layout and execution

```text
firmware/reference_unor4wifi/  one safe UNO R4 WiFi sketch
src/                           Svelte TypeScript UI and uPlot component
src-tauri/src/                 CLI, protocol, acquisition, recording, profiles
docs/ profiles/ schemas/       supplied controlled inputs, preserved unchanged
assets/branding/               supplied logo, preserved byte-for-byte
```

1. Initialize Git, retain supplied materials, generate the current npm Tauri 2/Svelte TypeScript template, and lock dependencies.
2. Build the WVU shell with Home, Firmware, Acquisition, and Diagnostics plus a visible teaching/not-medical-device notice.
3. Implement an argument-array Arduino CLI adapter, board discovery, safe firmware, shared protocol constants, incremental parsing, integrity counters, bounded channels, and the simulator path.
4. Write raw records continuously in `.bmeg`, plus metadata JSON and CSV; document the file layout and direct 12-bit 0–5 V conversion.
5. Test parser recovery/fuzz cases, state transitions, profiles, filename sanitation, recording/CSV, and simulator; format/lint frontend and Rust.
6. Compile/upload the firmware and, only with Arduino-alone/floating A0, conduct the authorized 60-second communication acquisition. Report exact metrics and any limitations.

## Safety / integrity controls

- Firmware accepts only one A0–A5, 12-bit, 1000 samples/s configuration; D4–D6 are initialized LOW and continually forced LOW.
- Malformed commands, bad configuration, CRC errors, reset, and command timeout stop acquisition safely.
- Frame payloads are capped at 1024 bytes; parser resynchronizes on `BMEG` and increments explicit integrity counters.
- Raw disk output is separated from bounded display data. uPlot updates are batched around 25 Hz and retain a finite window.
- Phase 1 excludes the editor, pulse-ox sequence, calibration wizards, and all physiological/clinical interpretation.

## Completion pass implementation plan — 2026-08-05

1. Move blocking byte-stream work into a cancellable Rust session worker while retaining
   only bounded display/status snapshots behind short-lived locks.
2. Use the same framed HELLO/PING/CONFIGURE/START/STOP path for serial hardware and a
   deterministic simulator transport; record only parser-validated samples.
3. Register nonblocking Tauri commands for combined Phase 1 connect/configure/record
   runs and poll bounded snapshots from the Svelte Acquisition view at 25 Hz.
4. Read back `.bmeg`, metadata, and CSV outputs; run simulator acceptance before an
   Arduino-alone, floating-A0 60-second production-controller capture.

## Phase 1.1 plan — 2026-08-06

1. Replace the fixed maximum-duration session request with an explicit, serialized
   `Timed` or `UntilStopped` mode, retaining validated timed presets and a manual stop.
2. Keep sample data streaming by adding injectable preflight/periodic disk-space checks,
   periodic flushes, and a single metadata-aware finalization path for user, timed,
   disconnect, storage, close, and fault outcomes.
3. Expose duration, elapsed time, remaining time (timed only), storage, and stop reason
   in the controller/Tauri status. Exercise the same transport path in simulator tests.
4. Reflow the desktop UI with responsive grids and a resize-observed uPlot container;
   record manual window-size observations and then perform hardware/physical-disconnect
   acceptance with Arduino alone and raw floating A0.

## Phase 1.1 reset/reconnect completion plan — 2026-08-06

1. Preserve the observed distinction between a nonresponsive USB CDC session and missing
   firmware. Add bounded, structured handshake diagnostics (open state, byte/frame/CRC counts,
   identity/PONG state, elapsed time, failure category, and next action) and retry PING without
   automatically resetting a board.
2. Add a user-initiated 1200-bps **Reset board and retry** path. It must release the port, touch
   only a discovered UNO R4 WiFi, poll Arduino/serial enumeration for a bounded period, match a
   returning device by serial/FQBN before port name, then perform a fresh production handshake.
   It will never upload firmware or reset during acquisition.
3. Cover deterministic reset matching and handshake outcomes with mocks, including delayed and
   changed-port returns, unrelated ports, ambiguity, no return, cancellation, and reset prohibition
   while recording. Keep real reset verification separate from those tests.
4. Repeat normal and post-touch handshakes, then perform the two-minute raw floating-A0
   Until-stopped recording and export validation. Only after that normal run passes will the
   user-assisted physical unplug/reconnect sequence be requested and documented.

## Phase 1.1 controlled firmware recovery plan — 2026-08-06

1. Treat the installed sketch as unknown after the Arduino analog-reading test and verify no
   interactive monitor, production app, or diagnostic worker owns the selected UNO port.
2. Compile and upload only `firmware/reference_unor4wifi/reference_unor4wifi.ino` to the
   rediscovered UNO R4 WiFi. Do not restore the ESP32-S3 connectivity firmware.
3. Use the production protocol parser to prove CRC-valid HELLO, CAPABILITIES, and PONG plus
   reference build identity before testing the Tauri session controller.
4. Run normal handshake and Until-stopped recording/export acceptance. Retain reset/retry as a
   separately reported recovery result if the fresh reference firmware still exposes a reset defect.
5. Complete only accurate manual UI and user-assisted disconnect evidence, then review current
   work and commit without amending the accepted Phase 1 baseline.
