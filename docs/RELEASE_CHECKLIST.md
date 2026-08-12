# Release Checklist

## Source and verification

- Confirm `package.json`, Tauri configuration, release manifest, and release notes use the intended application version.
- Run `cargo fmt --manifest-path src-tauri\Cargo.toml -- --check`.
- Run Cargo check, test, and Clippy with `-D warnings`.
- Run `npm run check`, `npm test`, and `npm run build`.
- Run `npm run tauri build`; do not substitute `cargo build --release` for a production application build.
- Launch the resulting Tauri release executable with no Vite or Node development server running. It must load bundled frontend assets and never request localhost.

## Functional smoke

- Confirm board discovery, firmware verification, and firmware restore work without external console windows.
- Record a short ECG session, a second session without restart, and a short multi-channel EMG session.
- Confirm raw pulse-ox fields and safe LED behavior with the appropriate setup or simulator.
- Open Instructor Manage Labs and verify that reading/selecting labs creates no version.

## Installer and distribution

- Validate the Arduino runtime manifest and bundled archive before running `scripts\build_student_release.ps1`.
- Install the NSIS package on a clean or isolated Windows user if available; confirm Program Files installation and standard-user operation.
- Confirm uninstall preserves Project folders and recordings.
- Regenerate installer, MSI, ZIP, manifest, and SHA-256 values for each candidate.
- Verify the icon, no-console behavior, offline startup, and student-facing copy.

## Publication review

- Confirm a project-owner-approved root `LICENSE` exists before granting public reuse rights.
- Confirm third-party notices for the bundled runtime.
- Confirm no secrets, recordings, personal calibration files, or developer paths are tracked.
- Confirm branding and code-signing decisions with the repository owner.
