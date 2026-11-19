# Calibration and Units

## Raw data

Raw ADC counts remain authoritative in BMEG. Display units and derived CSV columns never replace the acquired samples.

Counts are converted to Arduino input volts with:

`V = counts × Vref / (2^bits − 1)`

`Vref` is the **ADC reference voltage (V)** field in the **Calibration & Units** card. It defaults to 5 V and accepts 0.1–10 V. The MPXV sensor supply `Vs` field uses the same range. The value in use when you select **Start recording** is frozen into the new recording with the rest of the calibration snapshot.

A lab definition also contains `display.voltage_reference_v`. The application checks that it is positive but does not use it for any conversion: an instructor lab that sets 3.3 there still converts with the Calibration & Units field (5 V unless the student changes it). The field stays in the schema because it is part of every lab's canonical hash.

## MPXV pressure conversion

For channels configured as MPXV pressure inputs, the app can display:

`P_kPa = (Vout / Vs − 0.04) / 0.009`

`P_mmHg = 7.5006 × P_kPa`

The sensor supply `Vs` is configurable and stored with the recording. Values are not silently clamped.

The EMG + Force pressure channel supports ADC counts, volts, pressure in kPa, and pressure in mmHg using this same MPXV conversion. It remains a pressure display; the application does not infer muscular force.

Newest-value labels in live plots use the currently selected display unit. Live plot x-axis labels show elapsed recording seconds from 0 rather than calendar timestamps. These conversions are display and export layers only and never mutate raw ADC counts.

**Which units a signal offers** follows the signal, not the lab category. The eight raw fields of a legacy 4-state pulse-ox lab (`red_TX` … `dark2_RX`) offer counts and volts only. Every other channel, including each channel of an LED-sequenced pulse-ox lab, offers what its lab's `allowed_conversions` allow (counts and volts, plus MPXV pressure or a linear calibration where allowed). The unit selectors list lab channels: the four LED-state traces of a channel shown by LED state use that channel's unit and calibration, and the LED state itself is not a signal and has no unit.

## XGZP linear calibration

For Blood Pressure + PPG, students can fit the XGZP channel against the synchronized MPXV reference over an interval of the last recording. In the Calibration & Units card, choose **Calibrate XGZP**, then **Use completed synchronized BP recording**:

`MPXV_mmHg = slope × XGZP_volts + offset`

**Interval.** The interval is in seconds from the recording's first record. For a hardware recording, the default start skips the first 0.1 s: this is the startup ADC settling sample that the live plot hides. The recording itself keeps that sample.

**Vref and Vs.** The fit uses the ADC reference and MPXV supply currently entered in Calibration & Units, not necessarily the values frozen in that recording. The dialog states which values it used and names any value that differs from the recording's.

**Result.** The dialog reports slope, offset, R², paired-sample count, and interval. R² is informational; the application does not decide whether a calibration is acceptable. A fit from a recording is always saved as pressure in mmHg.

Saved presets are local to the current Windows user and are compatible only with their specified lab and channel. The XGZP mmHg display becomes available only when a compatible local linear calibration is selected. Choosing **None — counts/volts only** switches that channel's display back to volts.

## Generic linear calibration

On a channel that allows linear calibration, instructors or students can fit a simple line from two or more manually entered points. Choose **Calibrate channel** (**Calibrate XGZP** in the blood-pressure lab), then **Enter manual linear points**. Enter one "channel volts, reference value" pair per line. Separate the two numbers with a comma, a semicolon, or a tab (for example, pasted from a spreadsheet). With a semicolon or tab, a decimal comma is accepted (`0,80;20`). With a comma separator, use a decimal point. Every line must contain exactly two numbers; otherwise the dialog names the line.

Enter the output quantity and units. The blood-pressure lab's XGZP channel starts with `pressure` and `mmHg`; other channels start empty. Nonlinear fitting is not included. The fit is refused when the input voltages do not vary enough or a result is not finite.

Each channel has at most one active calibration. A selected linear calibration replaces the fixed MPXV equation on its channel. Deleting a saved calibration asks for confirmation first.

The list follows the saved files. A preset deleted in another window of the application, or whose file was removed, disappears from the list (and is deselected) the next time the list is read: when a calibration is selected or saved, the channel changes, or the window regains focus while nothing is recording.

## Recording files

Each recording produces three files in the effective session folder (Project folder plus Output folder). They are named `YYYYMMDD_HHMMSS_<lab export name>_RunNN` with the local time at which the session opened, for example `20261118_043539_blood_pressure_ppg_course_capture_Run01`:

| File | Content |
| --- | --- |
| `.bmeg` | Authoritative raw recording |
| `.csv` | Derived table, generated from the finished `.bmeg` |
| `.metadata.json` | Final metadata sidecar |

`RunNN` (01–99) is the first number whose files, and their `.part` temporaries, do not exist yet. Existing files are never overwritten.

### BMEG

A BMEG file has this layout:

| Part | Content |
| --- | --- |
| Magic | ASCII `BMEGREC1` (8 bytes) |
| Header length | u16, little-endian |
| Header | UTF-8 JSON, at most 65,535 bytes |
| Records | Streamed records, little-endian |

A synchronized record is `u32 sequence, u64 timestamp_us, u16 status_flags, u16 counts[field_count]`, that is `14 + 2 × field_count` bytes. The fields are:

- **Simultaneous:** one per channel, in the lab's channel order.
- **Legacy 4-state pulse ox:** `field_count` 8, in the CSV order below.
- **LED-sequenced pulse ox:** `field_count` is 1 + the channel count: the LED state code (0–3), then each channel's raw counts in the lab's channel order.

Older single-channel files use `u32 sequence, u64 timestamp_us, u16 counts`.

The JSON header is the metadata as it stood at Start, plus `record_layout`. It holds:

- `utc_start`, stamped just before START
- `session_opened_utc`
- The board, firmware, and tool provenance
- The frozen lab snapshot
- The calibration snapshot
- For an LED-sequenced lab, `led_sequenced_layout`: `record_fields` (`led_state`, then each channel's CSV field), `led_state_names` (`RED`, `DARK1`, `IR`, `DARK2`), `states_per_cycle` (4), and `state_dwell_us`. Other recordings have no such key.
- `recording_status` and `completion_status` `"active"`
- `total_samples` 0 and no markers

The header is never rewritten. While recording, the file is named `.bmeg.part`. It is renamed to `.bmeg` once it has been flushed, with a bounded retry if another program briefly holds it. A raw file that could not be finished keeps its `.bmeg.part` name, and that name is recorded.

### Metadata sidecar

`<name>.metadata.json` holds the final state. It is written **before** the CSV, so a CSV failure can never leave a recording without metadata. After the CSV attempt it is rewritten to record the CSV result. Both writes go through a temporary file that is flushed and then renamed into place.

Besides the header fields, the sidecar adds:

| Field | Content |
| --- | --- |
| `utc_stop`, `local_stop` | When the recording stopped |
| `host_elapsed_seconds`, `board_elapsed_seconds` | Elapsed time on the computer and on the board |
| `measured_sample_rate_hz`, `timing_diagnostics` | Measured rate and interval statistics; the ±5% tolerance count; for pulse ox, the per-state checks. From firmware build `0x00010007`: `firmware_skipped_deadlines` (sampling deadlines the board skipped) and, for pulse ox, `minimum_pulse_settle_us` (the shortest time from an LED edge to that state's sample). Each is absent when the board did not report it. |
| `total_samples` | Number of records |
| `integrity` | Packet and sample counters, including `overrun_flagged_records` (records of batches the board flagged after dropping a batch; firmware up to build `0x00010006` also flagged skipped deadlines this way), `deadline_skip_flagged_records` (records of batches flagged after skipped deadlines, build `0x00010007`), and `led_state_mismatches` (LED-sequenced records whose LED state is not `record_sequence % 4`; the records are kept unchanged) |
| `duration_mode`, `requested_duration_seconds` | The duration that was set |
| `stop_reason` | How the recording ended: `user`, `timed_complete`, `application_close`, `disconnect`, `storage_guard`, or `fault` |
| `recording_status` | The same, as `stopped_by_user`, `complete`, `application_close`, `disconnected`, `storage_guard`, or `faulted` |
| `completion_status` | `complete` or `incomplete` |
| `initial_free_disk_bytes`, `final_free_disk_bytes` | Free disk space at start and end |
| `markers` | Each marker's `timestamp_us` (board time of the newest received record when **Add marker** was selected) and optional `label` |
| `acquisition_started_utc` | Desktop time at which the board confirmed START |
| `bmeg_filename` | The raw file actually kept (a `.bmeg.part` name if it could not be renamed) |
| `csv_filename` | The CSV file name, or `null` when no CSV was written |
| `csv_error` | Present only when no CSV was written: why |
| `finalize_error` | Present only when flushing, renaming, or writing the files had a problem |

The **Last recording** card reads these results. It marks each file ✓ Saved or ✗ Not saved and shows `csv_error` or `finalize_error` as the reason.

### CSV

The CSV is streamed from the finished BMEG with the Vref, Vs, display units, and calibrations frozen in the recording. It is written as `.csv.part` and renamed only when its row count matches the BMEG record count; otherwise the sidecar records a `csv_error` and no CSV is kept. Free-text notes and markers are not written to the CSV (they are in the sidecar), and record status flags stay in the BMEG.

For **simultaneous** labs the header is `record_sequence,t_us,` followed by these columns for each channel, in the lab's channel order:

| Column | When | Content |
| --- | --- | --- |
| `<csv_name>` (for example `mpxv_counts`) | Always | Raw ADC counts |
| `<base>_V` | Always | Arduino input volts, 6 decimals |
| `<base>_kPa` | Channels converted with the MPXV equation | MPXV pressure in kPa |
| `<base>_mmHg` | Also on MPXV-equation channels, if the channel ID is `mpxv` or the channel's display unit was mmHg at Start | MPXV pressure in mmHg |
| `<base>_<units>` | Channels with a selected linear calibration | The calibrated value, named from the calibration's units (letters and digits only; `calibrated` if none remain) |

`<base>` is the CSV name without a trailing `_counts`. A channel offers the MPXV equation when its lab allows MPXV pressure; in older labs without that setting, the channel IDs `mpxv` and the EMG + Force `pressure` channel decide.

If a calibrated column would repeat an existing name (for example a calibration in volts next to `_V`), it is named `<base>_cal_<units>`. Any further repeat gets `_2`, `_3`, and so on.

Header cells are written so that they cannot shift columns or run as a spreadsheet formula. A name starting with `=`, `+`, `-`, `@`, a tab, or a carriage return gets a leading `'`. A name containing a comma, a double quote, or a line break is quoted (RFC 4180). Factory names need neither. Data cells are plain numbers.

For example, a Blood Pressure + PPG recording with the default units and no XGZP calibration has:

```text
record_sequence,t_us,ppg_counts,ppg_V,mpxv_counts,mpxv_V,mpxv_kPa,mpxv_mmHg,xgzp_counts,xgzp_V
```

A selected XGZP calibration in mmHg adds `xgzp_mmHg`.

For **LED-sequenced pulse-ox** labs (the built-in pulse-ox lab 1.1.0, and instructor labs in that mode) there is one row per LED state. The header is `record_sequence,t_us,cycle_index,led_state,led_state_name,` followed by the same per-channel columns as a simultaneous lab:

| Column | Content |
| --- | --- |
| `record_sequence` | Record number from 0 at START, one per LED state |
| `t_us` | Board timestamp of the state's sample, taken just before its first channel read |
| `cycle_index` | `record_sequence / 4` (integer division): the RED, DARK 1, IR, DARK 2 cycle the row belongs to |
| `led_state` | The state code the board recorded: 0 RED, 1 DARK 1, 2 IR, 3 DARK 2 |
| `led_state_name` | `RED`, `DARK1`, `IR`, or `DARK2`; a code outside 0–3 is written as-is with the name `UNKNOWN` |
| Channel columns | Each channel's raw counts, `_V`, and calibration columns, as in the simultaneous table above |

For example, the built-in lab (TX on A0, RX on A1, 14-bit, 5 V reference). The counts below are illustrative:

```text
record_sequence,t_us,cycle_index,led_state,led_state_name,tx_counts,tx_V,rx_counts,rx_V
0,5012345,0,0,RED,9830,3.000061,4915,1.500031
1,5013345,0,1,DARK1,1638,0.499908,820,0.250259
2,5014346,0,2,IR,8192,2.500153,4096,1.250076
3,5015345,0,3,DARK2,1200,0.366233,600,0.183117
4,5016345,1,0,RED,9841,3.003418,4920,1.501556
```

Every row holds all channels, read in the same LED state. To work with one LED state, select its rows (for example `led_state == 0` for RED): each state's samples come at a quarter of the row rate (250 per second at the built-in 1000 µs dwell), one per `cycle_index`. `record_sequence % 4` always equals `led_state` in a recording from the reference firmware; the sidecar's `integrity.led_state_mismatches` counts any row where it does not.

For **legacy 4-state pulse-ox** labs (the built-in pulse-ox lab 1.0.0 and older local revisions) the CSV is raw only, one row per cycle:

```text
cycle_index,t_us,red_TX,dark1_TX,ir_TX,dark2_TX,red_RX,dark1_RX,ir_RX,dark2_RX
```

`record_sequence` and `cycle_index` count records from 0 at START (in the 4-state CSV, `cycle_index` is the record number). A gap in `record_sequence` (or in the 4-state `cycle_index`) means records were lost (see the sidecar's `integrity`).

`t_us` is the firmware-measured timestamp in microseconds on the board's clock. It is not elapsed recording time and does not start at 0 for a hardware recording. Subtract the first row's `t_us` to get elapsed time. In simultaneous mode, `t_us` marks the start of each frame. In LED-sequenced mode, it marks the start of each state's channel reads, at the end of that state's dwell. In legacy 4-state mode, it marks the completion of each cycle's DARK 2 reads.

## What the application does not calculate

The application does not calculate ECG heart rate, EMG MVC/fatigue/activation, force, SBP/DBP, SpO2, perfusion index, or other physiological conclusions.

Teaching use only — not a medical device.
