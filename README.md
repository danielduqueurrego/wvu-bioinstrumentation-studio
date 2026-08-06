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
