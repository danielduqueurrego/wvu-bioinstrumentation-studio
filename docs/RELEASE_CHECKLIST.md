# Student Distribution Release Checklist

## Package

- [ ] Version, identifier, icons, and Windows metadata verified.
- [ ] Pinned Arduino CLI and UNO R4 runtime assets verified against `runtime-manifest.json`.
- [ ] NSIS and MSI installers built.
- [ ] Distribution ZIP and SHA-256 checksums produced.
- [ ] Generated installers, runtime caches, recordings, and ZIPs are not staged in Git.

## Student experience

- [ ] Single-window Board, Project folder, and Acquisition workflow uses student-facing copy.
- [ ] Advanced technical details are collapsed by default.
- [ ] Startup scan, Refresh Board, verify, and restore show no external console window.
- [ ] First start prepares included Arduino tools and does not rely on Arduino IDE or global Arduino15 settings.
- [ ] One connected UNO R4 WiFi is selected and verified without firmware modification.
- [ ] Launch the final Tauri-built release executable with no Vite/dev server running. The bundled
  UI loads without requesting localhost.
- [ ] A detected UNO with **Firmware update required** still permits Board selection, Refresh Board,
  Verify Firmware, and Restore WVU Firmware.
- [ ] After Verify Firmware or Restore WVU Firmware succeeds, a valid Project/Output folder and
  selected course lab can start a new recording without restarting the application.
- [ ] A blocked Start shows a concise, actionable student message; **Advanced details** retains the
  recording-start stage, code, selected board/lab, and exact technical detail.

## Acceptance

- [ ] Firmware verify and restore pass.
- [ ] ECG, EMG, BP, and pulse-ox acquisition/export smoke checks pass.
- [ ] Instructor Manage Labs remains usable.
- [ ] UI matrix completed at the actual Windows scaling.
- [ ] Clean-install or isolated-runtime result recorded.
