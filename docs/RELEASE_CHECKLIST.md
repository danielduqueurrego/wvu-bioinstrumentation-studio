# Student Distribution Release Checklist

## Package

- [ ] Version, identifier, icons, and Windows metadata verified.
- [ ] Pinned Arduino CLI and UNO R4 runtime assets verified against `runtime-manifest.json`.
- [ ] NSIS and MSI installers built.
- [ ] Distribution ZIP and SHA-256 checksums produced.
- [ ] Generated installers, runtime caches, recordings, and ZIPs are not staged in Git.

## Student experience

- [ ] Home, Firmware, Acquisition, and Diagnostics use student-facing copy.
- [ ] Advanced technical details are collapsed by default.
- [ ] Startup scan, Refresh Board, verify, compile, upload, and restore show no external console window.
- [ ] First start prepares included Arduino tools and does not rely on Arduino IDE or global Arduino15 settings.
- [ ] One connected UNO R4 WiFi is selected and verified without firmware modification.

## Acceptance

- [ ] Firmware compile, upload, and restore pass.
- [ ] ECG, EMG, BP, and pulse-ox acquisition/export smoke checks pass.
- [ ] Instructor Manage Labs remains usable.
- [ ] UI matrix completed at the actual Windows scaling.
- [ ] Clean-install or isolated-runtime result recorded.
