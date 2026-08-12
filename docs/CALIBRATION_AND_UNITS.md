# Calibration and engineering units

Phase 5 adds a small, course-facing derived-unit layer. It is not a validation system,
does not alter the locked acquisition profile, and does not perform physiological analysis.
Raw ADC counts in BMEG are authoritative and are never overwritten, filtered, removed, or
reinterpreted as a biological quantity.

## Direct Arduino-input volts

For every profile channel the app can show and export Arduino-input volts:

```text
V = counts × Vref / (2^bits − 1)
```

`Vref` defaults to 5.000 V and is editable before a recording. The selected ADC resolution and
the actual Vref assumption are frozen in the BMEG header and metadata sidecar. Older recordings
without `adc_reference_v` remain readable under the documented legacy 5.0 V assumption.

## MPXV pressure conversion

For the BP A1 MPXV reference channel—and the optional EMG A3 pressure-surrogate conversion—the
app can display and export kPa or mmHg using the local course equation:

```text
P_kPa  = (Vout / Vs − 0.04) / 0.009
P_mmHg = 7.5006 × P_kPa
```

`Vs` defaults to 5.000 V and is editable before recording. Values near zero output may be
negative because the equation includes the sensor offset; the app does not silently clamp them.
EMG A3 is labelled **Pressure (kPa)**, never muscular force.

## XGZP linear calibration

The Blood Pressure + PPG profile can fit its synchronized A1/A2 recording over a user-selected
time range:

```text
MPXV_mmHg = slope × XGZP_volts + offset
```

The dialog reports slope, offset, R², and paired-sample count. R² is informational only: the app
does not decide that a calibration is good enough. Students may also enter two or more manual
`volts, reference value` points for an ordinary least-squares linear fit. Local presets are stored
under the application user-data directory, filtered by profile and channel, and can be loaded or
deleted. They are not stored inside immutable course profiles.

## Instructor-declared generic linear channels

An Instructor Lab revision may permit a channel to use a generic linear calibration. Students can
enter two or more `volts, reference value` points, review slope/intercept/R², name the preset,
and choose its engineering quantity and unit text. This is the same lightweight derived-value
path as XGZP; it is not a validation state, does not alter raw BMEG records, and does not make a
physiological claim. The CSV preserves raw and voltage columns and appends the named derived
column only when that calibration is frozen into the recording.

## Recording and export provenance

At recording start the controller freezes:

- ADC reference voltage and MPXV supply voltage;
- selected display units;
- active fixed-formula or saved linear calibration IDs and parameters.

Editing calibration parameters is disabled while recording. Changing a display unit never changes
the recorded raw samples. CSV adds derived columns alongside raw counts (for example,
`mpxv_V`, `mpxv_kPa`, `mpxv_mmHg`, and, when selected, `xgzp_mmHg`). A missing XGZP calibration
never creates an `xgzp_mmHg` column.

Pulse-ox remains raw: each of its eight phase measurements can show counts or volts, while all eight
RED/DARK/IR/DARK raw count fields remain unchanged. The app does **not** calculate ECG heart rate,
EMG MVC/fatigue, SBP/DBP, SpO2, R, perfusion index, or any clinical conclusion.
