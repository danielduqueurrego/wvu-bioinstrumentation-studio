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
- Run `npm ci`.
- Run `npm run check`, then `npm run build`, then `npm test`. This order matters: a release-configuration test needs the built `build/index.html`, so `npm test` fails on a clean checkout without the build.
- Run `cargo fmt --manifest-path src-tauri\Cargo.toml -- --check`.
- Run `cargo check`, `cargo test`, and `cargo clippy` with `--locked` (Clippy with `--all-targets -- -D warnings`). The Rust version comes from `rust-toolchain.toml`.
- Run `npm run tauri build`; do not substitute `cargo build --release` for a production application build.
- Launch the resulting Tauri release executable with no Vite or Node development server running. It must load bundled frontend assets and never request localhost.
- Use the real, hash-pinned `src-tauri/resources/arduino-runtime.zip`. The CI placeholder satisfies source checks only and must never be packaged or used for hardware tests.

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
- The window stays responsive (plots, scrolling, other cards) during Verify, Restore, and Start.

**Outputs**

- D4/D5/D6 read LOW after Stop, after a fault, and after the 5 s watchdog. To trigger the watchdog, end the app's process during a recording.
- Pulse-ox RED and IR timing is correct, and RED and IR are never HIGH together (scope or logic analyzer).

## Installer and distribution

- Validate the Arduino runtime manifest and bundled archive before running `pwsh -NoProfile -File scripts\build_student_release.ps1` (PowerShell 7, clean Git work tree). The script builds the NSIS setup and the MSI package serially and stages both.
- Check the checksums:
  - `dist\WVU-Bioinstrumentation-Studio-<version>\SHA256SUMS.txt` covers both installers. The same file is inside the ZIP.
  - `<zip>.sha256` beside the ZIP covers the ZIP.
  - Keep `...-build-info.txt` with the release records.
- If the Op Amp Lab installer scripts changed, confirm `pwsh -NoProfile -File scripts\package_op_amp_lab_installer.ps1 -Check` passes.
- Install the NSIS package on a clean or isolated Windows user if available. Confirm Program Files installation and standard-user operation.
- The MSI ships beside NSIS for managed IT deployment. Test it on a separate clean machine, never on top of an NSIS installation. Confirm the [Installation](INSTALLATION.md) guidance (one installer type per computer) still holds.
- Confirm uninstall preserves Project folders and recordings.
- Regenerate installer, MSI, ZIP, manifest, and SHA-256 values for each candidate.
- Verify the icon, no-console behavior, offline startup, and student-facing copy.

## Documentation

- If the visible UI changed, refresh the README screenshots (see [Developer Guide](DEVELOPER_GUIDE.md#driving-and-screenshotting-the-window)). The Board card screenshot needs real hardware and the real runtime.
- Update the user guides, [USB Protocol](USB_PROTOCOL.md), and [Architecture](ARCHITECTURE.md) for any behavior the release changes.

## Publication review

- Confirm a project-owner-approved root `LICENSE` exists before granting public reuse rights.
- Confirm third-party notices for the bundled runtime.
- Confirm no secrets, recordings, personal calibration files, or developer paths are tracked.
- Confirm branding and code-signing decisions with the repository owner.
