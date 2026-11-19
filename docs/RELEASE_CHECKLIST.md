# Release Checklist

Work through every section for each release. Items marked **1.1.0** are specific to the first release after 1.0.2 (new firmware build, Arduino runtime revision 4, a new installer publisher). The [handoff](AGENT_HANDOFF.md#what-is-left-before-110-can-be-published) tracks what is still open for 1.1.0.

## Source and verification

- Set the intended application version in all of these:
  - `package.json`
  - The `package-lock.json` root (both `version` fields)
  - `src-tauri/Cargo.toml` and the `src-tauri/Cargo.lock` root package
  - `src-tauri/tauri.conf.json`
  - `release/release-manifest.json`

  Move the `## Unreleased` entries in `CHANGELOG.md` under a `## <version>` heading, and add a `RELEASE_NOTES_<version>.md` (see [Changing the application version](DEVELOPER_GUIDE.md#changing-the-application-version)). **1.1.0:** done on `release-1.1-prep`.
- `release/third-party-sources.json` names this version (`bundle_name`, `applies_to.app_versions`, `applies_to.installers`) and the pinned runtime. **1.1.0:** it does; do not change it, or the bundle built from it no longer verifies.
- Run `pwsh -NoProfile -File scripts\check_release_consistency.ps1`. It must report no mismatch in any of these:
  - The versions above, the `CHANGELOG.md` heading, and `RELEASE_NOTES_<version>.md`
  - The firmware protocol, build, and device ID
  - The reference `.ino` SHA-256
  - The Arduino CLI and core versions
  - The Tauri npm/crate pairs
  - The installed `LICENSE.txt` and `NOTICE.txt` copies and the copyright line
  - The custom NSIS template (`src-tauri/nsis/`) against the Tauri CLI version and its upstream copy. After a CLI update, see [Re-syncing the NSIS template](DEVELOPER_GUIDE.md#re-syncing-the-nsis-template).
- Run `npm ci`.
- Run `npm run check`, then `npm run build`, then `npm test`. This order matters: a release-configuration test needs the built `build/index.html`, so `npm test` fails on a clean checkout without the build.
- Run `cargo fmt --manifest-path src-tauri\Cargo.toml -- --check`.
- Run `cargo check`, `cargo test`, and `cargo clippy` with `--locked` (Clippy with `--all-targets -- -D warnings`). The Rust version comes from `rust-toolchain.toml`.
- Run `pwsh -NoProfile -File scripts\check_third_party_sources.ps1`. It validates both third-party source manifests without downloading and checks that `release/third-party-sources.json` covers the bundled runtime.
- Run `npm run tauri build`; do not substitute `cargo build --release` for a production application build.
- Launch the resulting Tauri release executable with no Vite or Node development server running. It must load bundled frontend assets and never request localhost.
- Use the real, hash-pinned `src-tauri/resources/arduino-runtime.zip`. The CI placeholder satisfies source checks only and must never be packaged or used for hardware tests. Verify it:
  - `(Get-FileHash src-tauri\resources\arduino-runtime.zip -Algorithm SHA256).Hash.ToLowerInvariant()` equals `arduino_runtime_archive_sha256` in `release/release-manifest.json` (runtime revision 4: `5baa8a5fc7fc0c2ae7fd9f49de1876d76859067af6d56e9b56546a88a5d02def`).
  - `pwsh -NoProfile -File scripts\audit_bundled_runtime_notices.ps1 -ArchivePath src-tauri\resources\arduino-runtime.zip` passes.
  - `cargo run --manifest-path src-tauri\Cargo.toml --example runtime_smoke -- C:\wvu-smoke` prints `PASS`: the runtime compiles the reference sketch offline. Record the flash and RAM use (build `0x00010007` with runtime revision 4: flash about 60.1 kB, 22%; RAM 12,344 bytes, 37%). The exact flash count varies by a few bytes with the build folder (60,080–60,088 bytes so far), because the binary embeds source paths, so a small difference does not mean the sketch changed; the source SHA-256 that `runtime_smoke` prints is the check. Then delete `C:\wvu-smoke`.

## Functional smoke

- Confirm board discovery, firmware verification, and firmware restore work without external console windows.
- Record a short ECG session, a second session without restart, and a short multi-channel EMG session.
- Confirm raw pulse-ox fields, safe LED behavior, absolute state scheduling, measured per-record timestamps, and explicit acquisition-queue overflow and skipped-deadline accounting with the appropriate hardware setup, in both pulse-ox modes. The detailed timing criteria are under [Hardware acceptance](#hardware-acceptance).
- Confirm final metadata timing diagnostics agree with the recorded timestamps and that optional recording notes are preserved exactly.
- Confirm new BMEG, CSV, and metadata filenames identify the lab and contain no development-phase label.
- Open Instructor Manage Labs and verify that reading/selecting labs creates no version.

## Hardware acceptance

Run these with the real runtime and an Arduino UNO R4 WiFi, using one application and one serial owner at a time. Record the build, the restore-job source hash, and the result of each. For 1.1.0, the [handoff's hardware checklist](AGENT_HANDOFF.md#hardware-verification-checklist) is the working copy of this section, with the 1.1.0-only items.

**Firmware and Arduino runtime**

- **Offline restore.** With the computer's network disconnected (Wi-Fi off, cable out), a first start after installing extracts the tools (or **Repair Arduino tools** extracts them again), and **Restore WVU Firmware** compiles and uploads. The restore log (`firmware_job_*.json`) shows Arduino CLI and core versions matching the runtime manifest (revision 4: CLI `1.6.0`, core `1.6.0`) and the reference `.ino` SHA-256 from `release/release-manifest.json` (build `0x00010007`: `86221c10…`).
- **Firmware rollout (1.1.0).** A board running build `0x00010006` (restored by 1.0.2) shows **Older WVU firmware** on this release; one **Restore WVU Firmware** makes it **Ready**. A 1.0.2 computer reports a build-`0x00010007` board as **Firmware update required** (incompatible firmware; expected).
- **Long profile folder (1.1.0).** On a Windows account whose profile folder name is longer than 24 characters, the app uses the private `%ProgramData%\WVU-BMEG-<id>\rt` folder and a restore compiles (details in the handoff checklist).

**Recording**

- An ECG lab recording (a lab with no outputs) is complete to the last sample:
  - The metadata shows no missing sample sequences.
  - `total_samples` matches the CSV row count.
  - `board_elapsed_seconds` of a timed run is within one sample period of the planned duration (for example 59.995 s for 60 s at 200 frames/s).
- **Stop recording** during a timed run saves a complete recording with stop reason `user`.
- Unplugging the board mid-recording ends the recording as incomplete, with the finalized partial `.bmeg` and its metadata. The Last recording card reports it.
- The first Start after the board was left acquiring (for example after a forced app close) is clean. The idle STOP before CONFIGURE takes effect, and no stale records start the new file.
- A laptop on battery stays awake during a 10-minute recording (`powercfg /requests` lists the app while it records).
- **Simultaneous, six channels at 1 kHz.** A 10-minute General Analog recording on A0–A5 at 1000 frames/s has `timing_diagnostics.firmware_skipped_deadlines` 0, `integrity.deadline_skip_flagged_records` 0, no missing sequences, and a measured rate within 1% of 1000 frames/s.

**Board operations**

- Verify Firmware while Arduino IDE's Serial Monitor holds the port reports the busy port and keeps the previous firmware result:
  - On a board not yet verified, the Board card chip reads **Port busy**.
  - On a board that was **Ready on COMx**, the chip stays **Ready on COMx**, the **Last firmware operation** card shows **Verify failed: Another program is using the board**, and with Source = Hardware the message center shows the same headline.
- Restore with two UNO R4 WiFi boards connected is refused.
- **Cancel restore** is unavailable once the upload starts.
- Restoring a board that runs the native-USB WVU firmware shows the double-reset countdown, and a double press of RESET continues the restore.
- The window stays responsive (plots, scrolling, other cards) during Verify, Restore, and Start.

**Pulse-ox timing, both modes**

- **LED-sequenced (built-in pulse-ox lab 1.1.0, 1000 µs dwell).** A physical 60-second capture:
  - Has a median record interval near 1000 µs and remains within 1% of 1000 records/s
  - Has no record interval outside ±5%, and no pattern by `record_sequence` modulo 10 (the batch size) or modulo 4
  - Has `led_state` equal to `record_sequence % 4` on every CSV row and `integrity.led_state_mismatches` 0
  - Has `timing_diagnostics.firmware_skipped_deadlines` 0, `integrity.deadline_skip_flagged_records` 0, and `integrity.overrun_flagged_records` 0
  - Has `timing_diagnostics.minimum_pulse_settle_us` at least 500 (50% of the dwell)
- **LED-sequenced with six channels** (an instructor lab on A0–A5 at 1000 µs): the same checks pass, and the CSV has the six channels' columns on every row.
- **Legacy 4-state** (the 1.0.0 file from `profiles/legacy/`, installed with **Install lab file…**, and a copy edited to a 250 µs dwell): at 1000 µs, the median cycle interval is near 4000 µs, within 1% of 250 cycles/s, with no cycle interval outside ±5% and no cycle-index-modulo-10 timing pattern. At both dwells, the per-state checks are 0; at 250 µs, also record the skipped deadlines and the minimum settle (at least 125 µs). Return to 1.1.0 with **Restore course default** afterwards.
- Final metadata reports zero per-state intervals outside ±5% for RED, DARK 1, IR, and DARK 2 at 1000 µs in both modes; review each state's maximum absolute timing error.
- In every pulse-ox recording, `timing_diagnostics.minimum_pulse_settle_us` is at least 50% of the dwell.

**Outputs and watchdogs**

- D4/D5/D6 read LOW after Stop, after a fault, and after the 5 s host-command watchdog. To trigger it, end the app's process during a recording.
- With a logic analyzer on D5 and D6 during an LED-sequenced recording at 1000 µs: the states run in the order RED, DARK 1, IR, DARK 2 and each lasts 1 ms, so each LED is HIGH for 1 ms per four-state cycle; D5 and D6 are never HIGH together (in the legacy 4-state mode too); both are LOW after Stop.
- **Hardware watchdog.** A 30-minute LED-sequenced recording (six channels, 1000 µs) completes with no board reset, no missing sequences, and the checks above. The board does not reset while idle for 30 minutes either.

**Labs (1.1.0)**

- The built-in Op Amp Lab records `vmid_counts` and `vout_counts` from A0 and A1.
- On a catalog that holds the Op Amp Lab 2.0.1 copy installed by the 1.0.2 student installer, the first start retires it, leaves a `lab_state.pre-migration-*.json` backup, and shows the migration notice in the Lab card; a second start shows no notice and makes no new backup.

## Installer and distribution

- Validate the Arduino runtime manifest and bundled archive before running `pwsh -NoProfile -File scripts\build_student_release.ps1` (PowerShell 7.2, clean Git work tree). The script builds the NSIS setup and the MSI package serially and stages both.
- **Third-party source bundle.** The release script either builds `dist\WVU-Bioinstrumentation-Studio-<version>-third-party-sources.zip` and its `.sha256` from `release/third-party-sources.json` (network access and Go; see [Third-party source bundles](DEVELOPER_GUIDE.md#third-party-source-bundles)), or, with `-ThirdPartySourcesZip <path>`, verifies a bundle built earlier (its `.sha256`, then `build_third_party_sources.ps1 -VerifyBundle`: every file against `SHA256SUMS.txt`, the manifest copy, every pinned component and vendor tree) and copies it to `dist\`. The file must be named exactly `WVU-Bioinstrumentation-Studio-<version>-third-party-sources.zip`, with its `.sha256` beside it.
  - **1.1.0:** reuse the bundle built on 2026-11-18 (279,590,658 bytes, SHA-256 `a55586d4ddd330e7de3e4dc15d941f542050b85a26e28deee82f25bde28c402a`; the [handoff](AGENT_HANDOFF.md#runtime-revision-4-and-the-third-party-source-bundles) says where it is): `pwsh -NoProfile -File scripts\build_student_release.ps1 -ThirdPartySourcesZip <folder>\WVU-Bioinstrumentation-Studio-1.1.0-third-party-sources.zip`. If it is lost, run without the parameter to build it again. With PowerShell 7.7.0 and .NET 11.0.0 the rebuild should have the same SHA-256; with another .NET it differs and is still valid, and that bundle is the one to publish.
  - Before publishing, open the bundle's `README.txt`: it names this version's installers and runtime, and its written offer points to the "Request corresponding source" issue form.
- Check the checksums:
  - `dist\WVU-Bioinstrumentation-Studio-<version>\SHA256SUMS.txt` covers both installers. The same file is inside the ZIP.
  - `<zip>.sha256` beside the ZIP covers the ZIP.
  - `...-third-party-sources.zip.sha256` covers the source bundle; `sha256sum -c SHA256SUMS.txt` inside the extracted bundle passes.
  - Keep `...-build-info.txt` with the release records. It lists the source commit and the runtime and source-bundle SHA-256.
- Install the NSIS package on a clean or isolated Windows user if available. Confirm Program Files installation and standard-user operation.
- The MSI ships beside NSIS for managed IT deployment. Test it on a separate clean machine, never on top of an NSIS installation. Confirm the [Installation](INSTALLATION.md) guidance (one installer type per computer) still holds.
- Installed apps shows the new version and the publisher from `tauri.conf.json` `bundle.publisher` (**1.1.0:** West Virginia University Department of Chemical and Biomedical Engineering).
- Confirm uninstall preserves Project folders and recordings. It also leaves the Arduino tools in `%LOCALAPPDATA%\WVU-BMEG\rt` and any `%ProgramData%\WVU-BMEG-<id>` folder, as [Installation](INSTALLATION.md#user-data) says.
- Regenerate installer, MSI, ZIP, manifest, and SHA-256 values for each candidate.
- Verify the icon, no-console behavior, offline startup, and student-facing copy.

## Upgrade from the previous release

Test in Windows Sandbox or a disposable virtual machine, never on a lab or maintainer computer. Install the previous release, create a Project folder with a recording, then install the candidate. After each upgrade, confirm:

- One Installed-apps entry, with the new version and publisher
- The install folder and the Start-menu shortcut
- The app starts
- Project folders, recordings, calibration presets, and local labs are kept

The upgrades to test:

- NSIS to NSIS with the default **Uninstall before installing** option, once from the default folder and once from a custom folder. The previous uninstaller runs without "Unable to uninstall", the directory page proposes the previous folder, and the candidate installs there.
- MSI to MSI. First read `UpgradeCode` from the previous release's MSI `Property` table (for example with Orca) and confirm it equals `bundle.windows.wix.upgradeCode` in `src-tauri/tauri.conf.json` (`6e489616-048c-5235-969f-11e68577b5d7`). The previous MSI install is removed.
- MSI to NSIS. The NSIS setup detects the MSI installation, removes it before installing, and leaves one Installed-apps entry.

**1.0.x to 1.1.** The publisher changed from `wvu` (Tauri's default, which 1.0.0-1.0.2 used) to the department. The custom NSIS template ([`src-tauri/nsis/README.md`](../src-tauri/nsis/README.md)) falls back to the 1.0.x registry key and publisher, so run every upgrade above with 1.0.2 as the previous release:

- NSIS 1.0.2 to NSIS 1.1 from the default folder. To exercise the last-resort fallback once, repeat it after deleting `HKLM\SOFTWARE\wvu\WVU Bioinstrumentation Studio`: the setup must then take the folder from the registered `UninstallString`.
- NSIS 1.0.2 to NSIS 1.1 from a custom folder. The candidate must propose and keep the custom folder.
- MSI 1.0.2 to MSI 1.1. The WiX folder search uses the publisher key, so a custom 1.0.2 MSI folder is expected not to be reused: 1.1 installs into the default folder. Record what happens.
- MSI 1.0.2 to NSIS 1.1. The NSIS setup must detect the 1.0.2 MSI, which is registered with Publisher `wvu`.
- After each upgrade, Installed apps lists one **WVU Bioinstrumentation Studio** entry, version 1.1.0, with the department as publisher, and no entry with publisher `wvu` remains.

After the NSIS upgrades, `HKLM\SOFTWARE\West Virginia University Department of Chemical and Biomedical Engineering\WVU Bioinstrumentation Studio` holds the install folder. `HKLM\SOFTWARE\wvu\WVU Bioinstrumentation Studio` may remain: the 1.0.2 uninstaller removes it only when **Delete the application data** is ticked. Make sure the release notes and [Installation](INSTALLATION.md) match what the tests show; until these tests pass, neither may promise a seamless upgrade from 1.0.x.

## Documentation

- If the visible UI changed, refresh the README screenshots (see [Developer Guide](DEVELOPER_GUIDE.md#driving-and-screenshotting-the-window)). The Board card screenshot needs real hardware and the real runtime. **1.1.0:** it was removed in the redesign and must be retaken on hardware.
- Update the user guides, [USB Protocol](USB_PROTOCOL.md), and [Architecture](ARCHITECTURE.md) for any behavior the release changes.

## Publication review

- Confirm a project-owner-approved root `LICENSE` exists before granting public reuse rights, and that `NOTICE` and `tauri.conf.json` (`publisher`, `copyright`) name the approved copyright holder and year.
- Confirm third-party notices for the bundled runtime: `BUNDLED_COMPONENTS.txt` and [Third-Party Notices](THIRD_PARTY_NOTICES.md) name the bundled versions, and the release carries the matching third-party source bundle.
- Review `RELEASE_NOTES_<version>.md`, which becomes the release body:
  - It keeps the teaching-use / not-a-medical-device notice.
  - **1.1.0 rollout rule:** it tells instructors to **update the app on every computer that connects to the course boards first (lab computers, instructors' laptops, and students' own installations), then restore every board once**, and to have students update before the boards are restored. A 1.0.x computer reports a restored board as **Firmware update required**, and its Restore would put build `0x00010006` back.
  - It describes the upgrade from 1.0.x only as the [upgrade tests](#upgrade-from-the-previous-release) showed it.
  - It mentions the third-party source bundle attached to the release.
- Before the commit the release is built from, `CHANGELOG.md`, the release notes, and the user and developer guides have no pre-release wording ("not published yet", "prepared but not released", tests still to run, branch names). They go into the tag and stay on `master`; the open status belongs in the [handoff](AGENT_HANDOFF.md) only.
- **1.0.x source bundle.** `WVU-Bioinstrumentation-Studio-1.0.x-third-party-sources.zip` (SHA-256 `b97bd698…`) and its `.sha256` have been attached to v1.0.0-v1.0.2 since 2026-11-19. Leave `release/third-party-sources-runtime3.json` unchanged. A note about it in those release bodies is optional and needs the maintainer's approval. The 1.1 notices do not depend on it: `BUNDLED_COMPONENTS.txt` and the 1.1 bundle's README point to `release/third-party-sources-runtime3.json` and the issue form for OpenOCD and dfu-util.
- Confirm no secrets, recordings, personal calibration files, or developer paths are tracked.
- Confirm branding and code-signing decisions with the repository owner.

## Publish

Only when the maintainer asks:

- Tag `v<version>` (an annotated tag, like the earlier releases) on the commit recorded in `...-build-info.txt`, the commit the release was built from.
- Create the GitHub release with `RELEASE_NOTES_<version>.md` as the body.
- Attach the NSIS setup, the MSI, the distribution ZIP and its `.sha256`, `SHA256SUMS.txt`, and the third-party source bundle with its `.sha256`. The source bundle is published with the installers, never later.
- After publishing, download the source bundle and the ZIP from the release page and check them against their `.sha256` files.
