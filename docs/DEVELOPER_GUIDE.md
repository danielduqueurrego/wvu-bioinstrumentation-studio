# Developer Guide

For resuming development after a computer reset or transferring work to another agent, start with the [Maintainer and Agent Handoff](AGENT_HANDOFF.md). It covers local assets, catalog recovery, the current branch and release baseline, open hardware checks, and deferred decisions. [Architecture](ARCHITECTURE.md) describes the code structure; [USB Protocol](USB_PROTOCOL.md) is the byte-level protocol reference.

## Prerequisites

Development is Windows-first. Install:

- Git and Node.js 22 or later with npm (`package.json` `engines`; CI uses Node 22).
- Rust through rustup with the MSVC host. `rust-toolchain.toml` pins the exact Rust version with rustfmt and Clippy; rustup installs it the first time cargo runs in the repository (or run `rustup toolchain install`). Change that pin deliberately and rerun every check, because a new Clippy can fail `-D warnings` on unchanged code.
- The Visual Studio C++ build tools (MSVC) with a Windows 10/11 SDK, and the WebView2 runtime: the [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows).
- PowerShell 7.2 or later (`pwsh`) for `scripts/build_student_release.ps1`, `build_arduino_runtime.ps1`, `build_third_party_sources.ps1` and `check_third_party_sources.ps1`. The other scripts also run in Windows PowerShell 5.1.
- Go (for example `winget install GoLang.Go`) only to build the third-party source bundles; see [Third-party source bundles](#third-party-source-bundles).

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

`.github/workflows/ci.yml` runs on Windows with the pinned toolchain and a Rust build cache, and runs the checks above plus `scripts/check_third_party_sources.ps1`, which validates the third-party source manifests without downloading anything. Third-party actions are pinned to commit SHAs, with the release tag in a comment.

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
- **Real runtime.** The runtime goes to `<sandbox>\WVU-BMEG\rt`, never elsewhere. The compiler needs that path to be shorter than 60 characters (see [Runtime folder and the compiler's path limit](#runtime-folder-and-the-compilers-path-limit)); the default `.dev-sandbox` under a typical repository path is longer, so use `-SandboxRoot C:\wvu-sbx` to restore firmware from the sandbox.

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
- `scripts/build_student_release.ps1`: canonical distribution build script (PowerShell 7.2).
- `scripts/check_release_consistency.ps1`: version, firmware and runtime metadata check used by CI and the release script.
- `scripts/build_arduino_runtime.ps1`, `scripts/build_third_party_sources.ps1`, `scripts/check_third_party_sources.ps1`, `scripts/audit_bundled_runtime_notices.ps1`: the offline Arduino runtime, its corresponding-source bundles and its notices (see [Bundled Arduino runtime](#bundled-arduino-runtime)). `release/arduino-runtime-entries.json`, `release/third-party-sources*.json` and `release/arduino-runtime-4.inventory.txt` are their inputs and records.
- `src-tauri/examples/runtime_smoke.rs`: offline compile test of the bundled runtime.
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

The release build requires `src-tauri/resources/arduino-runtime.zip`, which is intentionally not tracked because it contains third-party binary tooling. The reviewed archive is **runtime revision 4**: Arduino CLI `1.6.0`, Arduino UNO R4 core `1.6.0`, SHA-256 `5baa8a5fc7fc0c2ae7fd9f49de1876d76859067af6d56e9b56546a88a5d02def` (193,000,375 bytes). `src-tauri/resources/arduino-runtime-manifest.json` (and its copy in `src-tauri/resources/arduino-runtime/`) names those versions, and `release/release-manifest.json` pins the archive SHA-256; the release script refuses any other archive. `release/arduino-runtime-4.inventory.txt` records what the archive holds and how it was built. Tauri also validates resource existence during Cargo source checks. In CI, the workflow creates a disposable placeholder ZIP for those checks; a fresh local clone needs that same source-check preparation or the real archive. The placeholder cannot be used for firmware restore or student packaging. Do not download or substitute "latest" Arduino tooling during a release build.

`release/arduino-runtime-entries.json` lists what the archive must contain (required entries, every tool and platform folder, license files with their SHA-256) and must not contain (the removed openocd and dfu-util, per-machine Arduino CLI files). `scripts/build_student_release.ps1`, `scripts/audit_bundled_runtime_notices.ps1`, `scripts/build_arduino_runtime.ps1` and `scripts/check_third_party_sources.ps1` all read it. Run the audit against the pinned archive before publishing an installer:

```powershell
pwsh -NoProfile -File scripts\audit_bundled_runtime_notices.ps1 -ArchivePath src-tauri\resources\arduino-runtime.zip
```

It also checks that `src-tauri/resources/licenses/BUNDLED_COMPONENTS.txt` names every bundled version and the source-request form, and that `bossac-LICENSE.txt` is the reviewed BOSSA license. A maintainer must still review upstream redistribution requirements when any bundled version changes.

### Runtime folder and the compiler's path limit

The application extracts the archive to `%LOCALAPPDATA%\WVU-BMEG\rt` (`arduino_runtime.rs`). The bundled GCC 7-2017q4 fails with a missing header, such as `bits/c++config.h: No such file or directory` (`bits/c++allocator.h` and `bits/cpu_defines.h` were also seen), when the runtime root has 60 or more characters: measured on 2026-11-18, 59 compiles and 60 fails. That root is 35 characters plus the Windows profile folder name, so profile folders longer than 24 characters (for example `<20 characters>.CAMPUS`) hit it, with revision 3 (1.0.x) as well. `RUNTIME_ROOT_LENGTH_LIMIT` records the limit, and `resolve_runtime_root` handles a longer root:

- It uses `%ProgramData%\WVU-BMEG-<id>\rt` instead, where `<id>` is the first 12 hex digits of the SHA-256 of the user's SID (39 characters with `C:\ProgramData`). The folder is created with a protected access list (the user, SYSTEM, Administrators), because `%ProgramData%`'s inherited permissions would let every local user add files to another user's runtime, for example a `platform.local.txt` that changes the compile commands. An existing folder is used only if it is owned by the user (or SYSTEM or Administrators), has a protected access list whose allow entries name only those three accounts, and is not a link; the folder is then checked to be writable. A revision-3 copy left in `%LOCALAPPDATA%\WVU-BMEG\rt` by 1.0.x (about 700 MB) is not deleted; IT can remove it. Each user's folder sits directly in `%ProgramData%`, so no other account owns a folder above it.
- If that is not possible, preparation fails with `RuntimeError::PathTooLong`. The page shows **The Arduino tools cannot be set up for this Windows account** (category `runtime_path_too_long`, `ARDUINO_TOOLS_FAILURE_MESSAGES` in `src/lib/messages.ts`) and asks the student to contact IT support; Repair is not offered, because it extracts to the same folder.
- There is no 8.3 short-path step: Arduino CLI expands a short path back to the long one before it runs GCC (`runtime_smoke` with a 63-character root given as `C:\WVU-SM~2\WVULON~1\rt` still failed the same way).
- **Known limitation: characters outside the Windows ANSI code page.** The MinGW-built GCC opens files through the ANSI code page. Measured on 2026-11-18 with `runtime_smoke` on a code page 1252 computer: a runtime root containing `Zoë` (in code page 1252) compiled; a runtime root containing `张伟` failed with `arm-none-eabi-g++: error: C:\...\??\rt\...\variants\UNOWIFIR4/defines.txt: Invalid argument`, and so did a short ASCII runtime root whose scratch folder (the job folder and Arduino's build cache) contained `张伟` (`...\sketch\reference_unor4wifi.ino.cpp.merged: Invalid argument`). Moving only the runtime to `%ProgramData%` would therefore not help: the app's job folder (`%LOCALAPPDATA%\WVU Bioinstrumentation Studio\firmware\logs\jobs`) and Arduino CLI's build cache (`%LOCALAPPDATA%\arduino\sketches`) are under the profile too. The app has no fallback for such a profile; firmware restore fails with that compiler error ([Troubleshooting](TROUBLESHOOTING.md#arduino-tools-cannot-be-set-up-for-this-windows-account)).
- The debug-only `WVU_STUDIO_DEV_LOCAL_DATA` override is never redirected: the runtime stays in `<override>\WVU-BMEG\rt`, and a root of 60 or more characters only logs `ARDUINO_RUNTIME_ROOT_TOO_LONG_DEV`. The default sandbox (`<repository>\.dev-sandbox`) is usually that long; for a firmware restore from the sandbox, start it with a short `-SandboxRoot`, such as `C:\wvu-sbx`.

### Offline smoke test of the runtime

`src-tauri/examples/runtime_smoke.rs` extracts the archive into a scratch folder with the application's own code (`prepare_from_archive`, which applies the native-USB core shim and writes the application's `arduino-cli.yaml`), then compiles the reference sketch with `ArduinoCli::compile_to`, the application's compile command, while HTTP(S) goes to a dead proxy. LOCALAPPDATA, APPDATA, TEMP and TMP point into the scratch folder, so the Arduino build cache and the application log stay there; it never uploads and never touches `%LOCALAPPDATA%\WVU-BMEG\rt`.

```powershell
cargo run --manifest-path src-tauri\Cargo.toml --example runtime_smoke -- C:\wvu-smoke
```

The runtime goes to `C:\wvu-smoke\rt`; keep the scratch folder short, because a runtime root of 60 or more characters is refused. `--root <folder>` puts the runtime elsewhere: a long scratch folder with a short `--root` reproduces the `%ProgramData%` fallback (long job and build-cache paths, short runtime root). It prints the Arduino CLI and core versions, the `.ino` SHA-256, flash and RAM use, and exits 0 only if the sketch compiled and nothing was downloaded. On 2026-11-18 revision 4 compiled the `f3eb3fec…` sketch in about 7 s: flash 58,644 bytes (22%), RAM 11,712 bytes (35%). Delete the scratch folder afterwards (about 700 MB).

### Rebuilding the Arduino runtime

`scripts/build_arduino_runtime.ps1` (PowerShell 7.2 or later; about 3 GB of work space) builds a new archive from pinned inputs: the Arduino CLI release ZIP (SHA-256 pinned and checked against the release's checksum file), `arduino:renesas_uno@1.6.0` from the signed package index, and the gnu.org GPL-2.0 and LGPL-2.1 texts (SHA-256 pinned). It pins every tool version Arduino CLI installs, including the built-in discovery, monitor and ctags tools that Arduino CLI takes from the current package index, and stops on any difference, because a new tool needs a license and source review. It compiles the reference sketch offline three times (all tools, without openocd and dfu-util, and from the finished ZIP) at a 59-character runtime root, writes a deterministic ZIP, its `.sha256` and `arduino-runtime-<revision>.inventory.txt`, and compares the ZIP with `release/arduino-runtime-entries.json` (enforced when that file describes the same revision).

```powershell
pwsh -NoProfile -File scripts\build_arduino_runtime.ps1 -OutputZip C:\wvu-rtout\arduino-runtime.zip -BundleRevision 5 -DownloadCache C:\wvu-arduino-downloads
```

- The default work folder is `C:\wvu-rtbuild`. The script marks it (`.wvu-runtime-build`), and `-Force` empties only a marked folder. `-DownloadCache` must be outside it.
- `-KeepUnusedTools` keeps openocd and dfu-util. `-CheckDeterminism` writes the ZIP twice and compares.
- `-OutputZip` must be outside the repository: review the inventory and notices, then copy the archive to `src-tauri\resources\arduino-runtime.zip` yourself.
- Two builds are not byte-identical: Arduino CLI writes the core's `installed.json` with its tools in random order, and the indexes change over time. The reviewed archive, not the script, is what a release pins.

After a rebuild, in one change: bump `bundle_revision` (so installed copies extract the new runtime) in both runtime manifests, update `arduino_cli_version` and `arduino_runtime_archive_sha256` in `release/release-manifest.json`, `release/arduino-runtime-entries.json`, `release/third-party-sources.json` (with a new source bundle), `BUNDLED_COMPONENTS.txt`, [Third-Party Notices](THIRD_PARTY_NOTICES.md), the inventory under `release/`, and the handoff's irreplaceable-asset hash. Then run the audit, `runtime_smoke`, the full check list and a hardware restore.

### Third-party source bundles

The runtime holds GPL and LGPL programs, so every release publishes their corresponding source beside its installers. `release/third-party-sources.json` describes the bundle for the current runtime (`WVU-Bioinstrumentation-Studio-1.1.0-third-party-sources`) and `release/third-party-sources-runtime3.json` the one for 1.0.0 to 1.0.2 (`WVU-Bioinstrumentation-Studio-1.0.x-third-party-sources`). Each lists every component with its version, SPDX license, paths in the runtime, source URL, SHA-256, Git commit, published checksum, and the bundle's README text: the project description, build directions, license references, and, for the OpenOCD and dfu-util builds whose exact source only Arduino holds, what the bundle contains instead.

`scripts/build_third_party_sources.ps1` (PowerShell 7.2, Go, network access) downloads each source into its own folder, verifies the pinned SHA-256, published checksums and the commit in each GitHub archive, vendors the Go modules of Arduino CLI and the discovery tools with `go mod vendor` and `go mod verify`, cross-checks the shipped Go executables' build information against the vendored modules (`-Runtime`), and writes `<bundle>.zip` (one wrapper folder, sorted entries, fixed timestamps) and `<bundle>.zip.sha256`:

```powershell
pwsh -NoProfile -File scripts\build_third_party_sources.ps1 -Manifest release\third-party-sources.json -OutputDir C:\wvu-srcout -Runtime src-tauri\resources\arduino-runtime.zip
pwsh -NoProfile -File scripts\build_third_party_sources.ps1 -Manifest release\third-party-sources-runtime3.json -OutputDir C:\wvu-srcout
```

- **Pins.** Downloads are pinned by SHA-256. A Go vendor tree is pinned by content, not by ZIP bytes: `vendor.go_version`, the module and file counts, `files_sha256` (the SHA-256 of one line `<file SHA-256>  <path under vendor/>` per file, `modules.txt` excluded, sorted by path) and `modules_txt_sha256`. Go writes the vendor tree and `modules.txt`, so the script sets `GOTOOLCHAIN` to the manifest's `go_toolchain` (`go1.27.4`); a Go 1.21 or later of another version downloads exactly that release, verified by the checksum database. The ZIP bytes depend on .NET's Deflate encoder: two runs with the same PowerShell and .NET produce identical ZIPs (checked on 2026-11-18 with PowerShell 7.7.0 and .NET 11.0.0), but another .NET may not, so the bundle's own `.sha256` is what users check.
- **New pins.** After adding a component or changing a version, leave its `sha256` or `vendor` values empty, review the downloads, and run once with `-RecordHashes`: it fills only empty values and never replaces a pinned one.
- **Folders.** The default work folder is `C:\wvu-srcbuild` (marked `.wvu-third-party-sources`; a non-empty unmarked folder is refused). Each run uses its own subfolder; a successful run deletes its downloads and Go caches and keeps `logs\` and `build-log.txt` unless `-KeepRunFolder`. `-Clean` deletes earlier run folders. `-DownloadCache` and `-GoModCache` reuse verified downloads across runs.
- **Checks without downloads.** `-ValidateOnly` checks one manifest. `scripts/check_third_party_sources.ps1`, which CI and the release script run, validates both, requires every pin, and checks that the current manifest covers the runtime: the versions and archive SHA-256 in the runtime and release manifests, every tool and platform folder and root file in `release/arduino-runtime-entries.json`, and every removed tool. `-VerifyBundle <bundle>.zip` checks a finished bundle against its manifest, read only and without Go: the wrapper folder and entry names, every file against `SHA256SUMS.txt`, the manifest copy, every component's pinned SHA-256, every vendor ZIP's content pins, and no other file. Run it on a bundle before publishing it, for example the 1.0.x bundle.
- **Releases.** `scripts/build_student_release.ps1` builds the bundle for the release version into `dist\`, or checks one given with `-ThirdPartySourcesZip` (its `.sha256`, then `-VerifyBundle`). The 1.0.x bundle is not attached to the 1.0.0-1.0.2 releases; doing so changes published releases and needs the maintainer's explicit go-ahead.

### Release script

`scripts/build_student_release.ps1` requires PowerShell 7.2 and a Git work tree. It stops on the first failed native command (including `git`), runs the consistency check, checks the runtime archive against `release/arduino-runtime-entries.json` and runs the notice audit, validates the third-party source manifests and builds (or, with `-ThirdPartySourcesZip`, verifies) the source bundle, reinstalls npm packages with `npm ci` (network access or a warm npm cache), builds with `--locked`, and stages only installers created by that invocation. Do not reuse or manually copy an older installer after a failed release build. It builds the NSIS setup (the primary student installer) and an MSI package for managed IT deployment, one after the other. In `dist\`, the student folder's `SHA256SUMS.txt` lists the two installers and is identical to the copy inside the ZIP; `<zip>.sha256` beside the ZIP holds the ZIP's checksum; `WVU-Bioinstrumentation-Studio-<version>-third-party-sources.zip` and its `.sha256` sit beside the ZIP (not inside it: about 280 MB); and `WVU-Bioinstrumentation-Studio-<version>-build-info.txt` records the source commit, tool versions, runtime and source-bundle hashes. All checksum files use the `sha256sum` layout with LF line endings.

## Firmware

The application restores only `firmware/reference_unor4wifi/reference_unor4wifi.ino`. Production external commands are launched by the Rust backend with hidden Windows-console creation and captured output. Keep the D4/D5/D6 safe-LOW and pulse-ox mutual-exclusion invariants intact. Protocol 0.4 records firmware-measured timestamps for every synchronized record, advances pulse-state deadlines absolutely, and uses bounded raw-batch and nonblocking transmit queues; do not move packet encoding, CRC construction, or serial writes back into either acquisition timing path.

Any edit to the `.ino`, even a comment, changes its SHA-256. That hash is pinned in `src-tauri/src/reference_firmware.rs`, in `release/release-manifest.json`, and in exact-source tests, and `check_release_consistency.ps1` compares them. A functional change also needs a new firmware build ID, matching `required_firmware` in the factory profiles, a bench test, and a Restore on every lab board. Keep [USB Protocol](USB_PROTOCOL.md) in step with `protocol.rs` and the sketch. Firmware changes waiting for such a build are listed under deferred decisions in the [handoff](AGENT_HANDOFF.md#deferred-decisions).
