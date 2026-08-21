# Changelog

All notable user-facing changes are documented here.

## 1.0.1

### Fixed

- Stabilized long-running `Until stopped` acquisition on Arduino UNO R4 WiFi and retained precise serial/no-data diagnostics for genuine failures.
- Removed the initial ADC settling transient from the live display without changing authoritative BMEG/CSV samples.
- Changed live plot time labels to elapsed recording seconds starting at zero.
- Ensured startup failures are written to the application log and shown in a native Windows error dialog instead of appearing as a brief console flash.
- Made instructor lab-catalog writes transactional and allowed an explicit factory reset to recover from a malformed local catalog.

### Maintenance

- Added atomic, recoverable Arduino-runtime deployment and bounded application/catalog/firmware logs.
- Strengthened release validation, CI coverage, firmware/runtime hash checks, and bundled third-party notices.

## 1.0.0

### Added

- Synchronized ECG, EMG + pressure, blood pressure + PPG, and raw pulse-ox course acquisition.
- Project/trial folders, recording markers, and automatic BMEG/CSV/metadata output.
- Counts, volts, MPXV pressure conversion in kPa or mmHg (including EMG + Force pressure), and local linear-calibration presets.
- Instructor course-lab management with versioned revisions and factory defaults.
- Bundled Arduino tools, firmware verification, and WVU reference-firmware restoration.
- Safe live plot reconfiguration, configurable plot groups, and per-trace visibility during recording.
- Color-coded legends for multi-signal plots, a synchronized 0.5–30 s live plot window, and newest-value endpoint labels.
- Elapsed-seconds live plot axes starting at the first accepted recording frame, with the initial hardware ADC settling transient excluded from the display only; raw BMEG/CSV data remain unchanged.

### Known limitations

- The application is teaching software and does not provide physiological or clinical interpretation.
- Windows installers remain unsigned unless an approved institutional signing workflow is supplied.
