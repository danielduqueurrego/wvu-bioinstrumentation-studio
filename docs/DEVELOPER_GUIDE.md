# Developer Guide

For resuming development after a computer reset or transferring work to another agent, start with the [Maintainer and Agent Handoff](AGENT_HANDOFF.md). It covers local assets, catalog recovery, the current branch and release baseline, open hardware checks, and deferred decisions. [Architecture](ARCHITECTURE.md) describes the code structure; [USB Protocol](USB_PROTOCOL.md) is the byte-level protocol reference.

## Prerequisites

Development is Windows-first. Install:

- Git and Node.js 22 or later with npm (`package.json` `engines`; CI uses Node 22).
- Rust through rustup with the MSVC host. `rust-toolchain.toml` pins the exact Rust version with rustfmt and Clippy; rustup installs it the first time cargo runs in the repository (or run `rustup toolchain install`). Change that pin deliberately and rerun every check, because a new Clippy can fail `-D warnings` on unchanged code.
- The Visual Studio C++ build tools (MSVC) with a Windows 10/11 SDK, and the WebView2 runtime: the [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows).
- PowerShell 7 (`pwsh`) for `scripts/build_student_release.ps1`. The other scripts also run in Windows PowerShell 5.1.

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

`scripts/check_release_consistency.ps1` compares the application version in `package.json`, the `package-lock.json` root, `Cargo.toml`, the `Cargo.lock` root package, `tauri.conf.json` and `release/release-manifest.json`; requires a `CHANGELOG.md` heading and `RELEASE_NOTES_<version>.md` for it; and checks the firmware protocol, build and device ID, the reference `.ino` SHA-256, and the Arduino CLI and core versions against the manifest and the Rust constants. It also requires each Tauri npm package (`@tauri-apps/api`, `@tauri-apps/plugin-*`) in `package-lock.json` to share its major.minor release with the matching crate (`tauri`, `tauri-plugin-*`) in `Cargo.lock`, because `tauri build` refuses a mismatch. It requires `src-tauri/resources/licenses/LICENSE.txt` and `NOTICE.txt`, which the installers ship, to be byte-identical to the root `LICENSE` and `NOTICE` and to be listed in `tauri.conf.json` `bundle.resources`, and the `Copyright` line in `NOTICE` to equal `bundle.copyright`; after editing `NOTICE`, copy it over `NOTICE.txt` unchanged. It requires `bundle.windows.nsis.template` to be `nsis/installer.nsi`, and that template to match the installed Tauri CLI (see [Re-syncing the NSIS template](#re-syncing-the-nsis-template)). Comparisons are exact, including case. It lists every mismatch and exits with code 1. CI and the release script run it, so bump all of these together.

## Continuous integration

`.github/workflows/ci.yml` runs on Windows with the pinned toolchain and a Rust build cache, and runs the checks above. Third-party actions are pinned to commit SHAs, with the release tag in a comment.

`.github/workflows/audit.yml` runs weekly, on demand, and for pull requests that change a manifest or lockfile. `npm audit --omit=dev --audit-level=high` and `cargo audit` block; `npm audit` over all dependencies and the full `cargo audit` report are informational. The blocking `cargo audit` ignores only the reviewed `zip` 2.x advisories listed in that workflow, whose fixes need a semver-major `zip` upgrade; remove each ignore once `zip` is upgraded. `.github/dependabot.yml` opens weekly update pull requests for npm, Cargo and GitHub Actions, grouping minor and patch updates. Tauri npm packages and their crates arrive in separate npm and Cargo pull requests; when one moves to a new minor release, CI fails on the Tauri version check until both are updated together.

## Development and production

Use `npm run tauri dev` during development. It runs Vite on `http://localhost:1420` with hot module replacement and a debug build of the Rust backend; Rust changes rebuild and restart the app, frontend changes reload in place. Use one application per board: two running instances compete for the serial port.

### Development sandbox

To keep a development session away from your real lab catalog, calibration presets, logs and extracted runtime, run `pwsh -NoProfile -File scripts\start_dev_sandbox.ps1`.

- **Redirected data.** It starts `npm run tauri dev` with `LOCALAPPDATA` and the debug-only `WVU_STUDIO_DEV_LOCAL_DATA` pointed at `.dev-sandbox\`, or at `-SandboxRoot <absolute path>`. On first use it points the Project folder at `<sandbox>\recordings`.
- **Not redirected.** WebView2's own browser data stays in the normal place.
- **Release builds.** They ignore `WVU_STUDIO_DEV_LOCAL_DATA`.
- **Starting fresh.** Delete the folder; Git ignores `.dev-sandbox/`.
- **CI placeholder runtime.** With the CI placeholder `arduino-runtime.zip`, the Board card (or, with Source = Hardware, the message under the recording bar) reports that the Arduino tools need attention, and firmware restore cannot work. The Simulator, labs, calibration and recordings work normally.

### Driving and screenshotting the window

`-RemoteDebuggingPort 9222` (any port from 1024 to 65535) also enables WebView2 remote debugging. Any Chrome DevTools Protocol client can then attach to the running window:

- `http://127.0.0.1:9222/json` lists the page target and its `webSocketDebuggerUrl`. Chrome's `chrome://inspect` page can also attach to it.
- `Runtime.evaluate` runs JavaScript in the page, for example to click **Start recording** (`#start-recording-button`) for a Simulator run.
- `Emulation.setDeviceMetricsOverride` sets a viewport size, for example the 1366 × 690 used for the README screenshots (the Session setup image is a 1351 × 847 crop of a full-page capture). The override ends when the DevTools connection closes, so set it in the same session as the screenshot.
- `Page.captureScreenshot` saves a PNG. With `captureBeyondViewport` and a clip from `Page.getLayoutMetrics`, it captures the full page for cropping.

When you refresh the README screenshots:

- Use the Simulator, and say in the caption that the data are simulated.
- Do not publish personal paths: replace sandbox paths in displayed text with the example Project folder `C:\Users\Student\Documents\BMEG 420L` before capturing.
- The Board card needs real hardware and the real runtime; it is described in text until it can be captured on hardware.

The DevTools port gives full control of the page; use it only on a development machine.

### Production build

Use `npm run tauri build` for a production executable and installers. It runs the frontend production build and packages the configured local `build` directory into Tauri. Do **not** treat `cargo build --release` as a production application build: it does not package the frontend assets and can open a localhost error page.

`src-tauri/build.rs` embeds the source commit (`git rev-parse HEAD`) as `GIT_COMMIT`, which firmware restore logs record as `git_revision`. An explicit `GIT_COMMIT` environment variable takes precedence. Outside a Git work tree, or without `git`, the value is simply absent (`null`); the build never fails for that reason. The commit does not show uncommitted changes, so build releases from a clean tree.

## Repository layout

- `src/routes/+page.svelte`: the single window. It owns state and polling and wires the components together.
- `src/lib/components/`: Svelte components (recording bar, message center, setup cards, board panel, live signals, result card, dialogs), with `*.svelte.test.ts` component tests.
- `src/lib/*.ts`: pure TypeScript modules for messages, readiness, board state, plots, calibration, and storage, tested by the Vitest `*.test.ts` files beside them. [Architecture](ARCHITECTURE.md#frontend-structure) lists them.
- `src-tauri/src/`: Rust acquisition, protocol, serial lease, lab catalog, calibration, runtime, and reference-firmware workflows.
- `firmware/reference_unor4wifi/`: controlled UNO R4 WiFi reference firmware.
- `profiles/`: immutable factory course-lab definitions (`*.profile.json`, compiled in through `FACTORY_LAB_FILES` in `profiles.rs`) and `factory-labs.lock.json`, the append-only record of every factory lab version shipped (see [Factory labs](#factory-labs)).
- `src-tauri/examples/factory_labs.rs`: the factory lab file tool behind `scripts/update_factory_labs.ps1` (`check`, `fix`, `promote`).
- `docs/`: user, instructor, maintainer, protocol, and release documentation.
- `src-tauri/nsis/`: the NSIS template the setup is built from, its unmodified upstream copy, and a README (see [Re-syncing the NSIS template](#re-syncing-the-nsis-template)).
- `src-tauri/icons/`: only the icons `tauri.conf.json` `bundle.icon` lists (the Windows build uses `icon.ico`). If you regenerate them with `npm run tauri icon`, delete the Android, iOS and Microsoft Store (`Square*Logo.png`, `StoreLogo.png`) outputs it adds.
- `scripts/build_student_release.ps1`: canonical distribution build script (PowerShell 7).
- `scripts/check_release_consistency.ps1`: version, firmware and runtime metadata check used by CI and the release script.
- `scripts/check_environment.ps1`: read-only check of the development prerequisites.
- `scripts/start_dev_sandbox.ps1`: `npm run tauri dev` with disposable app data.
- `scripts/update_factory_labs.ps1`: checks, re-hashes, or promotes factory lab files (PowerShell 7 and 5.1).
- `src-tauri/src/testdata/op_amp_2.0.1_imported.lab.json`: the Op Amp Lab 2.0.1 that the retired v1.0.2 student installer embedded, byte for byte, for the hash and migration tests (see [Op Amp Lab migration](#op-amp-lab-migration)).

## Factory labs

The factory (built-in) course labs are `profiles/*.profile.json`, compiled into the application by `FACTORY_LAB_FILES` in `src-tauri/src/profiles.rs` and verified against their canonical SHA-256 hash at startup. The hash covers the parsed lab, not the file's layout. Version policy P2 keeps factory and local versions apart:

- A factory lab version is always `MAJOR.MINOR.0`; any change to a factory lab's content bumps MINOR (`X.Y+1.0`), and a published version never changes.
- A local revision always has PATCH ≥ 1 (`next_revision_version`), so it never has a factory version's ID and version.
- `required_firmware.build` is the minimum firmware build whose features the lab uses, and must be on the lab-firmware allow-list in `profiles.rs`. Labs that do not use a new build's features keep their build and hash.
- `profiles/factory-labs.lock.json` is the append-only list of every factory lab version shipped (`profile_id`, `profile_version`, `canonical_hash`).

Rust tests require every compiled-in lab to be in the lock with its hash, every `profiles/*.profile.json` to be compiled in, every factory lab to be `MAJOR.MINOR.0`, `built_in` and `locked`, no lock entry to repeat an ID and version, every lock entry to be held by a file in `profiles/` or `profiles/legacy/`, and the hashes of the labs v1.0.2 shipped to stay unchanged (the golden guard for the canonical serialization, read from the current or legacy file). `fix` and `promote` keep a replaced published version in `profiles/legacy/<file>_<version>.profile.json`; commit it with the lab file and the lock. Tests that need General Analog's version read it from the compiled-in lab (`general_default_version()`), so a new factory version does not break them. When you add a serde field to a lab type, give it a default and `skip_serializing_if` so existing labs keep their hashes and catalogs written by 1.0.x still load.

Change a factory lab only with `scripts/update_factory_labs.ps1` (`check`, `fix <file> [--bump-minor]`, `promote <exported.lab.json>`); [Lab Configuration](LAB_CONFIGURATION.md#how-to-change-a-built-in-lab) walks through both workflows with examples. `check` exits with code 1 on any problem; the Rust test `the_repository_profiles_folder_passes_the_factory_lab_check` runs the same check in CI.

## Op Amp Lab migration

Op Amp Lab is a factory lab (`profiles/op_amp_course_capture.profile.json`, category `course_op_amp`, appended last in `FACTORY_LAB_FILES`). The v1.0.2 student installer (`install_op_amp_lab_profile.bat`/`.ps1`, its ZIP, and `package_op_amp_lab_installer.ps1`) and its CI steps were removed. `.gitattributes` still checks out `*.bat`, `*.cmd` and `*.ps1` with CRLF on every host.

That installer stored Op Amp Lab as General Analog 2.0.1. `ProfileStore::with_root` runs startup migration `op_amp_builtin_v1` (`migrate_legacy_op_amp_lab` in `profiles.rs`) while it holds the catalog lock, never from `sync_with_disk`. It retires that revision when it is active and has one of the two published hashes, makes General Analog's course default active, keeps a `lab_state.pre-migration-<time>.json` copy, and reports `CatalogStatus.migration_notices`, which the Lab card shows in both modes (`catalogMigrationNotice` in `src/lib/lab-catalog.ts`; it never feeds the "Some labs could not be loaded" banner). The catalog's additive `applied_migrations` list records it (decided when a catalog is read, by the migration itself, or by an instructor's Import lab of the copy; never by a write while the copy is still selected and not migrated); [Lab Configuration](LAB_CONFIGURATION.md#catalog-file) gives the exact rules. Do not edit `src-tauri/src/testdata/op_amp_2.0.1_imported.lab.json`: the tests pin its hashes (`c73cc1f2…` as stored, `bcc08868…` as an instructor revision) and seed catalogs with it. A further one-time migration follows the same pattern: a new name in `applied_migrations`, run from `apply_startup_migrations`, with a backup before the write.

The Simulator gives `course_op_amp` its own signals: Vmid near 2.5 V with a few millivolts of deterministic noise and Vout a 1 Hz sine around it.

## Re-syncing the NSIS template

The NSIS setup is built from `src-tauri/nsis/installer.nsi`, not from the template inside the Tauri CLI. It is the CLI's template plus regions between `; WVU-LEGACY-PUBLISHER BEGIN` and `; WVU-LEGACY-PUBLISHER END` lines that let a 1.1 or later setup upgrade a 1.0.x install, which was published as `wvu`. [`src-tauri/nsis/README.md`](../src-tauri/nsis/README.md) explains the regions and records the upstream version, source, and SHA-256. The MSI uses Tauri's own WiX template.

The bundler keeps using this copy after a CLI update, so the consistency check stops with "Tauri CLI changed: re-sync src-tauri/nsis/installer.nsi from the new upstream template" when `@tauri-apps/cli` or `@tauri-apps/cli-win32-x64-msvc` (whose binary embeds the template) in `package-lock.json`, or in `node_modules` when installed, differs from the recorded version. A pull request that updates the CLI, including one from Dependabot, needs the re-sync before CI passes. After updating the CLI:

1. Download the new upstream template over `installer.upstream.nsi`, unchanged. The CLI's release tag is `tauri-cli-v<version>`:

   ```powershell
   $version = (Get-Content node_modules\@tauri-apps\cli\package.json -Raw | ConvertFrom-Json).version
   $url = "https://raw.githubusercontent.com/tauri-apps/tauri/tauri-cli-v$version/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi"
   Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile src-tauri\nsis\installer.upstream.nsi
   (Get-FileHash src-tauri\nsis\installer.upstream.nsi -Algorithm SHA256).Hash.ToLowerInvariant()
   ```

2. Confirm that it is the template the installed CLI embeds. The Windows CLI binary stores it with CRLF line endings. This must print `True`; if it does not, find the tag that matches the CLI build before going on.

   ```powershell
   $latin1 = [Text.Encoding]::GetEncoding(28591)
   $binary = $latin1.GetString([IO.File]::ReadAllBytes('node_modules\@tauri-apps\cli-win32-x64-msvc\cli.win32-x64-msvc.node'))
   $template = $latin1.GetString([IO.File]::ReadAllBytes('src-tauri\nsis\installer.upstream.nsi')).Replace("`r`n", "`n")
   $binary.Contains($template.Replace("`n", "`r`n")) -or $binary.Contains($template)
   ```

3. In the table in `src-tauri/nsis/README.md`, update the CLI version, the tag and its commit, the source URL, and the SHA-256 printed in step 1.
4. Read what changed upstream: `git diff src-tauri/nsis/installer.upstream.nsi`.
5. Copy the new `installer.upstream.nsi` over `installer.nsi`, then put each marked region from the previous version (`git show HEAD:src-tauri/nsis/installer.nsi`) back at the same place. Check each upstream change near them: the `MANUPRODUCTKEY` define, the MSI search loop in `PageReinstall`, the uninstall branch of `PageLeaveReinstall`, and `RestorePreviousInstallLocation`. If upstream changed what a region relies on, adapt the region. Keep `ClearErrors` and the second `UninstallString` read as the last lines of the `PageLeaveReinstall` region: the failed read of the new publisher key sets NSIS's error flag, and upstream's `${Errors}` check after `ExecWait` would otherwise report a successful 1.0.x uninstall as "Unable to uninstall". Change nothing outside the regions, and never write `{{` in a comment: both files are Handlebars templates.
6. Run `pwsh -NoProfile -File scripts\check_release_consistency.ps1`. It compares `installer.nsi` with the upstream copy outside the regions and requires the 1.0.x fallbacks and the error-flag reset to be present.
7. Run `npm run tauri build -- --bundles nsis` to compile the template with makensis. The CI placeholder runtime is enough for that; delete `src-tauri\target\release\bundle\nsis` afterwards, because that setup must not be distributed.
8. Before the next release, repeat the upgrade tests in the [Release Checklist](RELEASE_CHECKLIST.md#upgrade-from-the-previous-release).

## Bundled Arduino runtime

The release build requires `src-tauri/resources/arduino-runtime.zip`, which is intentionally not tracked because it contains third-party binary tooling. Its version is pinned by `arduino-runtime-manifest.json`; the release script verifies the reviewed archive SHA-256 and the reference-firmware SHA-256 before it stages artifacts. Tauri also validates resource existence during Cargo source checks. In CI, the workflow creates a disposable placeholder ZIP for those checks; a fresh local clone needs that same source-check preparation or the real archive. The placeholder cannot be used for firmware restore or student packaging. A maintainer must supply the reviewed real archive with its upstream notices and verify the manifest before building a student installer. Do not download or substitute “latest” Arduino tooling during a release build.

Run `scripts/audit_bundled_runtime_notices.ps1` against the pinned archive before publishing an installer. It verifies the reviewed Arduino CLI license copy, the component-specific BOSSA license, required runtime components, and the retained LICENSE/COPYING/NOTICE inventory. A maintainer must still review upstream redistribution requirements when any bundled version changes.

`scripts/build_student_release.ps1` requires PowerShell 7 and a Git work tree. It stops on the first failed native command (including `git`), runs the consistency check, reinstalls npm packages with `npm ci` (network access or a warm npm cache), builds with `--locked`, and stages only installers created by that invocation. Do not reuse or manually copy an older installer after a failed release build. It builds the NSIS setup (the primary student installer) and an MSI package for managed IT deployment, one after the other. In `dist\`, the student folder's `SHA256SUMS.txt` lists the two installers and is identical to the copy inside the ZIP; `<zip>.sha256` beside the ZIP holds the ZIP's checksum; and `WVU-Bioinstrumentation-Studio-<version>-build-info.txt` records the source commit and tool versions. All checksum files use the `sha256sum` layout with LF line endings.

## Firmware

The application restores only `firmware/reference_unor4wifi/reference_unor4wifi.ino`. Production external commands are launched by the Rust backend with hidden Windows-console creation and captured output. Keep the D4/D5/D6 safe-LOW and pulse-ox mutual-exclusion invariants intact. Protocol 0.4 records firmware-measured timestamps for every synchronized record, advances pulse-state deadlines absolutely, and uses bounded raw-batch and nonblocking transmit queues; do not move packet encoding, CRC construction, or serial writes back into either acquisition timing path.

Any edit to the `.ino`, even a comment, changes its SHA-256. That hash is pinned in `src-tauri/src/reference_firmware.rs`, in `release/release-manifest.json`, and in exact-source tests, and `check_release_consistency.ps1` compares them. A functional change also needs a new firmware build ID, matching `required_firmware` in the factory profiles, a bench test, and a Restore on every lab board. Keep [USB Protocol](USB_PROTOCOL.md) in step with `protocol.rs` and the sketch. Firmware changes waiting for such a build are listed under deferred decisions in the [handoff](AGENT_HANDOFF.md#deferred-decisions).
