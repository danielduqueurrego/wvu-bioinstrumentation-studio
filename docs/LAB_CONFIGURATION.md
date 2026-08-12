# Lab Configuration

Course labs are versioned acquisition definitions. Every recording embeds a snapshot of the selected lab, so later edits never change historical data.

## Factory labs

| Lab | Inputs/outputs | Rate | ADC | Default plots |
| --- | --- | ---: | ---: | --- |
| ECG — Course Capture | A0 ECG | 1000 frames/s | 14 bit | ECG |
| EMG + Force — Course Capture | A0 raw EMG, A1 rectified, A2 envelope, A3 pressure | 1000 frames/s | 14 bit | one plot per signal |
| Blood Pressure + PPG — Course Capture | A0 PPG, A1 MPXV, A2 XGZP, D4 green | 200 frames/s | 14 bit | PPG, MPXV, XGZP |
| Pulse Oximetry — TX + RX Raw Capture | A0 TX, A1 RX, D5 red, D6 IR | about 250 cycles/s | 14 bit | TX raw phases; RX raw phases |
| General Analog — Development | 1–6 unique A0–A5 inputs | 1000 frames/s | 14 bit | one plot per selected input |

The pulse-ox lab uses a fixed `RED → DARK 1 → IR → DARK 2` sequence with a 1000 µs nominal dwell. RED and IR are never driven HIGH at the same time.

## Instructor revisions

In Instructor mode, **Manage Labs** opens an in-memory draft. Only explicit actions such as **Save changes**, Duplicate, Import, Retire, Restore, Set active version, or Restore course default write the catalog. Reading, selecting, acquiring, plotting, calibrating, or navigating does not create a version.

For simultaneous analog labs, an instructor can configure one to six unique A0–A5 channels, machine-safe channel/CSV names, rate, supported ADC resolution, conversion capability, visibility, and plot assignment. D4/D5/D6 can be configured only with supported safe behaviors: LOW, HIGH while recording, or the fixed pulse-ox sequence.

## Compatibility and import/export

The connected firmware capabilities are checked before recording. A lab can be edited without a connected board, but an unsupported configuration cannot start. Imported lab JSON is validated and never silently overwrites a conflicting version. Factory defaults are always available; local customizations are distinct from immutable shipped definitions.
