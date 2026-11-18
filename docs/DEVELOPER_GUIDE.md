# Developer Guide

For resuming development after a computer reset or transferring work to another agent, start with the [Maintainer and Agent Handoff](AGENT_HANDOFF.md). It covers local assets, catalog recovery, pending work, and the current release baseline.

## Prerequisites

Development is Windows-first. Install:

- Git and Node.js 22 or later with npm (`package.json` `engines`; CI uses Node 22).
- Rust through rustup with the MSVC host. `rust-toolchain.toml` pins the exact Rust version with rustfmt and Clippy; rustup installs it the first time cargo runs in the repository (or run `rustup toolchain install`). Change that pin deliberately and rerun every check, because a new Clippy can fail `-D warnings` on unchanged code.
- The Visual Studio C++ build tools (MSVC) with a Windows 10/11 SDK, and the WebView2 runtime: the [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows).
- PowerShell 7 (`pwsh`) for `scripts/build_student_release.ps1` and `scripts/package_op_amp_lab_installer.ps1`. The student lab installer and the other scripts also run in Windows PowerShell 5.1.

`scripts/check_environment.ps1` checks these read-only and exits with code 1 when something required is missing; `-ReportPath` saves a copy. An Arduino IDE is not required for the distributed application; developers who run hardware tools outside the app need the approved UNO R4 environment.

## Setup and checks

```powershell
npm ci
pwsh -NoProfile -File scripts\check_release_consistency.ps1
npm run check
npm run build
npm test
cargo fmt --manifest-path src-tauri\Cargo.toml -- --check
cargo check --manifest-path src-tauri\Cargo.toml --all-targets --locked
cargo test --manifest-path src-tauri\Cargo.toml --locked
cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets --locked -- -D warnings
```

Build the frontend before `npm test`: a release-configuration test requires `build/index.html`. `--locked` makes cargo fail instead of silently rewriting an out-of-date `Cargo.lock`. Cargo needs `src-tauri/resources/arduino-runtime.zip`; see [Bundled Arduino runtime](#bundled-arduino-runtime).

`scripts/check_release_consistency.ps1` compares the application version in `package.json`, the `package-lock.json` root, `Cargo.toml`, the `Cargo.lock` root package, `tauri.conf.json` and `release/release-manifest.json`; requires a `CHANGELOG.md` heading and `RELEASE_NOTES_<version>.md` for it; and checks the firmware protocol, build and device ID, the reference `.ino` SHA-256, and the Arduino CLI and core versions against the manifest and the Rust constants. It also requires each Tauri npm package (`@tauri-apps/api`, `@tauri-apps/plugin-*`) in `package-lock.json` to share its major.minor release with the matching crate (`tauri`, `tauri-plugin-*`) in `Cargo.lock`, because `tauri build` refuses a mismatch. Comparisons are exact, including case. It lists every mismatch and exits with code 1. CI and the release script run it, so bump all of these together.

## Continuous integration

`.github/workflows/ci.yml` runs on Windows with the pinned toolchain and a Rust build cache. Besides the checks above, it confirms that `scripts/install_op_amp_lab_profile.zip` matches its scripts and installs the Op Amp Lab profile into temporary catalogs with Windows PowerShell 5.1, PowerShell 7 and the `.bat` launcher, checking the installed profile hash, that an existing catalog keeps its other labs and is backed up exactly once, an unchanged catalog on a second run, and exit code 1 for an invalid catalog. Third-party actions are pinned to commit SHAs, with the release tag in a comment.

`.github/workflows/audit.yml` runs weekly, on demand, and for pull requests that change a manifest or lockfile. `npm audit --omit=dev --audit-level=high` and `cargo audit` block; `npm audit` over all dependencies and the full `cargo audit` report are informational. The blocking `cargo audit` ignores only the reviewed `zip` 2.x advisories listed in that workflow, whose fixes need a semver-major `zip` upgrade; remove each ignore once `zip` is upgraded. `.github/dependabot.yml` opens weekly update pull requests for npm, Cargo and GitHub Actions, grouping minor and patch updates. Tauri npm packages and their crates arrive in separate npm and Cargo pull requests; when one moves to a new minor release, CI fails on the Tauri version check until both are updated together.

## Development and production

Use `npm run tauri dev` during development. It uses the Vite development URL.

To keep a development session away from your real lab catalog, calibration presets, logs and extracted runtime, run `pwsh -NoProfile -File scripts\start_dev_sandbox.ps1`. It starts `npm run tauri dev` with `LOCALAPPDATA` and the debug-only `WVU_STUDIO_DEV_LOCAL_DATA` pointed at `.dev-sandbox\` (or `-SandboxRoot <absolute path>`), and on first use points the Project folder at `<sandbox>\recordings`. `-RemoteDebuggingPort 9222` also enables WebView2 remote debugging on that port. Delete the folder to start fresh; Git ignores `.dev-sandbox/`.

Use `npm run tauri build` for a production executable and installers. It runs the frontend production build and packages the configured local `build` directory into Tauri. Do **not** treat `cargo build --release` as a production application build: it does not package the frontend assets and can open a localhost error page.

`src-tauri/build.rs` embeds the source commit (`git rev-parse HEAD`) as `GIT_COMMIT`, which firmware restore logs record as `git_revision`. An explicit `GIT_COMMIT` environment variable takes precedence. Outside a Git work tree, or without `git`, the value is simply absent (`null`); the build never fails for that reason. The commit does not show uncommitted changes, so build releases from a clean tree.

## Repository layout

- `src/`: Svelte frontend and live-plot UI.
- `src-tauri/src/`: Rust acquisition, protocol, lab catalog, calibration, runtime, and reference-firmware workflows.
- `firmware/reference_unor4wifi/`: controlled UNO R4 WiFi reference firmware.
- `profiles/`: immutable factory course-lab definitions.
- `docs/`: user, instructor, maintainer, protocol, and release documentation.
- `src-tauri/icons/`: only the icons `tauri.conf.json` `bundle.icon` lists (the Windows build uses `icon.ico`). If you regenerate them with `npm run tauri icon`, delete the Android, iOS and Microsoft Store (`Square*Logo.png`, `StoreLogo.png`) outputs it adds.
- `scripts/build_student_release.ps1`: canonical distribution build script (PowerShell 7).
- `scripts/check_release_consistency.ps1`: version, firmware and runtime metadata check used by CI and the release script.
- `scripts/check_environment.ps1`: read-only check of the development prerequisites.
- `scripts/start_dev_sandbox.ps1`: `npm run tauri dev` with disposable app data.
- `scripts/install_op_amp_lab_profile.ps1` and `.bat`: the student Op Amp Lab installer; `scripts/package_op_amp_lab_installer.ps1` rebuilds its ZIP (see below).

## Student Op Amp Lab installer

`scripts/install_op_amp_lab_profile.zip` is the student download. It contains `install_op_amp_lab_profile.bat` and then `install_op_amp_lab_profile.ps1`, both with CRLF line endings, stored uncompressed with a fixed timestamp, so the same scripts always give the same ZIP bytes. `.gitattributes` checks out `*.bat`, `*.cmd` and `*.ps1` with CRLF on every host. After changing either script, run `pwsh -NoProfile -File scripts\package_op_amp_lab_installer.ps1` and commit the regenerated ZIP with the change; CI runs it with `-Check` and fails on a stale ZIP. Keep both scripts ASCII so Windows PowerShell 5.1 reads them unchanged, and do not edit the embedded profile JSON: a Rust test checks its canonical hash, and any profile change needs a new locked version and hash.

## Bundled Arduino runtime

The release build requires `src-tauri/resources/arduino-runtime.zip`, which is intentionally not tracked because it contains third-party binary tooling. Its version is pinned by `arduino-runtime-manifest.json`; the release script verifies the reviewed archive SHA-256 and the reference-firmware SHA-256 before it stages artifacts. Tauri also validates resource existence during Cargo source checks. In CI, the workflow creates a disposable placeholder ZIP for those checks; a fresh local clone needs that same source-check preparation or the real archive. The placeholder cannot be used for firmware restore or student packaging. A maintainer must supply the reviewed real archive with its upstream notices and verify the manifest before building a student installer. Do not download or substitute “latest” Arduino tooling during a release build.

Run `scripts/audit_bundled_runtime_notices.ps1` against the pinned archive before publishing an installer. It verifies the reviewed Arduino CLI license copy, the component-specific BOSSA license, required runtime components, and the retained LICENSE/COPYING/NOTICE inventory. A maintainer must still review upstream redistribution requirements when any bundled version changes.

`scripts/build_student_release.ps1` requires PowerShell 7 and a Git work tree. It stops on the first failed native command (including `git`), runs the consistency check, reinstalls npm packages with `npm ci` (network access or a warm npm cache), builds with `--locked`, and stages only installers created by that invocation. Do not reuse or manually copy an older installer after a failed release build. In `dist\`, the student folder's `SHA256SUMS.txt` lists the two installers and is identical to the copy inside the ZIP; `<zip>.sha256` beside the ZIP holds the ZIP's checksum; and `WVU-Bioinstrumentation-Studio-<version>-build-info.txt` records the source commit and tool versions. All checksum files use the `sha256sum` layout with LF line endings.

## Firmware

The application restores only `firmware/reference_unor4wifi/reference_unor4wifi.ino`. Production external commands are launched by the Rust backend with hidden Windows-console creation and captured output. Keep the D4/D5/D6 safe-LOW and pulse-ox mutual-exclusion invariants intact. Protocol 0.4 records firmware-measured timestamps for every synchronized record, advances pulse-state deadlines absolutely, and uses bounded raw-batch and nonblocking transmit queues; do not move packet encoding, CRC construction, or serial writes back into either acquisition timing path.
