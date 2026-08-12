# Changelog

All notable user-facing changes are documented here.

## Unreleased

### Fixed

- EMG + Force pressure display now offers the same MPXV pressure units in kPa and mmHg.
- Reconfiguring live plot groups is isolated from acquisition so a chart refresh cannot interrupt recording.
- Multi-signal live plots now show a color-coded per-plot legend.

## 1.0.0

### Added

- Synchronized ECG, EMG + pressure, blood pressure + PPG, and raw pulse-ox course acquisition.
- Configurable plot groups, recording markers, project/trial folders, and automatic BMEG/CSV/metadata output.
- Counts, volts, MPXV pressure conversion, and local linear-calibration presets.
- Instructor course-lab management with versioned revisions and factory defaults.
- Bundled Arduino tools, firmware verification, and WVU reference-firmware restoration.

### Known limitations

- The application is teaching software and does not provide physiological or clinical interpretation.
- Windows installers remain unsigned unless an approved institutional signing workflow is supplied.
