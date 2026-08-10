# Phase 5 manual UI verification

Status: in progress. Initial BP completed-recording calibration attempt exposed a command-payload
defect; corrected release-app verification remains pending.

| Viewport | Windows scaling | ECG | EMG | BP calibration | Pulse ox | Overflow/clipping | Result |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 900 x 650 | pending | pending | pending | pending | pending | pending | pending |
| 1024 x 768 | pending | pending | pending | pending | pending | pending | pending |
| 1366 x 768 | pending | pending | pending | pending | pending | pending | pending |
| 1920 x 1080 | pending | pending | pending | pending | pending | pending | pending |
| maximized | pending | pending | pending | pending | pending | pending | pending |

Required checks: unit selectors change only display scale/label; plot grouping remains intact;
MPXV Counts/Volts/kPa/mmHg works; XGZP mmHg appears only after saving a linear calibration;
the dialog validates interval/manual points; and pulse-ox retains eight raw states with no SpO2,
PI, R, or heart-rate control.

## Calibration-dialog defect and correction

The initial release-app attempt selected **Use completed synchronized BP recording** and pressed
**Calculate linear fit**, but no result appeared. The UI sent nested request properties in
camelCase (`bmegPath`, `startSeconds`, and related fields), while the Rust/Tauri request model
expects the serde snake_case names. The command therefore failed before reading the BMEG file;
the error was only written to the page status behind the open dialog, making the action look
inactive.

The correction constructs the nested payload with `bmeg_path`, `start_seconds`, `end_seconds`,
`adc_reference_v`, and `mpxv_sensor_supply_v`, and renders a modal-visible `role=alert` error on
future failures. A frontend payload-shape regression test and a Rust persisted-BMEG,
selected-interval fit test cover the path. The corrected release binary still requires manual
retest after the prior executable is closed.
