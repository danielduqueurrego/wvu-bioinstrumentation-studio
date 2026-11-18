# Release Checklist

## Source and verification

- Set the intended application version in `package.json`, the `package-lock.json` root (both `version` fields), `src-tauri/Cargo.toml`, the `src-tauri/Cargo.lock` root package, `src-tauri/tauri.conf.json` and `release/release-manifest.json`; add a `## <version>` heading to `CHANGELOG.md` and a `RELEASE_NOTES_<version>.md`.
- Run `pwsh -NoProfile -File scripts\check_release_consistency.ps1`. It must report no mismatch in those versions, the firmware protocol/build/device ID, the reference `.ino` SHA-256, or the Arduino CLI and core versions.
- Run `npm ci`.
- Run `npm run check`, then `npm run build`, then `npm test` (the tests need the built frontend).
- Run `cargo fmt --manifest-path src-tauri\Cargo.toml -- --check`.
- Run `cargo check`, `cargo test` and `cargo clippy` with `--locked` (Clippy with `--all-targets -- -D warnings`).
- Run `npm run tauri build`; do not substitute `cargo build --release` for a production application build.
- Launch the resulting Tauri release executable with no Vite or Node development server running. It must load bundled frontend assets and never request localhost.

## Functional smoke

- Confirm board discovery, firmware verification, and firmware restore work without external console windows.
- Record a short ECG session, a second session without restart, and a short multi-channel EMG session.
- Confirm raw pulse-ox fields, safe LED behavior, absolute phase scheduling, measured per-cycle timestamps, and explicit acquisition-queue overflow accounting with the appropriate hardware setup.
- At a 1000 µs pulse-state dwell, confirm a physical 60-second capture has a median cycle interval near 4000 µs, remains within 1% of 250 cycles/s, has no cycle interval outside ±5%, and has no cycle-index-modulo-10 timing pattern.
- Confirm final metadata reports zero per-state intervals outside ±5% for RED, DARK 1, IR, and DARK 2; review each state's maximum absolute timing error.
- Confirm final metadata timing diagnostics agree with the recorded timestamps and that optional recording notes are preserved exactly.
- Confirm new BMEG, CSV, and metadata filenames identify the lab and contain no development-phase label.
- Open Instructor Manage Labs and verify that reading/selecting labs creates no version.

## Installer and distribution

- Validate the Arduino runtime manifest and bundled archive before running `pwsh -NoProfile -File scripts\build_student_release.ps1` (PowerShell 7, clean Git work tree).
- Check the checksums: `dist\WVU-Bioinstrumentation-Studio-<version>\SHA256SUMS.txt` covers the two installers (the same file is inside the ZIP), and `<zip>.sha256` beside the ZIP covers the ZIP. Keep `...-build-info.txt` with the release records.
- If the Op Amp Lab installer scripts changed, confirm `pwsh -NoProfile -File scripts\package_op_amp_lab_installer.ps1 -Check` passes.
- Install the NSIS package on a clean or isolated Windows user if available; confirm Program Files installation and standard-user operation.
- Confirm uninstall preserves Project folders and recordings.
- Regenerate installer, MSI, ZIP, manifest, and SHA-256 values for each candidate.
- Verify the icon, no-console behavior, offline startup, and student-facing copy.

## Publication review

- Confirm a project-owner-approved root `LICENSE` exists before granting public reuse rights.
- Confirm third-party notices for the bundled runtime.
- Confirm no secrets, recordings, personal calibration files, or developer paths are tracked.
- Confirm branding and code-signing decisions with the repository owner.
