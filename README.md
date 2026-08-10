# WVU Bioinstrumentation Studio

Windows teaching software for BMEG 420L. It is **not a medical device** and never produces
diagnostic or treatment advice. The app records raw engineering signals and preserves the timing,
channel map, firmware identity, profile snapshot, and integrity counters needed for course work.

## Current Phase 4 capabilities

- One UNO R4 WiFi at a time, with controlled protocol v0.2 firmware:
  build `0x00010002`, device `0x554E4F34`, USB CDC configured at 921600 baud.
- Synchronized logical analog frames with one to six unique A0–A5 inputs; ADC reads are sequential
  within a frame, not electrically simultaneous.
- Locked course captures: ECG (A0); EMG + force (A0–A3); blood pressure + PPG (A0–A2, D4 green);
  and pulse-ox TX/RX raw capture (A0/A1, D5 red, D6 IR).
- Timed and **Until stopped** recording, bounded uPlot display updates, markers, continuous BMEG,
  profile-aware CSV, metadata, storage guards, disconnect finalization, and simulator support.
- A firmware workspace for one-file UNO projects plus locked, profile-aware course capture and
  display-only plot groups. Formal analog-module characterization is outside the runtime app
  scope and does not gate ordinary course capture.

See [course profile mapping](docs/COURSE_ACQUISITION_PROFILES.md),
[protocol v0.2](docs/USB_PROTOCOL_SPECIFICATION_v0.2.md), and
[profile schema](docs/ACQUISITION_PROFILE_SCHEMA_v1.md).

## Safety boundary

Follow BMEG 420L lab instructions and instructor safety procedures. Do not use this app for
diagnosis or clinical decisions. The app does not calculate heart rate, SpO2, blood pressure, EMG
activation/fatigue, or any physiological interpretation. Raw ADC counts remain authoritative;
volts are direct Arduino-input conversion only, never calibrated physiological units.

The reference firmware makes D4/D5/D6 LOW at startup, idle, Stop, protocol/configuration errors,
and watchdog faults. D4 may be HIGH only during the configured BP/PPG capture; D5/D6 are active
HIGH only during the fixed pulse-ox RED/DARK/IR/DARK sequence and are never HIGH together.

## Build and checks

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

Run the desktop app with `npm run tauri dev`. The frontend only polls bounded snapshots at about
25 Hz; it does not receive one UI event per raw ADC record.

## Controlled firmware

Close Arduino IDE, Serial Monitor, Serial Plotter, and other serial tools before using the app.
Rediscover the port; do not assume `COM12`.

```powershell
arduino-cli board list --format json
arduino-cli compile --fqbn arduino:renesas_uno:unor4wifi firmware\reference_unor4wifi
arduino-cli upload --fqbn arduino:renesas_uno:unor4wifi --port <CURRENT_UNO_PORT> firmware\reference_unor4wifi --verbose
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase1_capture -- probe
```

The probe must show HELLO, CAPABILITIES, PONG, zero CRC failures, protocol 0.2, build
`0x00010002`, and device `0x554E4F34`. A successful upload alone is not identity proof. The
Firmware workspace’s **Restore WVU reference firmware** action uses the same controlled source and
requires a verified protocol handshake before Acquisition is re-enabled.

## Recording and exports

BMEG is the authoritative raw recording and streams while acquisition runs; the entire session is
never held in RAM. Metadata includes start/stop time, duration mode, profile snapshot, firmware,
board/port, active analog pins, digital mapping, ADC/rate, markers, free space, completion/stop
reason, and integrity counters. CSV streams from BMEG after finalization.

For Phase 4 BMEG/CSV, leading columns are `record_sequence,t_us` (or `cycle_index,t_us` for
pulse ox), followed by profile-defined raw count fields. Existing Phase 1–3 single-channel BMEG
files remain readable and are never relabelled as course profiles.

## Controlled acceptance harnesses

The harnesses call the same Rust session/controller path as Tauri and write temporary ignored
outputs. They never authorize human measurement.

```powershell
# protocol identity probe
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase1_capture -- probe

# Phase 4 simulator or UNO-only (floating inputs/safe bench source) profile smoke capture
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase4_multichannel_capture -- simulator emg 10
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase4_multichannel_capture -- hardware ecg 30
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase4_multichannel_capture -- hardware emg 30
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase4_multichannel_capture -- hardware bp 30
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase4_multichannel_capture -- hardware pulseox 30
```

`KNOWN_ISSUES.md` lists the separately documented reset/retry recovery limitation and any pending
manual display-scaling checks.
