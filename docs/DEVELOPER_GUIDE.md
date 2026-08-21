# Developer Guide

## Prerequisites

Development is Windows-first. Install a current Node.js/npm environment, Rust stable with the Windows toolchain, and the Tauri prerequisites for Windows. An Arduino IDE is not required for the distributed application; developers who run hardware tools outside the app need the approved UNO R4 environment.

## Setup and checks

```powershell
npm ci
cargo fmt --manifest-path src-tauri\Cargo.toml -- --check
cargo check --manifest-path src-tauri\Cargo.toml
cargo test --manifest-path src-tauri\Cargo.toml
cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets -- -D warnings
npm run check
npm run build
npm test
```

## Development and production

Use `npm run tauri dev` during development. It uses the Vite development URL.

Use `npm run tauri build` for a production executable and installers. It runs the frontend production build and packages the configured local `build` directory into Tauri. Do **not** treat `cargo build --release` as a production application build: it does not package the frontend assets and can open a localhost error page.

## Repository layout

- `src/`: Svelte frontend and live-plot UI.
- `src-tauri/src/`: Rust acquisition, protocol, lab catalog, calibration, runtime, and reference-firmware workflows.
- `firmware/reference_unor4wifi/`: controlled UNO R4 WiFi reference firmware.
- `profiles/`: immutable factory course-lab definitions.
- `docs/`: user, instructor, maintainer, protocol, and release documentation.
- `scripts/build_student_release.ps1`: canonical distribution build script.

## Bundled Arduino runtime

The release build requires `src-tauri/resources/arduino-runtime.zip`, which is intentionally not tracked because it contains third-party binary tooling. Its version is pinned by `arduino-runtime-manifest.json`; the release script verifies the reviewed archive SHA-256 and the reference-firmware SHA-256 before it stages artifacts. A fresh clone can run source checks without the archive, but cannot make the student installer until a maintainer supplies the reviewed archive with its upstream notices and verifies the manifest. Do not download or substitute “latest” Arduino tooling during a release build.

Run `scripts/audit_bundled_runtime_notices.ps1` against the pinned archive before publishing an installer. It verifies the reviewed Arduino CLI license copy, the component-specific BOSSA license, required runtime components, and the retained LICENSE/COPYING/NOTICE inventory. A maintainer must still review upstream redistribution requirements when any bundled version changes.

`scripts/build_student_release.ps1` stops on the first failed native command and stages only installers created by that invocation. Do not reuse or manually copy an older installer after a failed release build.

## Firmware

The application restores only `firmware/reference_unor4wifi/reference_unor4wifi.ino`. Production external commands are launched by the Rust backend with hidden Windows-console creation and captured output. Keep the D4/D5/D6 safe-LOW and pulse-ox mutual-exclusion invariants intact.
