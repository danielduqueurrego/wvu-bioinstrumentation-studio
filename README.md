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

## Firmware

```powershell
arduino-cli compile --fqbn arduino:renesas_uno:unor4wifi firmware\reference_unor4wifi
arduino-cli upload --fqbn arduino:renesas_uno:unor4wifi --port COM12 firmware\reference_unor4wifi
```

Rediscover the port with `arduino-cli board list`; do not assume COM12. The safe Phase 1
sketch forces D4, D5, and D6 LOW.

## Acceptance harness

The feature-gated harness calls the same nonblocking session start/status path as Tauri
commands, without the frontend. It is useful for controlled engineering acceptance:

```powershell
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase1_capture -- simulator 5
cargo run --manifest-path src-tauri\Cargo.toml --features acceptance-harness --bin phase1_capture -- hardware 61
```

The Phase 1 BMEG layout is `BMEGREC1`, a little-endian `u16` JSON-header length, UTF-8
metadata JSON, followed by little-endian `(u32 sample_sequence, u64 timestamp_us,
u16 counts)` records. CSV is streamed from BMEG and uses direct conversion
`volts = counts * 5.0 / 4095.0`. Generated recordings are ignored by Git.

See [PHASE_1_REPORT.md](PHASE_1_REPORT.md) and the dated files under `logs/` for
measured acceptance results.
