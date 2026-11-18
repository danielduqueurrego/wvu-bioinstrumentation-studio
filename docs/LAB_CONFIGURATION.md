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

The pulse-ox lab uses a fixed `RED → DARK 1 → IR → DARK 2` sequence with a 1000 µs nominal dwell. The firmware advances phase deadlines on one absolute schedule rather than scheduling from the end of ADC and bookkeeping work. RED and IR are never driven HIGH at the same time. Each completed cycle carries its measured timestamp; the requested cycle rate remains configuration metadata rather than a replacement for measured timing.

Recording metadata preserves the selected profile's bench-only status, human-connection authorization, course-notice acknowledgement, and non-medical validation status. The acquisition screen does not ask students to classify the connected signal separately.

## Instructor revisions

In Instructor mode, **Manage Labs** opens an in-memory draft. Only explicit actions such as **Save changes**, Duplicate, Import, Retire, Restore, Set active version, or Restore course default write the catalog. Reading, selecting, acquiring, plotting, calibrating, or navigating does not create a version.

For simultaneous analog labs, an instructor can configure one to six unique A0–A5 channels, machine-safe channel/CSV names, rate, supported ADC resolution, conversion capability, visibility, and plot assignment. D4/D5/D6 can be configured only with supported safe behaviors: LOW, HIGH while recording, or the fixed pulse-ox sequence. The saved output list is exactly what is driven: an unchecked output stays LOW.

Rules for a revision saved or imported now (labs that are already installed keep loading unchanged):

- CSV fields use 1–64 letters, digits, or underscores (for example `01_counts`). Every generated column must be distinct, ignoring case: the raw field, its `_V` column, and the `_kPa`/`_mmHg` columns of an MPXV-capable channel, and none may reuse `record_sequence`, `t_us`, or `cycle_index`. If a student's linear calibration unit would repeat an existing column (for example volts next to `_V`), the CSV names that column `<name>_cal_<unit>`; unsafe names in older labs are quoted in the CSV header.
- Simultaneous frame rates are the firmware's 100, 200, 250, 500, or 1000 frames/s. The pulse-ox cycle rate is `round(1,000,000 / (4 × dwell))` and dwell is 250–5000 µs.
- Pulse-ox labs have no analog channels and only their RED and IR outputs. Pulse-ox acquisition never used a channel list, so the unused `Channel 1 / A0` that the v1.0.2 editor stored in pulse-ox labs is dropped when such a package is imported (and when the lab is next edited); it is not a reason to refuse the package. RED/IR come from an optional `pulse_ox_role` (`"red"`/`"ir"`) on the output; without it, the label `Red LED` / `IR LED` (any letter case) decides.
- Names, labels, descriptions, and safety notices have length limits.

A lab's `allowed_duration_modes` and `minimum_custom_duration_seconds` are enforced: the page offers only the allowed modes, and Start refuses any other duration. A preset duration is always allowed.

A new revision is numbered one patch above the highest existing revision with the same ID and MAJOR.MINOR (factory, retired, and local revisions all count), so editing after **Restore course default** or after activating an older revision works.

## Catalog file

The catalog (`lab_state.json`) is replaced atomically, and writes from two open windows are serialized; a window always continues from the other window's saved revisions. If one stored lab cannot be loaded, only that lab is hidden: it stays in the file, the application log records `LAB_PROFILE_SKIPPED`, and the Lab panel shows a notice in both modes. When `lab_state.json` is missing or unreadable after an interrupted save, a complete leftover copy is restored (an unreadable file is kept as `lab_state.corrupt-<time>.json`). Only a file that was read but does not parse is replaced this way; a catalog that cannot be read at all (for example while another program holds it) is never replaced, and only the factory labs are shown until the next start. Leftover copies that cannot be read are renamed to `lab_state.corrupt-<time>-<n>.json` and reported to instructors instead of being deleted. **Reset local customizations** first saves the catalog as `lab_state.reset-<time>.json` and reports that path.

## Compatibility and import/export

The connected firmware capabilities are checked before recording. A lab can be edited without a connected board, but an unsupported configuration cannot start. Imported lab JSON is validated and never silently overwrites a conflicting version; re-importing a lab that is already installed (for example an exported instructor lab now stored as imported) is reported as already installed. Factory defaults are always available; local customizations are distinct from immutable shipped definitions.
