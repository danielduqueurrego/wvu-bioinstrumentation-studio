# Calibration and Units

## Raw data

Raw ADC counts remain authoritative in BMEG. Display units and derived CSV columns never replace the acquired samples.

Counts are converted to Arduino input volts with:

`V = counts × Vref / (2^bits − 1)`

The default `Vref` is 5.0 V. The configured value and active calibration snapshot are stored with each new recording.

## MPXV pressure conversion

For channels configured as MPXV pressure inputs, the app can display:

`P_kPa = (Vout / Vs − 0.04) / 0.009`

`P_mmHg = 7.5006 × P_kPa`

The sensor supply `Vs` is configurable and stored with the recording. Values are not silently clamped.

## XGZP linear calibration

For Blood Pressure + PPG, students can fit the XGZP channel against the synchronized MPXV reference over a selected recording interval:

`MPXV_mmHg = slope × XGZP_volts + offset`

The dialog reports slope, offset, R², paired-sample count, and interval. R² is informational; the application does not decide whether a calibration is acceptable. Saved presets are local to the current Windows user and are compatible only with their specified lab and channel.

## Generic linear calibration

Instructors or students can fit a simple line from two or more manually entered voltage/reference-value points. Nonlinear fitting is not included.

## What the application does not calculate

The application does not calculate ECG heart rate, EMG MVC/fatigue/activation, force, SBP/DBP, SpO2, perfusion index, or other physiological conclusions.
