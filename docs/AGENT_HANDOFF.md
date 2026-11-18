# Maintainer and agent handoff

Snapshot checked on **2026-11-18**. This document transfers project context. It is a dated snapshot, not a permanent statement about HEAD: inspect the current Git state and source before acting on a later request.

## Start here

Read this document, the root [AGENTS.md](../AGENTS.md), and the [Developer Guide](DEVELOPER_GUIDE.md); [Architecture](ARCHITECTURE.md) maps the code. A GitHub clone alone does not recover the local instructor catalog, the ignored real Arduino runtime archive, or unpushed branches (see below). Ask the maintainer for the private recovery archive when those are needed.

The previous conversation is not required. Current source, manifests, logs, and the maintainer's next request are the evidence to use.

## Verified repository and release state

| Item | Snapshot |
| --- | --- |
| Repository | <https://github.com/danielduqueurrego/wvu-bioinstrumentation-studio> |
| `master` and `origin/master` | `537be977b1ca8e8e41554841c544c8e7f4a05256` — `update README.md and add AGENTS.md and AGENT_HANDOFF.md` (unchanged) |
| Working branch | `review-fixes`, created from `537be97`, **local only, not pushed** |
| Existing remote release | [v1.0.2](https://github.com/danielduqueurrego/wvu-bioinstrumentation-studio/releases/tag/v1.0.2) |
| `v1.0.0` commit | `03d13129b5b5e637cc1c2d4cd53710bb54f1e499` |
| `v1.0.1` commit | `7e729364c61fd9487de928faff0d43e42ec9d5f3` |
| `v1.0.2` commit | `94c4bef7561cba3914a673c2c6a6f4bdaadf5b83` |
| Application version | `1.0.2` in package, Cargo, Tauri, and release metadata (unchanged on `review-fixes`) |
| Controlled firmware | Protocol `0.4`, build `0x00010006`, device ID `0x554E4F34`; the `.ino`, its SHA-256, the manifest, and `profiles/*.json` are unchanged on `review-fixes` |
| Arduino tools | Arduino CLI `1.5.2-rc.1`, Renesas UNO core `1.6.0`, runtime bundle revision `3` |
| Rust toolchain | `1.100.0`, pinned in `rust-toolchain.toml` |
| First-party license | Apache-2.0; root `LICENSE` and package license metadata exist |

`review-fixes` holds one commit per phase of the 2026-11-17 whole-project review, oldest first, on top of `537be97`:

| Commit | Summary |
| --- | --- |
| `16b3bd3` | Make recording finalization unconditional and keep the recording tail |
| `c70c1d1` | Keep the window responsive and make board/firmware operations safe |
| `b2b864e` | Harden the lab catalog, lab authoring, CSV names and project paths |
| `a86905e` | Redesign the student UI and fix frontend state bugs |
| `bc5f359` | Fix confirmations in Lab Manager, sweep review nits, update dependencies |
| `b422dd3` | Harden CI, release scripts and the student installer |

This handoff, the documentation refresh, and the new README screenshots were written after `b422dd3`. `git log 537be97..HEAD` shows the commit messages, which summarize each phase; [CHANGELOG.md](../CHANGELOG.md) groups the changes under **Unreleased**.

Nothing has been pushed, merged, tagged, or released. **Hardware verification is still pending** (see the [checklist](#hardware-verification-checklist-for-review-fixes)). Push, merge, retag, or publish only when the maintainer asks.

The tag commit IDs above are the peeled commits of annotated tags, not the tag-object IDs. Do not recreate or move an existing tag unless the maintainer explicitly asks for that operation.

The README includes the reviewed AI-assisted-development disclosure and the teaching-use notice. Third-party licenses remain separate; see [Third-Party Notices](THIRD_PARTY_NOTICES.md). There is currently no root `NOTICE`; do not infer that it was lost during recovery. The maintainer previously confirmed ownership; do not invent a different institutional or personal copyright owner.

### This computer

This computer was set up for the review work:

- **Toolchain.** It has Node 22, Rust `1.100.0` through rustup and `rust-toolchain.toml` (with rustfmt and Clippy), and the Visual Studio 2022 Build Tools (MSVC with a Windows SDK). `npm ci` has been run.
- **Runtime archive.** `src-tauri/resources/arduino-runtime.zip` currently holds the **CI placeholder** (269 bytes), created with the procedure in `.github/workflows/ci.yml`. It satisfies Cargo source checks only.
  - Before any hardware test or release build, replace it with the real hash-pinned ZIP from the private recovery archive. That archive is not on this computer; [Verify the irreplaceable asset](#verify-the-irreplaceable-asset) gives the hash.
  - The file is ignored by Git; never commit, package, or test hardware with the placeholder.
- **Existing app data.** The existing real app data on this computer was left untouched by the review work: `%LOCALAPPDATA%\WVU Bioinstrumentation Studio`, including firmware logs from August 2026 and `project-folder.json`, and `%LOCALAPPDATA%\WVU-BMEG\rt`.
- **UI testing.** All UI testing used `scripts\start_dev_sandbox.ps1`, with data in the Git-ignored `.dev-sandbox\` folder, the Simulator, and the placeholder runtime. The sandbox therefore shows "Arduino tools need attention" and has no board.

## Hardware verification checklist for review-fixes

None of these can run without the real runtime ZIP and an Arduino UNO R4 WiFi. Use one application and one serial owner at a time. Record the result of each, with the app build and the restore-job source hash.

**Recordings**

- [ ] An ECG (no-output) lab recording is complete to the last sample: there are no missing sequences, and `total_samples` matches the CSV rows.
- [ ] **Stop recording** during a timed run saves a complete recording (stop reason `user`).
- [ ] Unplugging the board mid-recording ends **Faulted**, with a finalized partial recording and its metadata, and the **Last recording** card shows it.
- [ ] The first Start after the idle-STOP change works, including on a board left acquiring by a previous session (no stale records at the head of the file).
- [ ] The laptop stays awake during a 10-minute recording on battery (`powercfg /requests` lists the app while it records).
- [ ] The window stays responsive during Verify, Restore, and Start.

**Board operations**

- [ ] Verify Firmware while Arduino IDE (Serial Monitor) holds the port reports the busy port and keeps the earlier result: the chip reads **Port busy** on a board not yet verified; a board that was **Ready on COMx** keeps that chip, and **Last firmware operation** (plus the message center, with Source = Hardware) says **Another program is using the board**.
- [ ] Restore with two UNO R4 WiFi boards connected is refused.
- [ ] Cancel is unavailable once the upload starts.
- [ ] The double-reset countdown works on a board already running the native-USB WVU firmware.

**Outputs**

- [ ] D4/D5/D6 read LOW after Stop, after a fault, and after the 5 s watchdog (for example, end the app's process mid-recording).
- [ ] Pulse-ox RED/IR timing passes the [Release Checklist](RELEASE_CHECKLIST.md) pulse-ox criteria, and RED and IR are never HIGH together.

**Docs**

- [ ] Retake the README Board screenshot on hardware with the real runtime, then add it back to README's **Board** section. `docs/images/board-and-firmware.png` was removed because it predated the redesign.
- [ ] (No hardware needed.) After `review-fixes` is merged to `master`, open the README on GitHub and check its four images. README links them as absolute `raw.githubusercontent.com/.../master/docs/images/...` URLs (the maintainer chose that form in `03d1312` "Fix README image links"). Until the merge, `recording-result.png` returns 404 and the other three show master's pre-redesign screenshots.

## Files that must survive formatting

| Data | Location relative to the repository or current Windows user | Recovery importance |
| --- | --- | --- |
| Source, docs, scripts | Repository working tree and Git history | `master` is on GitHub. **`review-fixes` exists only on this computer** until it is pushed or bundled. |
| Real offline Arduino runtime | `src-tauri/resources/arduino-runtime.zip` | Ignored by Git; required for genuine firmware restore and release packaging. **Currently the CI placeholder on this computer.** |
| Instructor lab catalog | `%LOCALAPPDATA%\WVU Bioinstrumentation Studio\profiles\lab_state.json` | Local labs are not automatically uploaded to GitHub |
| Catalog backups and audit logs | Same `profiles` directory (`lab_state.reset-*.json`, `lab_state.corrupt-*.json`, audit logs) | Preserve with the catalog |
| Calibration presets | `%LOCALAPPDATA%\WVU Bioinstrumentation Studio\calibrations\*.json` | User-specific; preserve if present |
| Project-folder preference | `%LOCALAPPDATA%\WVU Bioinstrumentation Studio\project-folder.json` | May need a new folder selection if the Windows username/path changes |
| Application log | `%LOCALAPPDATA%\WVU Bioinstrumentation Studio\logs\application.log` | Diagnostic evidence, including rotated files if present |
| Restore provenance | `%LOCALAPPDATA%\WVU Bioinstrumentation Studio\firmware\logs\firmware_job_*.json` and `jobs/` | Records actual compiled source hash, upload result, and verification |
| Recordings | Selected Project folder plus relative Output folder; default `Documents\BMEG 420L` | Preserve `.bmeg`, `.csv`, `.metadata.json`, and any `.bmeg.part` files together |
| Older local test evidence | Repository `recordings/` and `logs/` | Ignored by Git; retained privately in the recovery archive when present |
| Extracted Arduino tools | `%LOCALAPPDATA%\WVU-BMEG\rt` | Recreated by the app from the archive; do not transplant an old absolute-path CLI configuration |
| Development sandbox | Repository `.dev-sandbox\` | Disposable test data; not needed for recovery |
| Published installers | GitHub v1.0.2 release or local `dist/` | The recovery archive does not need the multi-gigabyte set of historical installers |

Before formatting again, preserve `review-fixes`:

- Push it when the maintainer authorizes, or
- Create a bundle, for example `git bundle create ..\wvu-studio-review-fixes.bundle --all`, and copy it off the computer with the real runtime ZIP.

### The 2026-10-05 recovery archive

The private archive `WVU-Studio-Recovery-2026-10-05-verified.zip`, with its adjacent `.sha256.txt` checksum, was made before the previous reformat. It contains:

- `repository.bundle`
- `source/`
- `local-assets/arduino-runtime.zip`
- `user-data/app-local/`
- Selected local recording and evidence folders under `user-data/`
- `START_HERE.md`
- `MANIFEST.json`, which lists each captured file and its SHA-256

It is a private backup, not a student release asset, because it contains local logs and recordings. It does not capture other Project folders, Downloads, other users' AppData, credentials, conversation history, or historical `dist/` binaries.

That archive **predates** both `537be97` and `review-fixes`:

- Its `repository.bundle` was made while HEAD was `94c4bef`.
- Its `source/` snapshot holds the then-uncommitted installer and handoff work, which has since been committed in `537be97`.

For source, use GitHub (plus a bundle of `review-fixes` if it was not pushed). Use the archive only for `local-assets/arduino-runtime.zip` and `user-data/`. Never overlay its `source/` folder onto a newer clone: that would silently revert later commits.

## Recover on a newly installed computer

1. Copy the recovery archive and its checksum from the external or cloud backup. Verify the archive SHA-256, then extract it to a recovery folder.
2. Install the toolchain:
   - Git, Node.js 22 with npm, Rust through rustup with the Windows MSVC toolchain, the Microsoft C++ build tools with a Windows SDK, WebView2, and PowerShell 7. Follow the official [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows).
   - Rustup installs the version pinned in `rust-toolchain.toml` on first use.
   - Keep `package-lock.json` and `src-tauri/Cargo.lock`.
   - `scripts/check_environment.ps1` reports anything missing.
3. Clone the repository and restore the local assets. Example, with the extracted archive in `D:\WVU-Recovery`:

```powershell
git clone https://github.com/danielduqueurrego/wvu-bioinstrumentation-studio.git "$env:USERPROFILE\Documents\wvu-bioinstrumentation-studio"
if ($LASTEXITCODE -ne 0) { throw 'Clone failed' }
Set-Location "$env:USERPROFILE\Documents\wvu-bioinstrumentation-studio"
# Only if review-fixes was bundled instead of pushed:
# git fetch D:\path\to\wvu-studio-review-fixes.bundle review-fixes:review-fixes
Copy-Item -LiteralPath D:\WVU-Recovery\local-assets\arduino-runtime.zip -Destination src-tauri\resources\arduino-runtime.zip
git status --short
git log -1 --oneline
```

4. With the app closed, copy the **contents** of `user-data/app-local/` to `%LOCALAPPDATA%\WVU Bioinstrumentation Studio` for the new Windows user. Restore recording folders to the desired Documents or Project location. If the new account already has a catalog or presets, back them up and merge deliberately; do not blindly overwrite them. The app's Import feature can transfer individual exported labs instead.
5. Select the new Project folder in the app. Old absolute paths in preferences and historical logs may refer to the former Windows account. Let the app prepare a fresh `%LOCALAPPDATA%\WVU-BMEG\rt`; its native-USB core shim is applied automatically.
6. Authenticate GitHub and the coding agent again through their normal sign-in methods if required. No credentials are included in the recovery archive.

The release script requires PowerShell 7 (`#Requires -Version 7.0`). The student lab installer deliberately supports Windows PowerShell 5.1.

### Verify the irreplaceable asset

The real archive was checked against `release/release-manifest.json` on 2026-10-05:

```text
Arduino runtime ZIP SHA-256:
4e068c540b485e8e7ceff29dac849db250b41efb56108b10edadfb033dc7d501

Controlled reference .ino SHA-256:
f3eb3fec0166189f66a8ddfeb058172a8d97a3c60d95d574b3234eecbd144aab
```

```powershell
Get-FileHash src-tauri\resources\arduino-runtime.zip -Algorithm SHA256
Get-FileHash firmware\reference_unor4wifi\reference_unor4wifi.ino -Algorithm SHA256
```

Do not substitute a CI placeholder, download newer tools, or change the reviewed manifest merely to accept a different archive. Changes to firmware or bundled dependencies require a new review of matching identity, hashes, protocol/profile compatibility, and notices.

## Build and check

From the repository root (the real runtime is required for `npm run tauri build`, hardware tests, and release packaging; the CI placeholder is enough for the other checks):

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
npm run tauri build
```

Check every command's exit code. In a multi-command PowerShell script, use `$ErrorActionPreference = 'Stop'` and explicitly check `$LASTEXITCODE` after each native executable; a later success can otherwise mask an earlier failure. Build the frontend before `npm test`: a release-configuration test needs `build/index.html`.

**Production executable.**

- The executable is `src-tauri/target/release/wvu_bioinstrumentation_studio.exe`.
- The default Tauri bundle is NSIS, under `src-tauri/target/release/bundle/nsis/`.
- `frontendDist` is `../build`; only development uses `http://localhost:1420`. Validate the production executable with no Vite server running.
- `mainBinaryName` and Cargo `default-run` explicitly select the GUI app, so a diagnostic binary cannot become the installer target.

**Official distribution.** Read the [Release Checklist](RELEASE_CHECKLIST.md), then run `pwsh -NoProfile -File scripts/build_student_release.ps1`. That script:

- Checks versions, hashes, and notices
- Runs verification
- Builds NSIS and MSI serially
- Stages `dist/WVU-Bioinstrumentation-Studio-<version>/`
- Generates the distribution ZIP and checksums

Its normal path requires a clean tree; `-AllowDirty` is for an explicitly reviewed temporary candidate. Never call an old artifact final after a failed build.

**CI.** It uses separate Windows steps, builds the frontend before tests, and creates a tiny **CI-only** placeholder runtime ZIP before the Cargo source checks. A clean clone needs either that placeholder or the real archive even for `cargo check/test/clippy`, because Tauri validates that resources exist. The placeholder must never be accepted for production packaging. See [.github/workflows/ci.yml](../.github/workflows/ci.yml).

For UI work without hardware, use `scripts\start_dev_sandbox.ps1` (see [Developer Guide](DEVELOPER_GUIDE.md#development-sandbox)).

## Source map for the next request

| Area | Main files |
| --- | --- |
| Single-window page: state, polling, Start/Stop, wiring | `src/routes/+page.svelte` |
| Recording bar, message center, setup cards, board panel, restore stepper, result card | `src/lib/components/RecordingBar.svelte`, `MessageCenter.svelte`, `SetupCard.svelte`, `BoardPanel.svelte`, `FirmwareJobProgress.svelte`, `RecordingResultCard.svelte` |
| Plain-language texts and next actions; readiness; Source default | `src/lib/messages.ts`, `readiness.ts`, `recording-start.ts`, `source-select.ts` |
| Live signals, plot lifecycle, legends, endpoint overlays, watermark | `src/lib/components/LiveSignalsPanel.svelte`, `PlotToolbar.svelte`, `LivePlot.svelte` |
| Plot groups, colours, visibility; time window, warmup, ticks | `src/lib/multichannel.ts`, `live-display.ts` |
| Lab card, Lab Manager, lab authoring | `src/lib/components/LabCard.svelte`, `LabManager.svelte`, `OperatingModeControl.svelte`, `src/lib/lab-catalog.ts`, `lab-authoring.ts` |
| Tauri command boundary, startup, window close | `src-tauri/src/lib.rs`, `main.rs` |
| Serial-ownership lease | `src-tauri/src/serial_lease.rs` |
| Worker lifecycle, serial diagnostics, cancellation, storage guard, finalization, display history | `src-tauri/src/session.rs`, `acquisition.rs` |
| Protocol framing, CRC, sequences, per-record timestamps | `src-tauri/src/protocol.rs`, [USB Protocol](USB_PROTOCOL.md) |
| BMEG/CSV/metadata | `src-tauri/src/recording.rs`, [Calibration and Units](CALIBRATION_AND_UNITS.md#recording-files) |
| Profiles, local versions, transactional catalog | `src-tauri/src/profiles.rs`, factory `profiles/*.profile.json` |
| Calibration conversion and presets | `src/lib/calibration*.ts`, `src/lib/components/CalibrationCard.svelte`, `CalibrationDialog.svelte`, `src-tauri/src/calibration.rs` |
| Project/Output path validation | `src-tauri/src/project_paths.rs`, `src/lib/project-folder.ts` |
| Restore, Verify, CLI process flags, runtime deployment | `src-tauri/src/firmware_workflow.rs`, `arduino_cli.rs`, `arduino_runtime.rs`, `reference_firmware.rs` |
| Controlled sketch | `firmware/reference_unor4wifi/reference_unor4wifi.ino` |
| Release pins, packaging, consistency | `release/release-manifest.json`, `src-tauri/tauri.conf.json`, `scripts/build_student_release.ps1`, `scripts/check_release_consistency.ps1` |

### Accepted behavior to preserve

**Layout and display**

- The header, recording bar, message center, setup cards (Board, Project folder, Lab, Session setup, Recording duration, Calibration & Units), Live signals, and Last recording stay in one window.
- **Source** starts on Simulator (practice). It switches to Hardware **once**, the first time firmware becomes ready on the selected port while nothing is recording. It never switches during or after a recording, or after the student chose a Source. Simulator plots carry the SIMULATED DATA watermark.
- Raw ADC counts remain authoritative in BMEG/CSV. One synchronized 0.5–30 s live window (default 5 s), up to 30,000 retained display frames, and at most 2,000 requested display points keep plotting bounded.
- Timestamp filtering and decimation always include the exact newest frame. Display changes, including Pause display, must not issue acquisition commands.
- Multi-signal legends appear only with at least two visible signals in that plot. Endpoint values appear for each visible trace in its selected unit. Plot exceptions are isolated and logged as `PLOT_RENDER_FAIL`.
- The time axis is elapsed recording seconds, not a calendar date. Hardware startup warmup is display-only; the initial raw sample remains in the recording.

**Board and firmware**

- Board Refresh, Verify, and Restore availability is independent of acquisition firmware readiness; Restore is a recovery path for unknown or outdated firmware.
- Start readiness for Hardware is bound to `verified_port`: it requires a compatible board and a port match, and never consults the last restore. Any failure or job start clears it.
- Verify never uploads firmware. A Verify that cannot open the port reports `PortBusy` and keeps the previous compatibility.
- "Reset board and retry" is retired; recovery is Retry connection, then Verify Firmware, then Restore WVU Firmware.
- Cancel is refused once a restore reaches `Uploading`, and the window cannot close during a firmware job.
- Restore is refused while more than one UNO R4 is connected.
- A native-USB WVU board needs the manual double reset (90 s window); the restore window shows it.

**Concurrency and session state**

- One serial-ownership lease (`serial_lease.rs`) serializes recordings, Verify, Retry, Restore, and tool repair. It is released by `Drop` on every path, so failed operations release serial ownership and busy state.
- Long board operations run in `spawn_blocking`; Tauri command argument names and serde payload names stay aligned with their frontend callers.
- `recording_active` is true only while a Start owns the session. Verify, Retry, and restore-verification handshakes are not recordings.
- `fault_origin` says which operation raised `last_error`.
- `SessionError::Cancelled` is the only quiet worker ending; any other failure after Stop faults with its stage.

**Capture protocol**

- `UntilStopped` is a real enum variant with no requested seconds.
- Legitimate terminal paths include User, TimedComplete, Disconnect, ApplicationClose, StorageGuard, and actual acquisition faults.
- Samples that cease for seven seconds trigger a diagnosed fault, not an automatic restart.
- The host sends an **idle-safe STOP before every CONFIGURE**, so a board left acquiring is quiesced and its outputs go LOW before reconfiguration.
- Frames and ERRORs from before the START STATUS are not counted against the new recording.
- After STOP, the host waits up to 300 ms for a **STATUS newer than STOP** (`acquiring = 0`, mask 0) and drains the tail. A mask that is already 0 proves nothing for labs without outputs.

**Files and storage**

- Finalization is unconditional and crash-safe: BMEG finish and fsync, rename with retry, then **metadata before CSV**, then a best-effort CSV, then the sidecar rewritten with `csv_filename` or `csv_error`.
- `finalize_error` records flush, rename, or metadata problems. Raw data are never deleted, and files of other recordings are never overwritten: the raw `.bmeg.part` uses `create_new`, run names skip existing files and `.part` siblings, and final moves never replace a file. Only the recording's own metadata sidecar is deliberately replaced when it is rewritten with the CSV result.
- The storage guard stops below 250 MiB **plus the projected CSV size**; the warning is at 1 GiB, or earlier by the same margin. A failed free-space query is not fatal.
- Windows is kept awake (`PowerSetRequest`) from the worker's start through finalization.

**Firmware, provenance, and development**

- The firmware uses RA4M1 native USB CDC. The compile recipe deliberately omits `NO_USB`, and `arduino_runtime.rs` adds the version-pinned `SerialUSB::write_raw` compatibility shim in the app-owned extracted core. Do not revert to the ESP32 bridge or patch an unrelated Arduino IDE installation.
- Pulse-ox phase deadlines advance absolutely with `nextTickMicros += stateDwellUs`. ADC and record creation are isolated from frame encoding, CRC, and serial transmission by bounded queues and partial writes. Host recording retains actual per-record timestamps and per-state timing checks.
- MPXV uses `(Vout / Vsupply - 0.04) / 0.009` kPa and `kPa * 7.5006` mmHg. EMG pressure supports counts, volts, kPa, and mmHg. XGZP uses its selected local linear calibration, with at most one active preset per channel. Pulse ox remains raw-only, with no ambient subtraction or physiological computations.
- Tool-version provenance is best effort: versions are cached, and a failed query records `"unavailable"` instead of failing a recording or restore. Restore logs record `git_revision` from the `GIT_COMMIT` embedded at build time, or `null`.
- `WVU_STUDIO_DEV_LOCAL_DATA` relocates the extracted runtime **in debug builds only**; release builds ignore it.
- Preserve hidden subprocess creation (`CREATE_NO_WINDOW`), the GUI Windows subsystem, offline runtime deployment, precise Advanced details, serial ownership cleanup, and transactional catalog writes.

## Recent issues and evidence

### Historical acquisition stalls

Intermittent stalls were reproduced on Home and Work computers with the same UNO and cable. They included PING write timeouts, serial read failures, and a seven-second no-data timeout while the COM identity could remain listed. Early ADC-blocking hypotheses were diagnostic candidates, not established causes.

The current implementation documents the native-USB change to avoid the intermittent RA4M1-to-ESP32 bridge path. The maintainer subsequently reported stable behavior and accepted releases. The current sketch has no temporary D13 stage-heartbeat or diagnostic transport-only mode.

If a stall recurs, use this evidence:

- The actual installed executable version
- The restore-job source hash
- The terminal stage, error kind, and Windows error
- The last sample, PONG, and PING timestamps
- The metadata
- Post-fault enumeration

Distinguish host faults from physical device removal. Do not change the duration, increase the no-data timeout, or restart or append a failed recording merely to conceal the symptom.

### New boards: upload bootloader unavailable

On 2026-09-23, new-board restore attempts compiled successfully, but BOSSA returned `No device found on COM9` after the 1200-bps touch. Other attempts on new ports showed the same error. This is an upload/bootloader communication failure; the evidence did not prove a compilation or duration defect.

Suggested recovery:

1. Close other serial applications and Arduino Cloud Agent.
2. Double-press the physical RESET button.
3. Refresh the board selection.
4. Restore again.

For an existing native-USB WVU sketch, the app explicitly waits for the double reset to return the upload interface. The restore window shows a 90 s countdown. The upload bridge PID is `0x1002`, and the WVU native application PID is `0x006D`. COM numbers and the USB serial identity can change across that transition. Restore now refuses to run with more than one UNO R4 connected. Do not reflash the ESP32 connectivity firmware without a separately justified task.

Three local restore logs provide positive evidence: `firmware_job_12_20260923_154231.json`, `firmware_job_1_20260923_161951.json`, and `firmware_job_1_20260930_151855.json`. Each reports stage `complete`, compatible verification, and the current `f3eb...` source hash. The last job moves from COM34 to COM35. This does not establish which physical step resolved every earlier attempt. Inspect those backed-up logs if the maintainer asks about new-board recovery again.

## Local Op Amp Lab and student installation

The saved local catalog has a locked instructor profile:

```text
ID:       wvu.bmeg420l.general.analog.development.v2
Version:  2.0.1
Name:     Op Amp Lab
Inputs:   A0 (Vmid), A1 (Vout)
Rate/ADC: 100 frames/s, 14 bit
Required: protocol 0.4, firmware build 0x00010006
```

It uses the factory **General Analog — Development** ID, so on a computer where it is active it replaces General Analog in the student Lab list. That ID choice is a deferred decision (below).

**Hashes.** The instructor canonical hash is `bcc08868305ca55e81920e3a5805b020cfa612a0374e75fd5a290fa11f918ad9`. The standalone student installer embeds the same profile with `source=imported`, as the application's Import operation does, which gives the hash `c73cc1f2941910696d88f934b7b4c09b8b110438a4886d7d799b13807f61471d`. That source-field change explains the different hash. A Rust test pins both the factory hashes and the installer's embedded profile hash.

**The installer.** The `.bat` and `.ps1` must stay together. Double-clicking the `.bat` requires no elevation and installs and activates version 2.0.1 for the current Windows user. The PowerShell script:

- Preserves labs with other IDs
- Backs up an existing catalog before a change
- Rejects conflicting same-ID/version content
- Verifies the embedded hash
- Refuses installation while the app runs

Explicit UTF-8 catalog reads and escaped non-ASCII JSON preserve hash consistency in PowerShell 5.1.

**CI and packaging.** CI runs the install, hash, existing-catalog preservation and backup, idempotence, and invalid-catalog checks under Windows PowerShell 5.1, PowerShell 7, and the `.bat` launcher. After editing either script, rebuild the student ZIP with `pwsh -NoProfile -File scripts\package_op_amp_lab_installer.ps1`; CI fails on a stale ZIP.

The instructor profile is not a factory definition in Git. Recover the private AppData catalog or use the embedded installer; do not assume a fresh clone contains all local instructor labs. Test changes against a temporary `-CatalogRoot`, not the maintainer's live catalog.

## Deferred decisions

The 2026-11-17 review recorded these items. They need hardware, legal review, or the maintainer's decision, so this pass did not resolve them.

**Firmware** (any `.ino` edit changes the pinned source SHA-256 in `reference_firmware.rs`, the release manifest, and the exact-string tests. It needs a new build ID, profile `required_firmware` updates, a bench test, and a Restore on every lab board. Bundle these items into one reviewed firmware build.)

- CONFIGURE received mid-acquisition changes the configuration without stopping. The host now sends an idle STOP first; the firmware option is to treat CONFIGURE while acquiring as an implicit stop, and to purge queued sample frames at START.
- Count skipped sampling deadlines in the firmware itself, for example with a dedicated flag or counter. The host currently counts flagged records as `overrun_flagged_records`.
- Pulse-ox settle time after a catch-up: a late state can leave the next state almost no settle time before sampling. A minimum-settle guard is the proposed fix.
- Use a table-driven CRC to shorten batch encoding at short dwells.
- Hardware watchdog, and a `readCommands` loop that cannot spin when `Serial.read()` returns -1.
- Reject any D5/D6 bit in simultaneous-mode CONFIGURE. Today the firmware rejects only both together, and the desktop rejects D5/D6 HIGH-while-recording.
- The `.ino` header comment describes the default course configuration as if pins and timing were fixed.

**Release and legal**

- Corresponding-source obligations for GPL tools in the bundled runtime (Arduino CLI, the gcc toolchain)
- License texts for Rust crates and npm packages in the installers
- The production pin on Arduino CLI `1.5.2-rc.1`, a pre-release
- `SECURITY.md` has no private reporting channel
- Installer publisher and copyright metadata
- A documented rebuild recipe for `arduino-runtime.zip`

**Design**

- The Op Amp Lab reuses the factory General Analog profile ID, so installing it hides General Analog. Its A1 channel also has `csv_name` `01_counts`. Options: document it as intended, re-issue it under its own ID, or ship it as a factory lab.
- Numbering of local revisions versus future factory patches of the same ID
- Non-ASCII Windows usernames in the extracted toolchain path
- A recovery tool for `.bmeg.part` files
- A CSV `elapsed_s` column and a separate markers CSV
- Push versus poll for display data (currently polled)
- `display.voltage_reference_v` in lab definitions is validated but unused. It is documented as informational; seeding the UI Vref from it is an option.
- Stricter startup content hashing of the extracted runtime

**Dependencies**

- `zip` crate 2.x advisories. The fix is a semver-major upgrade; `audit.yml` lists the reviewed ignores.
- Remaining development-tool npm advisories that need semver-major upgrades

## Opening request for a future agent

The maintainer can paste:

> Work in my wvu-bioinstrumentation-studio repository. Read AGENTS.md and docs/AGENT_HANDOFF.md, inspect current Git state (including whether review-fixes has been pushed or merged), and preserve local catalogs, recordings, calibration presets, and the real Arduino runtime. My new request is: [describe the task]. Use current source and logs as evidence, and follow the documented production build and hardware safety invariants.
