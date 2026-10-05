# Maintainer and agent handoff

Snapshot checked on **2026-10-05** before the development computer is formatted. This document transfers project context; inspect current Git and source before acting on a later request.

## Start here

Read this document, the root [AGENTS.md](../AGENTS.md), and the [Developer Guide](DEVELOPER_GUIDE.md). Ask the maintainer for the private recovery archive if the computer has already been formatted. A GitHub clone alone does not recover the uncommitted student installer, local instructor catalog, or ignored Arduino runtime archive.

The previous conversation is not required. Current source, manifests, logs, and the maintainer's next request are the evidence to use.

## Verified repository and release state

| Item | Snapshot |
| --- | --- |
| Repository | <https://github.com/danielduqueurrego/wvu-bioinstrumentation-studio> |
| Branch | `master` |
| Local HEAD and remote master | `94c4bef7561cba3914a673c2c6a6f4bdaadf5b83` — `Fix synchronized acquisition timing` |
| Existing remote release | [v1.0.2](https://github.com/danielduqueurrego/wvu-bioinstrumentation-studio/releases/tag/v1.0.2) |
| `v1.0.0` commit | `03d13129b5b5e637cc1c2d4cd53710bb54f1e499` |
| `v1.0.1` commit | `7e729364c61fd9487de928faff0d43e42ec9d5f3` |
| `v1.0.2` commit | `94c4bef7561cba3914a673c2c6a6f4bdaadf5b83` |
| Application version | `1.0.2` in package, Cargo, Tauri, and release metadata |
| Controlled firmware | Protocol `0.4`, build `0x00010006`, device ID `0x554E4F34` |
| Arduino tools | Arduino CLI `1.5.2-rc.1`, Renesas UNO core `1.6.0`, runtime bundle revision `3` |
| First-party license | Apache-2.0; root `LICENSE` and package license metadata exist |

The tag commit IDs above are the peeled commits of annotated tags. They are not the tag-object IDs. Do not recreate or move an existing tag unless the maintainer explicitly asks for that operation.

The README includes the reviewed AI-assisted-development disclosure and the teaching-use notice. Third-party licenses remain separate; see [Third-Party Notices](THIRD_PARTY_NOTICES.md). There is currently no root `NOTICE`; do not infer that it was lost during recovery. The maintainer previously confirmed ownership; do not invent a different institutional or personal copyright owner.

### Uncommitted work to preserve

At the beginning of this handoff, the only pending changes were:

- `docs/STUDENT_QUICK_START.md`: instructions for the student Op Amp Lab installer.
- `scripts/install_op_amp_lab_profile.ps1`: standalone, per-user profile installer.
- `scripts/install_op_amp_lab_profile.bat`: double-click launcher for that script.
- `scripts/install_op_amp_lab_profile.zip`: existing distribution copy of the student scripts.

This handoff adds `AGENTS.md`, this document, and links/corrections in README and the Developer Guide. These changes have not been committed or pushed by the handoff task. The private recovery archive preserves the working files and Git history. Do not discard the pending changes when making the next focused change.

## Files that must survive formatting

| Data | Location relative to the repository or current Windows user | Recovery importance |
| --- | --- | --- |
| Source, docs, scripts, pending edits | Repository working tree | Recover from the private snapshot plus Git bundle |
| Real offline Arduino runtime | `src-tauri/resources/arduino-runtime.zip` | Ignored by Git; required for genuine firmware restore and release packaging |
| Instructor lab catalog | `%LOCALAPPDATA%\WVU Bioinstrumentation Studio\profiles\lab_state.json` | Local labs are not automatically uploaded to GitHub |
| Catalog backups and audit logs | Same `profiles` directory | Preserve with the catalog |
| Calibration presets | `%LOCALAPPDATA%\WVU Bioinstrumentation Studio\calibrations\*.json` | User-specific; preserve if present |
| Project-folder preference | `%LOCALAPPDATA%\WVU Bioinstrumentation Studio\project-folder.json` | May need a new folder selection if the Windows username/path changes |
| Application log | `%LOCALAPPDATA%\WVU Bioinstrumentation Studio\logs\application.log` | Diagnostic evidence, including rotated files if present |
| Restore provenance | `%LOCALAPPDATA%\WVU Bioinstrumentation Studio\firmware\logs\firmware_job_*.json` and `jobs/` | Records actual compiled source hash, upload result, and verification |
| Recordings | Selected Project folder plus relative Output folder; default `%USERPROFILE%\Documents\BMEG 420L` | Preserve `.bmeg`, `.csv`, metadata, markers, and partial recordings together |
| Older local test evidence | Repository `recordings/` and `logs/` | Ignored by Git; retained privately in the recovery archive when present |
| Extracted Arduino tools | `%LOCALAPPDATA%\WVU-BMEG\rt` | Recreated by the app from the preserved archive; do not transplant an old absolute-path CLI configuration |
| Published installers | GitHub v1.0.2 release or local `dist/` | Recovery archive does not need the multi-gigabyte set of historical installers |

The private archive is named `WVU-Studio-Recovery-2026-10-05-verified.zip` and is created beside the repository in the Windows Documents folder. Its adjacent `.sha256.txt` records its checksum. **Copy both files to an external drive or confirmed cloud storage before formatting. A backup on the same computer will be erased too.**

The archive contains `repository.bundle`, `source/`, `local-assets/arduino-runtime.zip`, `user-data/app-local/`, selected local recording/evidence folders under `user-data/`, `START_HERE.md`, and `MANIFEST.json`. The manifest lists exactly what was captured and each file's SHA-256. It is a private recovery backup, not a student release asset: it contains local logs and recordings. Other Project folders, Downloads attachments, other users' AppData, credentials, `.codex` conversation history, and historical `dist/` binaries are not automatically captured. Back up any additional folders that matter separately.

## Recover on the newly installed computer

1. Copy the archive and checksum from the external/cloud backup. Verify the archive SHA-256, then extract it to a recovery folder using Explorer or `Expand-Archive`.
2. Install Git, Node.js/npm, Rust with the Windows MSVC toolchain, Microsoft C++ build tools with a Windows SDK, WebView2, and PowerShell 7. Follow the official [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows). CI uses Node `22` and Rust stable; keep `package-lock.json` and `src-tauri/Cargo.lock`.
3. Recover Git from the bundle, then overlay the saved working files. Example, with the extracted archive in `D:\WVU-Recovery`:

```powershell
git clone -b master D:\WVU-Recovery\repository.bundle "$env:USERPROFILE\Documents\wvu-bioinstrumentation-studio"
if ($LASTEXITCODE -ne 0) { throw 'Git recovery failed' }
Set-Location "$env:USERPROFILE\Documents\wvu-bioinstrumentation-studio"
git remote set-url origin https://github.com/danielduqueurrego/wvu-bioinstrumentation-studio.git
Get-ChildItem -LiteralPath D:\WVU-Recovery\source -Force | Copy-Item -Destination . -Recurse -Force
Copy-Item -LiteralPath D:\WVU-Recovery\local-assets\arduino-runtime.zip -Destination src-tauri\resources\arduino-runtime.zip
git status --short
git log -1 --oneline
```

Overlay only into the new recovered clone. The `source/` snapshot includes hidden directories such as `.github` but does not contain `.git`, build caches, or authentication secrets. Cloning the bundle preserves the existing commits/tags; overlaying the snapshot restores uncommitted work. Alternatively, clone GitHub and verify the base before applying the snapshot. Inspect any later remote changes before merging them with saved edits.

4. With the app closed, copy the **contents** of `user-data/app-local/` to `%LOCALAPPDATA%\WVU Bioinstrumentation Studio` for the new Windows user. Restore recording folders to the desired Documents/project location. If the new account already has a catalog or presets, back it up and merge deliberately; do not blindly overwrite it. The app's Import feature can transfer individual exported labs instead.
5. Select the new Project folder in the app. Old absolute paths in preferences and historical logs may refer to the former Windows account. Let the app prepare a fresh `%LOCALAPPDATA%\WVU-BMEG\rt`; its native-USB core shim is applied automatically.
6. Authenticate GitHub and the coding agent again through their normal sign-in methods if required. No credentials are included in the recovery archive.

The environment observed before formatting was Node `24.19.0`, npm `11.17.0`, Rust/Cargo `1.97.1`, PowerShell `7.6.5`, and Visual Studio Community with C++ tools. These are an observation of this machine, not additional package-version pins. The release script uses `ConvertFrom-Json -AsHashtable` and should run with PowerShell 7. The student lab installer deliberately supports Windows PowerShell 5.1.

### Verify the irreplaceable asset

The real archive was checked against `release/release-manifest.json` on the handoff date:

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

With the preserved real runtime copied into place, run from the repository root:

```powershell
npm ci
npm run check
npm run build
npm test
cargo fmt --manifest-path src-tauri\Cargo.toml -- --check
cargo check --manifest-path src-tauri\Cargo.toml
cargo test --manifest-path src-tauri\Cargo.toml
cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets -- -D warnings
npm run tauri build
```

Check every command's exit code. In a multi-command PowerShell script, use `$ErrorActionPreference = 'Stop'` and explicitly check `$LASTEXITCODE` after each native executable; a later success can otherwise mask an earlier failure.

The production executable is `src-tauri/target/release/wvu_bioinstrumentation_studio.exe`; the default Tauri bundle is NSIS under `src-tauri/target/release/bundle/nsis/`. `frontendDist` is `../build`; development alone uses `http://localhost:1420`. Validate the production executable with no Vite server running. `mainBinaryName` and Cargo `default-run` explicitly select the GUI app so a diagnostic binary cannot become the installer target.

For an official distribution, read [Release Checklist](RELEASE_CHECKLIST.md), then run `pwsh -NoProfile -File scripts/build_student_release.ps1`. That script checks versions/hashes/notices, runs verification, builds NSIS and MSI serially, stages `dist/WVU-Bioinstrumentation-Studio-<version>/`, and generates the distribution ZIP/checksums. Its normal path requires a clean tree; `-AllowDirty` is for an explicitly reviewed temporary candidate. Never call an old artifact final after a failed build.

CI uses separate Windows steps, builds frontend output before tests, and creates a tiny **CI-only** placeholder runtime ZIP before Cargo source checks. A plain clean clone needs either that source-check preparation or the real archive even for `cargo check/test/clippy`, because Tauri validates resource existence. The placeholder must never be accepted for production packaging. See [.github/workflows/ci.yml](../.github/workflows/ci.yml).

No application test totals or new hardware acceptance are claimed by this documentation task; consult actual verification output when performing the next change.

## Source map for the next request

| Area | Main files |
| --- | --- |
| Single-window student UI, polling, Start/Stop, Board, Project folder | `src/routes/+page.svelte` |
| Live plot lifecycle, isolation/retry, custom legends and endpoint overlays | `src/lib/components/LivePlot.svelte` |
| Plot groups, color mapping, visibility | `src/lib/multichannel.ts` |
| Global time window, startup display warmup, endpoint formatting/layout | `src/lib/live-display.ts` |
| Readiness, duration, lab authoring | `src/lib/recording-start.ts`, `board-controls.ts`, `duration.ts`, `lab-authoring.ts`, `components/LabManager.svelte` |
| Tauri command boundary and startup | `src-tauri/src/lib.rs`, `main.rs` |
| Worker lifecycle, serial diagnostics, cancellation, storage, display history | `src-tauri/src/session.rs`, `acquisition.rs` |
| Protocol framing, CRC, sequences, per-record timestamps | `src-tauri/src/protocol.rs`, [USB Protocol](USB_PROTOCOL.md) |
| BMEG/CSV/metadata and finalization | `src-tauri/src/recording.rs` |
| Profiles, local versions, transactional catalog | `src-tauri/src/profiles.rs`, factory `profiles/*.profile.json` |
| Calibration/display conversion and presets | `src/lib/calibration.ts`, `src-tauri/src/calibration.rs` |
| Project/Output path validation | `src-tauri/src/project_paths.rs`, `src/lib/project-folder.ts` |
| Restore, CLI process flags, runtime deployment | `src-tauri/src/firmware_workflow.rs`, `arduino_cli.rs`, `arduino_runtime.rs`, `reference_firmware.rs` |
| Controlled sketch | `firmware/reference_unor4wifi/reference_unor4wifi.ino` |
| Release pins and packaging | `release/release-manifest.json`, `src-tauri/tauri.conf.json`, `scripts/build_student_release.ps1` |

### Accepted behavior to preserve

- Header, Board, Project folder, Acquisition remain in one window. Board Refresh/Verify/Restore availability is independent of acquisition firmware readiness; Restore is a recovery path for unknown/outdated firmware.
- `UntilStopped` is a real enum variant with no requested seconds. Legitimate terminal paths include User, Disconnect, ApplicationClose, StorageGuard, and actual acquisition faults. Critical free storage is 250 MiB; warning is 1 GiB. Samples that cease for seven seconds trigger a diagnosed fault, not an automatic restart.
- The firmware uses RA4M1 native USB CDC. The compile recipe deliberately omits `NO_USB`; `arduino_runtime.rs` adds the version-pinned `SerialUSB::write_raw` compatibility shim in the app-owned extracted core. Do not revert to the ESP32 bridge or patch an unrelated Arduino IDE installation.
- Pulse-ox phase deadlines advance absolutely with `nextTickMicros += stateDwellUs`. ADC/record creation is isolated from frame encoding, CRC, and serial transmission using bounded queues and partial writes. Host recording retains actual per-record timestamps and per-state timing checks.
- Raw ADC counts remain authoritative in BMEG/CSV. One synchronized 0.5-30 s live window (default 5 s), up to 30,000 retained display frames, and at most 2,000 requested display points keep plotting bounded. Timestamp filtering/decimation always includes the exact newest frame. Display changes must not issue acquisition commands.
- Multi-signal legends appear only with at least two visible signals in that plot; endpoint values appear for each visible trace in its selected unit. Plot exceptions are isolated and logged as `PLOT_RENDER_FAIL`. The time axis is elapsed recording seconds, not a calendar date. Hardware startup warmup is display-only; the initial raw sample remains in the recording.
- MPXV uses `(Vout / Vsupply - 0.04) / 0.009` kPa and `kPa * 7.5006` mmHg. EMG pressure supports counts/volts/kPa/mmHg. XGZP uses its selected local linear calibration. Pulse ox remains raw-only, with no ambient subtraction or physiological computations.
- Preserve hidden subprocess creation, the GUI Windows subsystem, offline runtime deployment, precise Advanced details, serial ownership cleanup, and transactional catalog writes.

## Recent issues and evidence

### Historical acquisition stalls

Intermittent stalls were reproduced on Home and Work computers with the same UNO and cable, including PING write timeouts, serial read failures, and a seven-second no-data timeout while the COM identity could remain listed. Early ADC-blocking hypotheses were diagnostic candidates, not established causes. The current implementation documents the native-USB change to avoid the intermittent RA4M1-to-ESP32 bridge path. The maintainer subsequently reported stable behavior and accepted releases. The current sketch has no temporary D13 stage-heartbeat or diagnostic transport-only mode.

If a later stall recurs, use the actual installed executable version, restore-job source hash, terminal stage/error kind/Windows error, last sample/PONG/PING timestamps, metadata, and post-fault enumeration. Distinguish host faults from physical device removal. Do not change duration, increase the no-data timeout, or restart/append a failed recording merely to conceal the symptom.

### New boards: upload bootloader unavailable

On 2026-09-23, new-board restore attempts compiled successfully but BOSSA returned `No device found on COM9` after the 1200-bps touch. Other attempts on new ports showed the same error. This is an upload/bootloader communication failure; the source did not prove a compilation or duration defect.

Suggested recovery was closing other serial applications/Arduino Cloud Agent, double-pressing the physical RESET button, refreshing the board selection, then restoring again. For an existing native-USB WVU sketch the app explicitly waits for the double reset to return the upload interface. Upload bridge PID is `0x1002`; WVU native application PID is `0x006D`, and COM numbers/USB serial identity can change across that transition. Use only one UNO for recovery to avoid ambiguous selection. Do not reflash ESP32 connectivity firmware without a separately justified task.

Later local restore logs provide positive evidence: `firmware_job_12_20260923_154231.json`, `firmware_job_1_20260923_161951.json`, and `firmware_job_1_20260930_151855.json` report stage `complete`, compatible verification, and the current `f3eb...` source hash. The last job transitions COM34 to COM35. This does not establish which physical step resolved every earlier attempt; no automatic fallback fix was committed. Inspect those backed-up logs if the maintainer asks about new-board recovery again.

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

Instructor canonical hash: `bcc08868305ca55e81920e3a5805b020cfa612a0374e75fd5a290fa11f918ad9`. The standalone student installer embeds the same profile with `source=imported`, as the application's Import operation does; its resulting hash is `c73cc1f2941910696d88f934b7b4c09b8b110438a4886d7d799b13807f61471d`. That source-field change explains the different hash.

The `.bat` and `.ps1` must stay together. Double-clicking the `.bat` requires no elevation and installs/activates version 2.0.1 for the current Windows user. The PowerShell script preserves unrelated labs, backs up an existing catalog before a change, rejects conflicting same-ID/version content, verifies the embedded hash, and refuses installation while the app runs. Previous checks covered Windows PowerShell 5.1, PowerShell 7, idempotence, conflict rejection, backup/preservation, and launcher exit codes. Explicit UTF-8 catalog reads and escaped non-ASCII JSON preserve hash consistency in PowerShell 5.1.

The instructor profile is not a factory definition in Git. Recover the private AppData catalog or use the embedded installer; do not assume a fresh clone contains all local instructor labs. Test changes against a temporary `-CatalogRoot`, not the maintainer's live catalog.

## Opening request for a future agent

The maintainer can paste:

> Work in my recovered wvu-bioinstrumentation-studio repository. Read AGENTS.md and docs/AGENT_HANDOFF.md, inspect current Git state, and preserve the pending student profile installer and local catalogs. My new request is: [describe the task]. Use current source and logs as evidence, and follow the documented production build and hardware safety invariants.
