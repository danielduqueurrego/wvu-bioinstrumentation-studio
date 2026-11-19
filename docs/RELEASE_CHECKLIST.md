# Release Checklist

## Source and verification

- Set the intended application version in all of these:
  - `package.json`
  - The `package-lock.json` root (both `version` fields)
  - `src-tauri/Cargo.toml` and the `src-tauri/Cargo.lock` root package
  - `src-tauri/tauri.conf.json`
  - `release/release-manifest.json`

  Move the `## Unreleased` entries in `CHANGELOG.md` under a `## <version>` heading, and add a `RELEASE_NOTES_<version>.md`.
- Run `pwsh -NoProfile -File scripts\check_release_consistency.ps1`. It must report no mismatch in any of these:
  - The versions above
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
  - `cargo run --manifest-path src-tauri\Cargo.toml --example runtime_smoke -- C:\wvu-smoke` prints `PASS`: the runtime compiles the reference sketch offline. Record the flash and RAM use, then delete `C:\wvu-smoke`.

## Functional smoke

- Confirm board discovery, firmware verification, and firmware restore work without external console windows.
- Record a short ECG session, a second session without restart, and a short multi-channel EMG session.
- Confirm raw pulse-ox fields, safe LED behavior, absolute phase scheduling, measured per-cycle timestamps, and explicit acquisition-queue overflow accounting with the appropriate hardware setup.
- At a 1000 µs pulse-state dwell, confirm that a physical 60-second capture:
  - Has a median cycle interval near 4000 µs
  - Remains within 1% of 250 cycles/s
  - Has no cycle interval outside ±5%
  - Has no cycle-index-modulo-10 timing pattern
- Confirm final metadata reports zero per-state intervals outside ±5% for RED, DARK 1, IR, and DARK 2; review each state's maximum absolute timing error.
- Confirm final metadata timing diagnostics agree with the recorded timestamps and that optional recording notes are preserved exactly.
- Confirm new BMEG, CSV, and metadata filenames identify the lab and contain no development-phase label.
- Open Instructor Manage Labs and verify that reading/selecting labs creates no version.

## Hardware acceptance

Run these with the real runtime and an Arduino UNO R4 WiFi, using one application and one serial owner at a time. Record the build, the restore-job source hash, and the result of each.

**Recording**

- An ECG lab recording (a lab with no outputs) is complete to the last sample:
  - The metadata shows no missing sample sequences.
  - `total_samples` matches the CSV row count.
  - `board_elapsed_seconds` of a timed run is within one sample period of the planned duration (for example 59.995 s for 60 s at 200 frames/s).
- **Stop recording** during a timed run saves a complete recording with stop reason `user`.
- Unplugging the board mid-recording ends the recording as incomplete, with the finalized partial `.bmeg` and its metadata. The Last recording card reports it.
- The first Start after the board was left acquiring (for example after a forced app close) is clean. The idle STOP before CONFIGURE takes effect, and no stale records start the new file.
- A laptop on battery stays awake during a 10-minute recording (`powercfg /requests` lists the app while it records).

**Board operations**

- Verify Firmware while Arduino IDE's Serial Monitor holds the port reports the busy port and keeps the previous firmware result:
  - On a board not yet verified, the Board card chip reads **Port busy**.
  - On a board that was **Ready on COMx**, the chip stays **Ready on COMx**, the **Last firmware operation** card shows **Verify failed: Another program is using the board**, and with Source = Hardware the message center shows the same headline.
- Restore with two UNO R4 WiFi boards connected is refused.
- **Cancel restore** is unavailable once the upload starts.
- Restoring a board that runs the native-USB WVU firmware shows the double-reset countdown, and a double press of RESET continues the restore.
- **Offline restore.** With the computer's network disconnected (Wi-Fi off, cable out), a first start after installing extracts the tools (or **Repair Arduino tools** extracts them again), and **Restore WVU Firmware** compiles and uploads. The restore log (`firmware_job_*.json`) shows Arduino CLI and core versions matching the runtime manifest (revision 4: CLI `1.6.0`, core `1.6.0`) and the reference `.ino` SHA-256 from `release/release-manifest.json`.
- The window stays responsive (plots, scrolling, other cards) during Verify, Restore, and Start.

**Outputs**

- D4/D5/D6 read LOW after Stop, after a fault, and after the 5 s watchdog. To trigger the watchdog, end the app's process during a recording.
- Pulse-ox RED and IR timing is correct, and RED and IR are never HIGH together (scope or logic analyzer).

## Installer and distribution

- Validate the Arduino runtime manifest and bundled archive before running `pwsh -NoProfile -File scripts\build_student_release.ps1` (PowerShell 7.2, clean Git work tree). The script builds the NSIS setup and the MSI package serially and stages both.
- **Third-party source bundle.** The release script builds `dist\WVU-Bioinstrumentation-Studio-<version>-third-party-sources.zip` and its `.sha256` from `release/third-party-sources.json` (network access and Go; see [Third-party source bundles](DEVELOPER_GUIDE.md#third-party-source-bundles)), or verifies a bundle built earlier with `-ThirdPartySourcesZip <path>` (its `.sha256`, then `build_third_party_sources.ps1 -VerifyBundle`: every file against `SHA256SUMS.txt`, the manifest copy, every pinned component and vendor tree). `release/third-party-sources.json` must name this version (`bundle_name`, `applies_to.app_versions`, `installers`) and the pinned runtime. Before publishing, open its `README.txt`: it names this version's installers and runtime, and its written offer points to the "Request corresponding source" issue form.
- Check the checksums:
  - `dist\WVU-Bioinstrumentation-Studio-<version>\SHA256SUMS.txt` covers both installers. The same file is inside the ZIP.
  - `<zip>.sha256` beside the ZIP covers the ZIP.
  - `...-third-party-sources.zip.sha256` covers the source bundle; `sha256sum -c SHA256SUMS.txt` inside the extracted bundle passes.
  - Keep `...-build-info.txt` with the release records. It lists the runtime and source-bundle SHA-256.
- **Attach** the setup, the MSI, the distribution ZIP, the checksum files and the third-party source bundle with its `.sha256` to the GitHub Release. The source bundle must be published with the installers, never later.
- Install the NSIS package on a clean or isolated Windows user if available. Confirm Program Files installation and standard-user operation.
- The MSI ships beside NSIS for managed IT deployment. Test it on a separate clean machine, never on top of an NSIS installation. Confirm the [Installation](INSTALLATION.md) guidance (one installer type per computer) still holds.
- Confirm uninstall preserves Project folders and recordings.
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
- MSI to MSI. First read `UpgradeCode` from the previous release's MSI `Property` table (for example with Orca) and confirm it equals `bundle.windows.wix.upgradeCode` in `src-tauri/tauri.conf.json`. The previous MSI install is removed.
- MSI to NSIS. The NSIS setup detects the MSI installation, removes it before installing, and leaves one Installed-apps entry.

**1.0.x to 1.1.** The publisher changed from `wvu` to the department. The custom NSIS template (`src-tauri/nsis/README.md`) falls back to the 1.0.x registry key and publisher, so run every upgrade above with 1.0.2 as the previous release:

- NSIS 1.0.2 to NSIS 1.1 from the default folder. To exercise the last-resort fallback once, repeat it after deleting `HKLM\SOFTWARE\wvu\WVU Bioinstrumentation Studio`: the setup must then take the folder from the registered `UninstallString`.
- NSIS 1.0.2 to NSIS 1.1 from a custom folder. The candidate must propose and keep the custom folder.
- MSI 1.0.2 to MSI 1.1. The WiX folder search uses the publisher key, so a custom 1.0.2 MSI folder is expected not to be reused: 1.1 installs into the default folder. Record what happens.
- MSI 1.0.2 to NSIS 1.1. The NSIS setup must detect the 1.0.2 MSI, which is registered with Publisher `wvu`.

After the NSIS upgrades, `HKLM\SOFTWARE\West Virginia University Department of Chemical and Biomedical Engineering\WVU Bioinstrumentation Studio` holds the install folder. `HKLM\SOFTWARE\wvu\WVU Bioinstrumentation Studio` may remain: the 1.0.2 uninstaller removes it only when **Delete the application data** is ticked. Make sure the release notes and [Installation](INSTALLATION.md) match what the tests show.

## Documentation

- If the visible UI changed, refresh the README screenshots (see [Developer Guide](DEVELOPER_GUIDE.md#driving-and-screenshotting-the-window)). The Board card screenshot needs real hardware and the real runtime.
- Update the user guides, [USB Protocol](USB_PROTOCOL.md), and [Architecture](ARCHITECTURE.md) for any behavior the release changes.

## Publication review

- Confirm a project-owner-approved root `LICENSE` exists before granting public reuse rights, and that `NOTICE` and `tauri.conf.json` (`publisher`, `copyright`) name the approved copyright holder and year.
- Confirm third-party notices for the bundled runtime: `BUNDLED_COMPONENTS.txt` and [Third-Party Notices](THIRD_PARTY_NOTICES.md) name the bundled versions, and the release carries the matching third-party source bundle.
- The 1.0.x source bundle (`release/third-party-sources-runtime3.json`) is a separate decision: attaching it to the published v1.0.0-v1.0.2 releases changes those releases and needs the maintainer's explicit go-ahead. Build it with `-Runtime` pointing at the v1.0.2 runtime archive (SHA-256 `4e068c54…`, from the private recovery archive) so its runtime pin is checked too, and check the result with `-VerifyBundle` before uploading it. The 1.1 notices do not depend on that decision: `BUNDLED_COMPONENTS.txt` and the 1.1 bundle's README point to `release/third-party-sources-runtime3.json` and the issue form for OpenOCD and dfu-util, not to the 1.0.x bundle.
- Confirm no secrets, recordings, personal calibration files, or developer paths are tracked.
- Confirm branding and code-signing decisions with the repository owner.
