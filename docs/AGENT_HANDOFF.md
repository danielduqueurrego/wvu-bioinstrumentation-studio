# Maintainer and agent handoff

Snapshot checked on **2026-11-18**; the firmware build `0x00010007` and pulse-ox 1.1.0 sections were added on **2026-11-19**. This document transfers project context. It is a dated snapshot, not a permanent statement about HEAD: inspect the current Git state and source before acting on a later request.

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
| Controlled firmware | Releases 1.0.0-1.0.2 and `review-fixes`: protocol `0.4`, build `0x00010006`, device ID `0x554E4F34`, `.ino` SHA-256 `f3eb3fec…`. On `release-1.1-prep`: protocol `0.4`, build **`0x00010007`**, same device ID, `.ino` SHA-256 `86221c10…` (see [Firmware build 0x00010007](#firmware-build-0x00010007-and-led-sequenced-pulse-ox)); every board needs one Restore, after every lab PC is updated |
| Arduino tools | Releases 1.0.0-1.0.2: Arduino CLI `1.5.2-rc.1`, Renesas UNO core `1.6.0`, runtime bundle revision `3` (`4e068c54…`). On `release-1.1-prep`: Arduino CLI `1.6.0`, core `1.6.0`, runtime bundle revision `4` (`5baa8a5f…`), without OpenOCD and dfu-util |
| Rust toolchain | `1.100.0`, pinned in `rust-toolchain.toml` |
| First-party license | Apache-2.0; root `LICENSE` and package license metadata exist (root `NOTICE` added on `release-1.1-prep`) |

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

The README includes the reviewed AI-assisted-development disclosure and the teaching-use notice. Third-party licenses remain separate; see [Third-Party Notices](THIRD_PARTY_NOTICES.md). On `release-1.1-prep`, the root [`NOTICE`](../NOTICE) carries the copyright line the maintainer chose on 2026-11-18, "Copyright 2026 West Virginia University Department of Chemical and Biomedical Engineering", which is also the installers' publisher and copyright. `LICENSE` is the unchanged Apache-2.0 text. Byte-identical copies of both, `src-tauri/resources/licenses/LICENSE.txt` and `NOTICE.txt`, install with the app, and `scripts/check_release_consistency.ps1` checks that they still match. The installed third-party notice is `src-tauri/resources/licenses/BUNDLED_COMPONENTS.txt`, with `bossac-LICENSE.txt` beside it. `docs/THIRD_PARTY_NOTICES.md` is a maintainer document that ships in the release folder only and is not installed; this narrows the round-2 plan, which also listed the third-party notices for `resources/licenses/`. The planned runtime-revision-4 work rewrites `BUNDLED_COMPONENTS.txt` as the installed source-and-license directions. Do not invent a different institutional or personal copyright owner.

### This computer

This computer was set up for the review work:

- **Toolchain.** It has Node 22, Rust `1.100.0` through rustup and `rust-toolchain.toml` (with rustfmt and Clippy), and the Visual Studio 2022 Build Tools (MSVC with a Windows SDK). `npm ci` has been run.
- **Runtime archive.** Since 2026-11-18, `src-tauri/resources/arduino-runtime.zip` holds the real **runtime revision 4** (SHA-256 `5baa8a5f…`, 193,000,375 bytes), built on this computer with `scripts/build_arduino_runtime.ps1` (see [Runtime revision 4](#runtime-revision-4-and-the-third-party-source-bundles)). It is the only copy besides the build scratch folders: archive it privately (see [Verify the irreplaceable asset](#verify-the-irreplaceable-asset)).
  - The revision-3 archive of releases 1.0.0-1.0.2 (`4e068c54…`) is not on this computer; it is in the 2026-10-05 recovery archive.
  - The file is ignored by Git; never commit it. A clone without it needs the CI placeholder (`.github/workflows/ci.yml`) for Cargo source checks only; never package or test hardware with the placeholder.
- **Go.** Go 1.27.4 (`C:\Program Files\Go`) is installed for building the third-party source bundles.
- **Existing app data.** The existing real app data on this computer was left untouched by the review work: `%LOCALAPPDATA%\WVU Bioinstrumentation Studio`, including firmware logs from August 2026 and `project-folder.json`, and `%LOCALAPPDATA%\WVU-BMEG\rt`.
- **UI testing.** All UI testing used `scripts\start_dev_sandbox.ps1`, with data in the Git-ignored `.dev-sandbox\` folder, the Simulator, and the placeholder runtime. The sandbox therefore shows "Arduino tools need attention" and has no board. With the real runtime, the default sandbox path is too long for the compiler (see [Runtime revision 4](#runtime-revision-4-and-the-third-party-source-bundles)); use `-SandboxRoot C:\wvu-sbx` for a firmware restore from the sandbox.
- **Disposable build folders (2026-11-18).** None of these is needed for recovery; delete them when the runtime and bundles are archived: `C:\wvu-rtbuild` (about 3 GB, the revision-4 build and earlier source-bundle runs), `C:\wvu-srcbuild` (source-bundle run logs), the empty `C:\wvu-naE` (left by the non-ASCII `runtime_smoke` runs), and the agent session's temporary `groupE`, `groupE2` and `fixE` folders under `%TEMP%\claude` (build logs, download and Go module caches; `fixE\bundles` holds the bundles listed under [Runtime revision 4](#runtime-revision-4-and-the-third-party-source-bundles), `groupE2\bundles-final` the superseded first build). The copy of the revision-4 ZIP in `groupE` is identical to `src-tauri/resources/arduino-runtime.zip`; keep it until the ZIP is archived.

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

**Arduino runtime revision 4 (release-1.1-prep)**

- [ ] With the network disconnected, the first start of a `release-1.1-prep` build extracts revision 4 (it replaces revision 3 once), and Restore WVU Firmware compiles and uploads. The restore log shows Arduino CLI `1.6.0`, core `1.6.0` and the reference `.ino` SHA-256 of `release/release-manifest.json`.
- [ ] On a Windows account whose profile folder name is longer than 24 characters (a disposable local account is enough), the app uses `%ProgramData%\WVU-BMEG-<id>\rt` (`ARDUINO_RUNTIME_ROOT reason=program_data` in `application.log`), the folder's access list shows only that user, SYSTEM and Administrators (`icacls`), and a restore compiles. A second account cannot open the folder.

**Labs (release-1.1-prep)**

- [ ] On the maintainer's own catalog, the first start of a `release-1.1-prep` build retires the Op Amp Lab 2.0.1 copy, leaves a `lab_state.pre-migration-*.json` backup, and shows the migration notice in the Lab card. A second start shows no notice and makes no new backup.
- [ ] The built-in Op Amp Lab records `vmid_counts` and `vout_counts` from A0 and A1 on hardware.

**Firmware build 0x00010007 and LED-sequenced pulse ox (release-1.1-prep)**

- [ ] **Restore every board once.** A board restored by 1.0.2 (build `0x00010006`) shows **Older WVU firmware**; one Restore WVU Firmware (offline, revision 4) makes it **Ready**, and the restore log shows the `.ino` SHA-256 `86221c10…`. Before restoring boards in the lab, **update every lab PC**: a 1.0.2 PC reports a build-7 board as incompatible firmware and would restore build 6.
- [ ] A 1.0.2 PC meeting a build-7 board reports incompatible firmware (expected; it confirms the rollout rule above). If a newer build is ever available, a 1.1 PC meeting it shows **Newer WVU firmware** and no Retry connection.
- [ ] **Built-in pulse ox 1.1.0 at 1 ms.** A 60 s recording has 1000 rows/s within 1%, a median interval near 1000 µs, no interval outside ±5%, `led_state == record_sequence % 4` on every CSV row, `led_state_mismatches` 0, `firmware_skipped_deadlines` 0, `deadline_skip_flagged_records` 0, per-state checks 0, and `minimum_pulse_settle_us` ≥ 500 (≥ 50% of the dwell). TX and RX show the expected RED/DARK/IR/DARK levels with a finger or a bench light source.
- [ ] **A 6-channel LED-sequenced lab** (an instructor lab on A0-A5, 1000 µs; for example a photodetector on A0 and bench voltages on A1-A5) passes the same checks, and every row holds all six channels. Measure the read spacing between channels (the docs estimate about 20 µs per `analogRead`, see [Hardware](HARDWARE.md#analog-inputs)) and correct the docs if it differs.
- [ ] **Logic analyzer on D5 and D6** during a sequenced recording: each LED HIGH for one dwell per four-state cycle in the order RED, DARK 1, IR, DARK 2; D5 and D6 never HIGH together; both LOW after Stop, after a fault, and after the 5 s host-command watchdog.
- [ ] **Legacy 4-state at 1 ms and 250 µs.** Install `profiles/legacy/pulseox_tx_rx_course_capture_1.0.0.profile.json` with **Install lab file…** (1000 µs) and record 60 s: about 250 cycles/s, no cycle interval outside ±5%, per-state checks 0. Then record a revision edited to 250 µs and note the per-state checks, skipped deadlines, and minimum settle (≥ 125 µs). **Restore course default** returns to 1.1.0.
- [ ] **Minimum settle.** In every pulse-ox recording above, `minimum_pulse_settle_us` is at least 50% of the dwell.
- [ ] **Watchdog soak.** A 30-minute six-channel sequenced recording completes with no board reset and no missing sequences, and an idle board connected for 30 minutes does not reset. (Optional: a sketch-level stall test is not part of the reference firmware; do not add a test hook to it.)
- [ ] **CONFIGURE during acquisition.** The first Start after a board was left acquiring (forced app close) is clean, as for build 6.

**Docs**

- [ ] Retake the README Board screenshot on hardware with the real runtime, then add it back to README's **Board** section. `docs/images/board-and-firmware.png` was removed because it predated the redesign.
- [ ] (No hardware needed.) After `review-fixes` is merged to `master`, open the README on GitHub and check its four images. README links them as absolute `raw.githubusercontent.com/.../master/docs/images/...` URLs (the maintainer chose that form in `03d1312` "Fix README image links"). Until the merge, `recording-result.png` returns 404 and the other three show master's pre-redesign screenshots.

## Files that must survive formatting

| Data | Location relative to the repository or current Windows user | Recovery importance |
| --- | --- | --- |
| Source, docs, scripts | Repository working tree and Git history | `master` is on GitHub. **`review-fixes` exists only on this computer** until it is pushed or bundled. |
| Real offline Arduino runtime | `src-tauri/resources/arduino-runtime.zip` | Ignored by Git; required for genuine firmware restore and release packaging. **Revision 4 (`5baa8a5f…`) exists only on this computer: archive it privately.** Keep the revision-3 archive (`4e068c54…`) of 1.0.x too. |
| Third-party source bundles | Rebuilt from `release/third-party-sources*.json` with `scripts/build_third_party_sources.ps1` | Reproducible from the manifests while the upstream URLs last; keep the published bundles with the release records |
| Instructor lab catalog | `%LOCALAPPDATA%\WVU Bioinstrumentation Studio\profiles\lab_state.json` | Local labs are not automatically uploaded to GitHub |
| Catalog backups and audit logs | Same `profiles` directory (`lab_state.reset-*.json`, `lab_state.corrupt-*.json`, `lab_state.pre-migration-*.json`, audit logs) | Preserve with the catalog |
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

The release script and the runtime and source-bundle scripts require PowerShell 7.2 (`#Requires -Version 7.2`); building the source bundles also needs Go. The other scripts also run in Windows PowerShell 5.1.

### Verify the irreplaceable asset

**Runtime revision 4** (`release-1.1-prep`, Arduino CLI 1.6.0, core 1.6.0) was built on 2026-11-18 and is pinned in `release/release-manifest.json`. It cannot be rebuilt byte for byte (Arduino CLI writes the core's `installed.json` in random order, and the indexes change), so **this exact file must be archived privately** with the next recovery archive, together with `release/arduino-runtime-4.inventory.txt`:

```text
Arduino runtime revision 4 ZIP SHA-256 (193,000,375 bytes):
5baa8a5fc7fc0c2ae7fd9f49de1876d76859067af6d56e9b56546a88a5d02def
```

**Runtime revision 3** of releases 1.0.0-1.0.2 was checked against the v1.0.2 `release/release-manifest.json` on 2026-10-05 and is in the 2026-10-05 recovery archive. Keep it: it is the archive those releases shipped, and the 1.0.x source bundle should be cross-checked against it (`-Runtime`) before that bundle is published.

```text
Arduino runtime revision 3 ZIP SHA-256:
4e068c540b485e8e7ceff29dac849db250b41efb56108b10edadfb033dc7d501

Controlled reference .ino SHA-256 (build 0x00010006, releases 1.0.0-1.0.2):
f3eb3fec0166189f66a8ddfeb058172a8d97a3c60d95d574b3234eecbd144aab

Controlled reference .ino SHA-256 (build 0x00010007, release-1.1-prep):
86221c104b8a7cf30bfbb0ac621cfdfebc8b08635ed0710df66db8fa4f5d681d
```

```powershell
Get-FileHash src-tauri\resources\arduino-runtime.zip -Algorithm SHA256
Get-FileHash firmware\reference_unor4wifi\reference_unor4wifi.ino -Algorithm SHA256
pwsh -NoProfile -File scripts\audit_bundled_runtime_notices.ps1 -ArchivePath src-tauri\resources\arduino-runtime.zip
```

Do not substitute a CI placeholder, download newer tools, or change the reviewed manifest merely to accept a different archive. Changes to firmware or bundled dependencies require a new review of matching identity, hashes, protocol/profile compatibility, and notices.

## Runtime revision 4 and the third-party source bundles

**Runtime.** `scripts/build_arduino_runtime.ps1` built revision 4 from the pinned Arduino CLI 1.6.0 release (`2769b9b2…`) and `arduino:renesas_uno@1.6.0`, removed OpenOCD and dfu-util (the UNO R4 WiFi uploads with bossac), and proved an offline compile of the reference sketch three times. The committed script carries the review fixes made after that build (default work folders and a guarded `-Force`, `-KeepUnusedTools`, relative paths, pinned built-in tool versions, `release/arduino-runtime-entries.json`, 59-character proofs); a validation rebuild with it passed all proofs (its ZIP differs, as every rebuild does). `src-tauri/examples/runtime_smoke.rs` compiled the `f3eb3fec…` sketch with the real ZIP offline: flash 58,644 bytes (22%), RAM 11,712 bytes (35%); and later the build-`0x00010007` sketch `86221c10…`: flash about 60.1 kB (22%; 60,088 bytes in one scratch folder, 60,080 in two others, because the binary embeds source paths), RAM 12,344 bytes (37%), most of the increase being the 512-byte CRC table. `cli_config_text` now also writes `updater.enable_notification: false` and `network.cloud_api.skip_board_detection_calls: true`. [Developer Guide](DEVELOPER_GUIDE.md#bundled-arduino-runtime) has the rebuild recipe.

**Path length.** GCC 7-2017q4 fails when the runtime root has 60 or more characters (59 compiles). `%LOCALAPPDATA%\WVU-BMEG\rt` is 35 characters plus the profile folder name, so profile folders longer than 24 characters fail; 1.0.2 has the same problem and no mitigation. `resolve_runtime_root` (`RUNTIME_ROOT_LENGTH_LIMIT` in `arduino_runtime.rs`) then uses `%ProgramData%\WVU-BMEG-<SHA-256 of the user SID, 12 hex>\rt`, created with a protected access list for the user, SYSTEM and Administrators; otherwise the page shows category `runtime_path_too_long` ("contact IT support"). The plan's 8.3 short-path step was dropped: Arduino CLI expands short paths back to the long path before running GCC (tested with `runtime_smoke`). The plan's `%ProgramData%\WVU-BMEG\<id>` layout with default permissions was changed to a per-user folder directly in `%ProgramData%` with a protected access list, because the default permissions let every local user add files (for example a `platform.local.txt` that changes compile commands) inside another user's runtime, and the first user to create `WVU-BMEG` would own the folder above everyone else's. An existing fallback folder is used only if its owner is the user, SYSTEM or Administrators, its access list is protected and its allow entries name only those three accounts, and it is not a link (junction). The debug-only `WVU_STUDIO_DEV_LOCAL_DATA` override is never redirected. The fallback's access-list code is unit-tested in a temporary folder only (creation, an unprotected folder, a protected folder that also allows Users, a junction); it has not run against the real `%ProgramData%` (see the hardware checklist). A profile folder name with characters outside the Windows ANSI code page is a separate, unmitigated limitation (see [Deferred decisions](#deferred-decisions)).

**Source bundles.** `release/third-party-sources.json` (revision 4, `WVU-Bioinstrumentation-Studio-1.1.0-third-party-sources`) and `release/third-party-sources-runtime3.json` (1.0.0-1.0.2) describe them; `scripts/build_third_party_sources.ps1` builds them, `scripts/check_third_party_sources.ps1` (CI) validates the manifests offline, and `build_third_party_sources.ps1 -VerifyBundle` checks a finished bundle offline (the release script runs it on `-ThirdPartySourcesZip`). Bundles rebuilt on 2026-11-18 after the review fixes, which reworded the OpenOCD component of the 1.0.x manifest and the removed-tool notes of the 1.1.0 manifest (identical in two runs each; the 1.1.0 bundle with `-Runtime` against the real revision-4 ZIP; both pass `-VerifyBundle`; not uploaded anywhere):

```text
WVU-Bioinstrumentation-Studio-1.1.0-third-party-sources.zip  SHA-256 a55586d4ddd330e7de3e4dc15d941f542050b85a26e28deee82f25bde28c402a (279,590,658 bytes)
WVU-Bioinstrumentation-Studio-1.0.x-third-party-sources.zip  SHA-256 b97bd6983a3fd449c2d282ba23feb21746808958cf4e9b898677dcb38bd61105 (289,631,695 bytes)
```

These hashes hold only for the committed manifests and PowerShell 7.7.0/.NET 11.0.0; any manifest change, or another .NET, gives a new bundle. The 1.1 notices (`BUNDLED_COMPONENTS.txt` and the 1.1.0 bundle's README) do not refer to the unpublished 1.0.x bundle: for OpenOCD and dfu-util they point to `release/third-party-sources-runtime3.json` and the issue form. The 1.0.x bundle's written offer does not promise source the maintainers do not hold: for OpenOCD 0.11.0-arduino2 and dfu-util 0.11.0-arduino5 it lists what is included (the upstream releases, Arduino's OpenOCD fork at the commit the binary names, Arduino's OpenOCD build scripts with their static libraries), says Arduino holds the exact build sources, and says requests are forwarded to Arduino and anything received is published, for at least three years. The 1.0.x bundle was built without `-Runtime` (only the real extracted `%LOCALAPPDATA%\WVU-BMEG\rt` held revision 3 here, and it was not touched), so its `runtime_archive_sha256` has not been checked against the actual archive yet.

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
- The default Tauri bundle is NSIS, under `src-tauri/target/release/bundle/nsis/`. On `release-1.1-prep` it is built from the custom template `src-tauri/nsis/installer.nsi`; re-sync it on every Tauri CLI update ([Developer Guide](DEVELOPER_GUIDE.md#re-syncing-the-nsis-template)).
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
| Plot groups, colours, visibility, record-to-trace mapping and LED-state traces; time window, warmup, ticks | `src/lib/multichannel.ts`, `live-display.ts` |
| Lab card, Lab Manager, lab authoring | `src/lib/components/LabCard.svelte`, `LabManager.svelte`, `ChannelGridEditor.svelte`, `OperatingModeControl.svelte`, `src/lib/lab-catalog.ts`, `lab-authoring.ts` |
| Tauri command boundary, startup, window close | `src-tauri/src/lib.rs`, `main.rs` |
| Serial-ownership lease | `src-tauri/src/serial_lease.rs` |
| Worker lifecycle, serial diagnostics, cancellation, storage guard, finalization, display history | `src-tauri/src/session.rs`, `acquisition.rs` |
| Protocol framing, CRC, sequences, per-record timestamps | `src-tauri/src/protocol.rs`, [USB Protocol](USB_PROTOCOL.md) |
| BMEG/CSV/metadata | `src-tauri/src/recording.rs`, [Calibration and Units](CALIBRATION_AND_UNITS.md#recording-files) |
| Profiles, local versions, transactional catalog, startup migration `op_amp_builtin_v1` | `src-tauri/src/profiles.rs`, factory `profiles/*.profile.json`, published earlier versions in `profiles/legacy/`, `profiles/factory-labs.lock.json`, `src-tauri/src/testdata/` |
| Factory lab file tool (version policy P2) | `scripts/update_factory_labs.ps1`, `src-tauri/examples/factory_labs.rs`, [Lab Configuration](LAB_CONFIGURATION.md#how-to-change-a-built-in-lab) |
| Calibration conversion and presets | `src/lib/calibration*.ts`, `src/lib/components/CalibrationCard.svelte`, `CalibrationDialog.svelte`, `src-tauri/src/calibration.rs` |
| Project/Output path validation | `src-tauri/src/project_paths.rs`, `src/lib/project-folder.ts` |
| Restore, Verify, CLI process flags, runtime deployment | `src-tauri/src/firmware_workflow.rs`, `arduino_cli.rs`, `arduino_runtime.rs`, `reference_firmware.rs` |
| Controlled sketch | `firmware/reference_unor4wifi/reference_unor4wifi.ino` |
| Runtime build, source bundles, notices | `scripts/build_arduino_runtime.ps1`, `build_third_party_sources.ps1`, `check_third_party_sources.ps1`, `audit_bundled_runtime_notices.ps1`; `release/arduino-runtime-entries.json`, `release/third-party-sources*.json`, `release/arduino-runtime-4.inventory.txt`; `src-tauri/examples/runtime_smoke.rs`; `src-tauri/resources/licenses/BUNDLED_COMPONENTS.txt` |
| Release pins, packaging, consistency | `release/release-manifest.json`, `src-tauri/tauri.conf.json`, `scripts/build_student_release.ps1`, `scripts/check_release_consistency.ps1` |

### Accepted behavior to preserve

**Layout and display**

- The header, recording bar, message center, setup cards (Board, Project folder, Lab, Session setup, Recording duration, Calibration & Units), Live signals, and Last recording stay in one window.
- **Source** starts on Simulator (practice). It switches to Hardware **once**, the first time firmware becomes ready on the selected port while nothing is recording. It never switches during or after a recording, or after the student chose a Source. Simulator plots carry the SIMULATED DATA watermark.
- Raw ADC counts remain authoritative in BMEG/CSV. One synchronized 0.5–30 s live window (default 5 s), up to 30,000 retained display frames, and at most 2,000 requested display points keep plotting bounded.
- Timestamp filtering and decimation always include the exact newest frame. Display changes, including Pause display and **Show by LED state**, must not issue acquisition commands.
- For LED-sequenced labs, display decimation keeps whole four-record cycles (by `record_sequence / 4`) so every LED state is sampled equally and a plot never aliases to one state, still within `max_points` and with the exact newest record last. A channel shown by LED state is four traces `led:<state>:<id>`, `null` outside their state with gaps spanned; units, calibration, and unit selectors are keyed by the parent channel; the startup-transient filter works per (value index, LED state). `split_by_led_state` is a display default only.
- Multi-signal legends appear only with at least two visible signals in that plot. Endpoint values appear for each visible trace in its selected unit. Plot exceptions are isolated and logged as `PLOT_RENDER_FAIL`.
- The time axis is elapsed recording seconds, not a calendar date. Hardware startup warmup is display-only; the initial raw sample remains in the recording.
- The Live signals "Lab outputs" line lists the outputs the board is driven with (`resolvedDigitalOutputs`, which mirrors Rust's `resolved_digital_outputs`), so a stored legacy revision with only `led_outputs` still shows its D4.

**Board and firmware**

- Board Refresh, Verify, and Restore availability is independent of acquisition firmware readiness; Restore is a recovery path for unknown or outdated firmware.
- Start readiness for Hardware is bound to `verified_port`: it requires a compatible board and a port match, and never consults the last restore. Any failure or job start clears it, including a Retry connection or hardware Start handshake on that port that proves other firmware (`non_protocol_bytes`, `wrong_protocol_version`, `missing_firmware_identity`, `incompatible_firmware_identity`, `older_wvu_firmware`, `newer_wvu_firmware`; applied when the workflow status or Start readiness is next read).
- The handshake requires exactly `REFERENCE_FIRMWARE_BUILD`. The WVU device ID with another build of the same family (upper 16 bits equal, lower 16 bits nonzero) is `older_wvu_firmware` or `newer_wvu_firmware` (Board card **Older/Newer WVU firmware**, both with Restore; no Retry connection, because a retry gets the same answer); anything else stays `incompatible_firmware_identity`. Verify records them as `FirmwareErrorCategory::OlderWvuFirmware`/`NewerWvuFirmware`.
- **Rollout rule:** update the app on every lab PC before restoring any board to a new firmware build, because an older app treats the newer board as incompatible and its Restore puts its own older build back.
- Verify never uploads firmware. A Verify that cannot open the port reports `PortBusy` and keeps the previous compatibility.
- After a failed or canceled restore, the board refresh follows a board that appeared on another port but does not verify it, so the Board card keeps the restore's failure and log. Advanced details always lists the last restore's result and log path. A student-initiated Verify still replaces the card.
- A failed Repair Arduino tools reports `previous_runtime_kept` (the tools in use are still complete on disk); the page then keeps the tools' state, so Restore stays available.
- "Reset board and retry" is retired; recovery is Retry connection, then Verify Firmware, then Restore WVU Firmware.
- Cancel is refused once a restore reaches `Uploading`, and the window cannot close during a firmware job.
- Restore is refused while more than one UNO R4 is connected.
- A native-USB WVU board needs the manual double reset (90 s window); the restore window shows it.

**Concurrency and session state**

- One serial-ownership lease (`serial_lease.rs`) serializes recordings, Verify, Retry, Restore, and tool repair. It is released by `Drop` on every path, so failed operations release serial ownership and busy state.
- Long board operations run in `spawn_blocking`; Tauri command argument names and serde payload names stay aligned with their frontend callers.
- `recording_active` is true only while a Start owns the session. Verify, Retry, and restore-verification handshakes are not recordings. The window-close policy uses the same test (`SessionController::recording_active`): closing during a Verify or Retry closes normally; only a recording is stopped and saved first.
- `fault_origin` says which operation raised `last_error`. The page treats a `storage` fault without `last_summary` as a refusal before START ("could not start", with `last_error`), not as a recording that was not saved. `simulator` keeps describing the last recording after a board check, so a fault is a simulator recording's only while `port` is `SIM` and the origin is not `verification`.
- `SessionError::Cancelled` is the only quiet worker ending; any other failure after Stop faults with its stage.

**Capture protocol**

- `UntilStopped` is a real enum variant with no requested seconds.
- Legitimate terminal paths include User, TimedComplete, Disconnect, ApplicationClose, StorageGuard, and actual acquisition faults.
- Samples that cease for seven seconds trigger a diagnosed fault, not an automatic restart.
- The host sends an **idle-safe STOP before every CONFIGURE**, so a board left acquiring is quiesced and its outputs go LOW before reconfiguration. Build `0x00010007` also stops by itself on a CONFIGURE during acquisition (terminal STATUS, outputs LOW); keep the host STOP anyway.
- Each acquisition mode needs its own CAPABILITIES bit (`FirmwareCapabilities::supports_modes`; 0x01 simultaneous, 0x02 legacy 4-state, 0x04 LED-sequenced). Mode 2 CONFIGURE is `[2, bits, dwell u32, n, pins…, red, ir]`.
- STATUS ≥ 42 bytes carries skipped deadlines and ≥ 46 bytes the minimum settle; both, like the per-phase checks, count only from the terminal STATUS of the current run. A 38-byte STATUS (build 6) still parses.
- Sample flag `0x0004` (`overrun_flagged_records`) and `0x0010` (`deadline_skip_flagged_records`) are counted separately and never added to `firmware_overflows`. For LED-sequenced labs every record with `field[0] != sequence % 4` increments `led_state_mismatches`; the record is still stored unchanged.
- Frames and ERRORs from before the START STATUS are not counted against the new recording.
- After STOP, the host waits up to 300 ms for a **STATUS newer than STOP** (`acquiring = 0`, mask 0) and drains the tail. A mask that is already 0 proves nothing for labs without outputs. A STOP whose write timed out (`TimedOut`/`WouldBlock`), or that is unanswered after 100 ms, is resent at most twice; each resend extends the wait by up to 100 ms. Other write failures are not retried.

**Files and storage**

- Finalization is unconditional and crash-safe: BMEG finish and fsync, rename with retry, then **metadata before CSV**, then a best-effort CSV, then the sidecar rewritten with `csv_filename` or `csv_error`.
- `finalize_error` records flush, rename, or metadata problems. Raw data are never deleted, and files of other recordings are never overwritten: the raw `.bmeg.part` uses `create_new`, run names skip existing files and `.part` siblings, and final moves never replace a file. Only the recording's own metadata sidecar is deliberately replaced when it is rewritten with the CSV result.
- The storage guard stops below 250 MiB **plus the projected CSV size**; the warning is at 1 GiB, or earlier by the same margin. A failed free-space query is not fatal.
- Windows is kept awake (`PowerSetRequest`) from the worker's start through finalization.

**Firmware, provenance, and development**

- The firmware uses RA4M1 native USB CDC. The compile recipe deliberately omits `NO_USB`, and `arduino_runtime.rs` adds the version-pinned `SerialUSB::write_raw` compatibility shim in the app-owned extracted core. Do not revert to the ESP32 bridge or patch an unrelated Arduino IDE installation.
- Pulse-ox phase deadlines advance absolutely with `nextTickMicros += stateDwellUs`. `scheduleNextPulseDeadline` then skips (and counts) deadlines until the new state's sample is at least `dwell / 2` after its LED edge; it lengthens a state and never advances `pulseState`. ADC and record creation are isolated from frame encoding, CRC, and serial transmission by bounded queues and partial writes (no `Serial.write`, `enqueueFrame`, `crc16`, or `serviceBatchEncoding` in `acquirePulseState`/`acquirePulseSequenced`, which a test checks). Host recording retains actual per-record timestamps and per-state timing checks.
- Firmware build `0x00010007` keeps: `buildCrcTable()` as the first statement of `setup()`; `WDT.begin(1000)` last in `setup()` and `WDT.refresh()` first in `loop()`; at most 64 command bytes per pass with `Serial.read() < 0` ending the pass; encoding skipped within 120 µs of the next sample; simultaneous mode refusing any D5/D6 bit; each batch slot holding its own period and field count; START never purging queued sample frames. `MAX_FIELDS` stays 8 (mode 2 uses at most 7).
- LED-sequenced records are `[led_state, ch1..chN]`, read while that state's LEDs are unchanged, with `record_sequence % 4 == led_state`. The CSV is `record_sequence,t_us,cycle_index,led_state,led_state_name,` plus the simultaneous channel columns (`plan_channel_columns`/`write_channel_values`); the legacy 4-state CSV and record layout are unchanged and stay supported.
- The built-in pulse-ox lab is 1.1.0 in the LED-sequenced mode (`tx` A0, `rx` A1, both split, D5/D6, 1000 µs, build `0x00010007`, hash `8d8428e9…`); 1.0.0 (hash `019c25a6…`) stays published in `profiles/legacy/` and installable as a rollback. The other factory labs keep build `0x00010006` and their hashes, and `ACCEPTED_LAB_FIRMWARE_BUILDS` must keep every earlier build, or stored labs disappear.
- MPXV uses `(Vout / Vsupply - 0.04) / 0.009` kPa and `kPa * 7.5006` mmHg. EMG pressure supports counts, volts, kPa, and mmHg. XGZP uses its selected local linear calibration, with at most one active preset per channel. The preset list follows the saved files (a preset deleted elsewhere is dropped with its selection; only the one this window just saved is kept while the list catches up), and it is read again when the window regains focus outside a recording. Pulse ox remains raw-only, with no ambient subtraction or physiological computations.
- Tool-version provenance is best effort: versions are cached, and a failed query records `"unavailable"` instead of failing a recording or restore. Restore logs record `git_revision` from the `GIT_COMMIT` embedded at build time, or `null`.
- `WVU_STUDIO_DEV_LOCAL_DATA` relocates the extracted runtime **in debug builds only**; release builds ignore it, and the runtime under it is never redirected.
- The runtime root stays shorter than `RUNTIME_ROOT_LENGTH_LIMIT` (60) characters, the bundled GCC's limit: a longer `%LOCALAPPDATA%\WVU-BMEG\rt` moves to the user's private `%ProgramData%\WVU-BMEG-<id>\rt`, or fails as `runtime_path_too_long` instead of failing in the compiler.
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

## Op Amp Lab: built in, and the migration of the old copy

On `release-1.1-prep`, **Op Amp Lab** is a factory lab with its own ID. It is the sixth entry, appended last in `FACTORY_LAB_FILES`:

```text
File:     profiles/op_amp_course_capture.profile.json
ID:       wvu.bmeg420l.op_amp.course.capture.v1
Version:  1.0.0 (canonical hash 90113222d07df3e239b95666510ab77b77356bd5dc0ae04b7660d60d0faa579d, in the lock)
Category: course_op_amp (bench-only; no ECG/EMG acknowledgement)
Inputs:   A0 vmid "Vmid (A0)" -> vmid_counts; A1 vout "Vout (A1)" -> vout_counts (counts/volts)
Rate/ADC: 100 frames/s, 14 bit; plots [vmid], [vout]; export signal_name op_amp_course_capture
Required: protocol 0.4, firmware build 0x00010006
```

The frontend has no category-specific code for it: units, plots, readiness, and summaries follow its fields, as for any simultaneous analog lab. The Simulator has a `course_op_amp` signal model (Vmid near 2.5 V with a few mV of deterministic noise; Vout a 1 Hz, 1.5 V sine around it).

**The old copy.** Before this, the Op Amp Lab existed only as a local revision of General Analog:

```text
ID:       wvu.bmeg420l.general.analog.development.v2
Version:  2.0.1
Name:     Op Amp Lab
Inputs:   A0 (id Vmid, csv a0_counts), A1 (id Vout, csv 01_counts)
Hashes:   bcc08868305ca55e81920e3a5805b020cfa612a0374e75fd5a290fa11f918ad9 (instructor, in the maintainer's own catalog)
          c73cc1f2941910696d88f934b7b4c09b8b110438a4886d7d799b13807f61471d (imported, as the v1.0.2 student installer stored it)
```

The two hashes differ only by `source`. While active, that copy replaced General Analog in the Lab list. The student installer (`install_op_amp_lab_profile.bat`/`.ps1`/`.zip`), `package_op_amp_lab_installer.ps1`, and their CI steps were removed. The installer's embedded JSON is kept byte for byte, `\u` escapes included, as `src-tauri/src/testdata/op_amp_2.0.1_imported.lab.json`; Rust tests pin both hashes from it and seed migration catalogs with it.

**Migration `op_amp_builtin_v1`** (`ProfileStore::apply_startup_migrations` / `migrate_legacy_op_amp_lab` in `profiles.rs`):

- **When it runs.** Only from `with_root`, while it holds the catalog lock and the catalog loaded writable; never from `sync_with_disk`. It runs if the catalog's additive `applied_migrations` list lacks `op_amp_builtin_v1` and General Analog 2.0.1 with one of the two verified hashes is the active, unretired General Analog revision.
- **What it does.**
  1. Copies the catalog's bytes (`write_new_file_durably`: `create_new`, write, `sync_all`; no attributes, so a read-only catalog gives a writable copy) to `lab_state.pre-migration-<time>.json`, or reuses an existing `lab_state.pre-migration-*.json` with the same bytes (left by a start whose write failed). If the copy fails, nothing changes (`LAB_CATALOG_MIGRATION_DEFERRED`).
  2. Retires 2.0.1 and activates General Analog's course default, then commits through the normal transactional write.
  3. Logs `LAB_CATALOG_MIGRATED` (retired version and hash, active version, backup path) and writes a `lab_write_audit.log` line, operation `op_amp_builtin_v1`.
  4. Adds `CatalogStatus.migration_notices`. The Lab card shows it in both modes through `catalogMigrationNotice` (`src/lib/lab-catalog.ts`); it does not raise the "Some labs could not be loaded" banner.
- **Failures.** A failed commit keeps the migrated runtime for this launch (the notice says it was not saved) and logs `LAB_CATALOG_MIGRATION_NOT_SAVED`. A later write in that launch saves it; otherwise the next start migrates again. Without the lock, nothing is written and the next start tries again.
- **The marker.** It is decided when a catalog is read, not when it is written (`settle_op_amp_builtin_migration`, called by `runtime_with_catalog` at startup and in `sync_with_disk`): a 2.0.1 copy that is already inactive or retired puts `op_amp_builtin_v1` in the in-memory `applied_migrations`. The migration itself and an instructor's **Import lab** of the copy (`install_package`) add it too. Writes persist exactly the in-memory set, so an instructor who restores or re-imports the copy keeps it, while a copy that is still selected and not migrated (a start without the lock, or a copy another program selected while the window was open) is never marked by a later write and is migrated at the next start. It is not written while the copy is absent, so a later run of the old installer is still caught. Reset local customizations clears it.

**On the maintainer's own PC** the migration also retires the maintainer's instructor copy (hash `bcc08868…`). It can be restored with **Restore** in Manage Labs, or from the `lab_state.pre-migration-*.json` backup. Hardware check: on that catalog the backup and the notice exist, and the built-in Op Amp Lab records `vmid_counts` and `vout_counts`.

Test catalog changes with temporary roots (the Rust tests) or `scripts\start_dev_sandbox.ps1`, never against the maintainer's live catalog.

## Firmware build 0x00010007 and LED-sequenced pulse ox

On `release-1.1-prep` (2026-11-19; design approved by the maintainer on 2026-11-18) the reference firmware is build `0x00010007`, still protocol 0.4 and device ID `0x554E4F34`:

```text
.ino SHA-256: 86221c104b8a7cf30bfbb0ac621cfdfebc8b08635ed0710df66db8fa4f5d681d (LF, ASCII)
Pinned in:    src-tauri/src/reference_firmware.rs (REFERENCE_SOURCE_SHA256),
              release/release-manifest.json (reference_firmware_build, reference_firmware_source_sha256),
              src-tauri/src/protocol.rs (REFERENCE_FIRMWARE_BUILD)
HELLO/CAPABILITIES mode byte: 0x07 (simultaneous, legacy 4-state, LED-sequenced)
STATUS: 46 bytes (skipped deadlines at 38, minimum settle at 42)
Compile (runtime revision 4, offline): flash about 60.1 kB (22%; 60,080-60,088 B, varies with the build folder), RAM 12,344 B (37%)
```

[USB Protocol](USB_PROTOCOL.md) describes the protocol changes and [Hardware](HARDWARE.md#pulse-oximetry) the timing.

**LED-sequenced mode** (`AcquisitionMode::PulseoxSequenced`, `"pulseox_sequenced"`, CONFIGURE mode 2). 1–6 channels of A0-A5, dwell 1000-5000 µs, rate `round(1e6 / dwell)`, distinct `acquisition_sequenced` RED/IR outputs on D4-D6 and no others, `required_firmware.build` ≥ `0x00010007` (checked before the hash), plot defaults covering every channel, channels in ADC counts. `split_by_led_state` is an optional, display-only channel flag (omitted when false, so existing hashes are unchanged) allowed only in this mode. Record fields are `["led_state", <csv names>]`; the BMEG header and sidecar carry `led_sequenced_layout`. The XGZP fit reads a sequenced channel one field later (after `led_state`), and calibration membership uses `resolved_channels()`, so `led_state` and LED-state trace IDs cannot be calibrated.

**Built-in pulse-ox lab 1.1.0.** `profiles/pulseox_tx_rx_course_capture.profile.json` was edited to the sequenced mode and published with `scripts/update_factory_labs.ps1 fix … --bump-minor`, which appended `1.1.0` / `8d8428e9d5de1d07480417adf0c94a61f401009f6c59ad31109f33dbed4239cd` to the lock and kept the committed 1.0.0 byte for byte as `profiles/legacy/pulseox_tx_rx_course_capture_1.0.0.profile.json` (hash `019c25a6…`). Tests take 4-state coverage from that file (`legacy_pulse_ox_lab()` in `profiles.rs`); `sequenced_test_lab()` is built from it and must equal 1.1.0's acquisition, outputs, and plot defaults. Install lab file accepts the 1.0.0 file (it is a published factory version), so it is a rollback; Restore course default returns to 1.1.0. A PC whose instructor made a local 4-state pulse-ox revision active keeps it.

**Rollout.** The host accepts exactly build `0x00010007`. Update the app on every lab PC first, then restore every board once from an updated PC: a 1.0.x PC reports a build-7 board as incompatible and its Restore puts build 6 back. 1.1 PCs show **Older WVU firmware** for build 1-6 boards and **Newer WVU firmware** for later builds of the family.

**Not verified on hardware yet:** everything in the [hardware checklist](#hardware-verification-checklist-for-review-fixes) under "Firmware build 0x00010007". The sketch only compiled (offline, runtime revision 4); no board was used.

## Deferred decisions

The 2026-11-17 review recorded these items. They need hardware, legal review, or the maintainer's decision, so this pass did not resolve them.

**Firmware** (any `.ino` edit changes the pinned source SHA-256 in `reference_firmware.rs`, the release manifest, and the exact-string tests. It needs a new build ID, profile `required_firmware` updates only for labs that use a new feature, a bench test, and a Restore on every lab board after every lab PC is updated.)

Resolved on `release-1.1-prep` in build `0x00010007` (see [Firmware build 0x00010007](#firmware-build-0x00010007-and-led-sequenced-pulse-ox)); the bench tests are in the hardware checklist:

- CONFIGURE received mid-acquisition now stops the acquisition first, like STOP. Queued sample frames are deliberately **not** purged at START (each batch slot keeps its own layout instead); the host STOP before CONFIGURE stays.
- Skipped sampling deadlines have their own flag `0x0010` and a STATUS counter; `0x0004` means a dropped batch only.
- A minimum-settle guard keeps every pulse-ox sample at least half a dwell after its LED edge, and STATUS reports the minimum observed settle.
- Table-driven CRC; batch encoding waits while a sample is due within 120 µs.
- Hardware watchdog (the sketch requests 1000 ms; the core's `WDT` library rounds it up to about 1.4 s, 1,398 ms, on the UNO R4); `readCommands` reads at most 64 bytes per pass and stops when `Serial.read()` returns -1.
- Simultaneous-mode CONFIGURE rejects any D5/D6 bit.
- The `.ino` header comment describes the three modes generically.

**Release and legal**

- License texts for Rust crates and npm packages in the installers
- The permissive (MIT, BSD, Apache-2.0) license texts of the Go modules linked into Arduino CLI and the discovery tools are only in the source bundle's vendor ZIPs, not in the installer (a notices file generated from `vendor/modules.txt` would fix it). The core's `bootloaders/*.hex` and `.elf` have no stated license or source pointer upstream.
- **1.0.x source bundle (maintainer decision).** Attaching `WVU-Bioinstrumentation-Studio-1.0.x-third-party-sources.zip` to v1.0.0-v1.0.2 changes published releases and needs an explicit go-ahead, after a rebuild with `-Runtime` pointing at the revision-3 archive. Ask Arduino now for the exact OpenOCD 0.11.0-arduino2 and dfu-util 0.11.0-arduino5 build sources, so a forwarded request can be answered.

Resolved on `release-1.1-prep`: corresponding source for the bundled GPL/LGPL tools (per-release source bundles and a written offer), the pre-release Arduino CLI pin (now 1.6.0), and the runtime rebuild recipe (`scripts/build_arduino_runtime.ps1`).

Resolved on `release-1.1-prep`: `SECURITY.md` points to GitHub private vulnerability reporting, and the installers carry publisher and copyright metadata and a pinned MSI upgrade code.

**Upgrade from 1.0.x under the new publisher (resolved in source; Sandbox test open).** The 1.0.x installers used Tauri's default publisher `wvu`, the second part of the identifier. Tauri CLI 2.11.4's NSIS template keys three things on the publisher: the install-folder key `Software\<publisher>\<product>` (HKLM for this per-machine install), which it passes to the old uninstaller as `_?=`; the folder it proposes; and the `DisplayName` plus `Publisher` match that finds an MSI install. With the stock template, a 1.1 setup would start the 1.0.x uninstaller with an empty `_?=`, ignore a custom 1.0.x folder, and miss a 1.0.x MSI install.

The maintainer chose on 2026-11-18 to keep the department as publisher and ship a custom NSIS template. `bundle.windows.nsis.template` is `nsis/installer.nsi`: the CLI 2.11.4 template, kept unmodified as `src-tauri/nsis/installer.upstream.nsi`, plus regions between `; WVU-LEGACY-PUBLISHER BEGIN` and `END` lines. They fall back to `Software\wvu\<product>` when uninstalling and when proposing the folder; then to the folder of the registered `UninstallString` if the app binary is there; and otherwise show "Unable to uninstall!" and abort, never starting the uninstaller with an empty `_?=`. The abort returns to the reinstall page in the GUI. In passive mode (`/P`) it skips the page and installs without uninstalling 1.0.x, as upstream's own failure path does. When a folder is found, the region ends with `ClearErrors` and a second `UninstallString` read. The failed read of the new publisher key sets NSIS's sticky error flag, and without the reset upstream's `${Errors}` check would turn the old uninstaller's exit code 0 into 2 ("Unable to uninstall!") after 1.0.x had already been removed. The consistency check requires both lines. They also accept Publisher `wvu` in the MSI search. NSIS installer hooks could not do this: the hooks file is included before the template's `!define`s, and `NSIS_HOOK_PREINSTALL` runs after the reinstall page. [`src-tauri/nsis/README.md`](../src-tauri/nsis/README.md) records the upstream tag, commit, and SHA-256. The upstream file was matched byte for byte against the template embedded in the installed CLI binary.

`scripts/check_release_consistency.ps1` fails with "Tauri CLI changed: re-sync src-tauri/nsis/installer.nsi from the new upstream template" when `@tauri-apps/cli` differs from the recorded version, and it fails when `installer.nsi` differs from the upstream copy outside the marked regions or lacks the fallbacks. Every Tauri CLI update needs the re-sync in [Developer Guide](DEVELOPER_GUIDE.md#re-syncing-the-nsis-template). makensis compiled the template on 2026-11-18 (`npm run tauri build -- --bundles nsis`, placeholder runtime; that setup was deleted).

The MSI needs no template change. Tauri's WiX template has `MajorUpgrade` (`Schedule="afterInstallInitialize"`), and the pinned `UpgradeCode` equals the one the 1.0.x MSIs derived from the product name, so a 1.1 MSI removes a 1.0.x MSI install. Its folder search reads `HKCU\Software\<publisher>\<product>`, so a custom 1.0.x MSI folder is not reused; this is accepted.

Still open: run the four upgrades from 1.0.2 in Windows Sandbox (NSIS to NSIS from the default folder and from a custom folder, MSI to MSI, MSI to NSIS), as listed in [Release Checklist](RELEASE_CHECKLIST.md#upgrade-from-the-previous-release). Only those tests exercise the fallbacks at run time. Until they pass, the 1.1 release notes and [Installation](INSTALLATION.md) must not promise a seamless upgrade.

**Design**

- The Op Amp Lab's shared General Analog ID: resolved on `release-1.1-prep` by shipping it as a factory lab with its own ID (`wvu.bmeg420l.op_amp.course.capture.v1`, CSV fields `vmid_counts`/`vout_counts`), retiring the student installer, and migration `op_amp_builtin_v1` for catalogs where the old 2.0.1 copy is active; see [Op Amp Lab](#op-amp-lab-built-in-and-the-migration-of-the-old-copy). The old copy's `01_counts` field stays as it was in that stored revision.
- Numbering of local revisions versus future factory patches of the same ID: resolved by version policy P2 (factory versions `MAJOR.MINOR.0`, edited revisions PATCH ≥ 1, `profiles/factory-labs.lock.json`); see [Lab Configuration](LAB_CONFIGURATION.md#factory-lab-versions). A new lab ID starts locally at 1.0.0, so `next_factory_lab_version` also counts the revision a factory version is made from (`promote`, "Save as course default file…"): a new lab saved as 1.0.x becomes factory 1.1.0. Every lock entry stays held by a file (`profiles/` or `profiles/legacy/<file>_<version>.profile.json`); `fix`/`promote` keep a replaced published version there (from the file, or from `git cat-file blob HEAD:profiles/<file>` in the example tool), and `check` reports a missing one. Install lab file refuses a `built_in` file that is not in the compiled lock with that hash (a course default file for a later build). A stored lab that still collides with a course default is kept and reported, never erased.
- Windows profile folder names with characters outside the ANSI code page. Measured on 2026-11-18 with `runtime_smoke` (code page 1252): a runtime root with `Zoë` compiles; `张伟` fails with `Invalid argument` both in the runtime root and, with an ASCII runtime root, in the job folder and Arduino build cache under `LOCALAPPDATA`. Moving only the runtime to `%ProgramData%` would not fix it: the app's job folder and Arduino CLI's build path would have to move too (an ASCII-only private folder, `--build-path`). Documented as a known limitation in [Troubleshooting](TROUBLESHOOTING.md#arduino-tools-cannot-be-set-up-for-this-windows-account) and the [Developer Guide](DEVELOPER_GUIDE.md#runtime-folder-and-the-compilers-path-limit); the app shows the compiler error, not a dedicated message.
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
