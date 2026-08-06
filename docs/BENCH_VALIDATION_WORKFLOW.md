# Bench-validation workflow

## Mandatory safety statement

**Bench-validation use only. No person or electrode system may be connected. Not a medical
device.** The workflow stores raw Arduino counts and direct 0–5 V Arduino input volts only. It
does not calculate heart rate, muscle activation, fatigue, physiological units, or clinical
outputs.

## Instructor workflow

1. Enter the local Instructor authoring workflow guard and select the locked ECG or EMG raw-output
   profile. This is not strong authentication.
2. Verify controlled UNO R4 WiFi firmware compatibility before hardware capture. Simulator runs
   keep their simulator provenance and do not substitute for hardware evidence.
3. Create a draft with module identity, board/COM/firmware identity, safe bench condition, and
   test-equipment metadata. Explicitly acknowledge the no-person/no-electrode condition.
4. Capture each run through the production acquisition controller. Every run is a separate BMEG,
   metadata, and CSV session; raw samples are never concatenated or filtered.
5. Review transparent raw-data metrics and enter local course acceptance criteria. Criteria are
   measured values with an explicit comparison operator, threshold, units, observed result, and
   pass/fail result; they are not manufacturer or clinical limits.
6. Finalize only when all required evidence passes. Export the manifest-verified package and keep
   raw recording references available for review.

## Required test evidence

| Test | Minimum bench evidence | Transparent metrics |
| --- | --- | --- |
| Zero-input / baseline | 10 s | mean, standard deviation, min/max, peak-to-peak, rail/clipping counts, continuity, sample rate |
| DC operating-range sweep | 2 s per entered safe 0–5 V setpoint | mean volts, absolute/percentage error, standard deviation, clipping |
| Known sine acquisition | 10 s with entered offset, frequency, and peak-to-peak value | mean, RMS, min/max, peak-to-peak, sample rate, clipping, raw mean-threshold rising-crossing frequency estimate |
| Saturation margin | documented source condition | min/max voltage, rail headroom, configurable 5% margin counts, exact ADC rails |
| Repeatability | at least three separate runs | individual mean/stddev/peak-to-peak and between-run mean/stddev; coefficient of variation only when mean is not near zero |

The application never drives a function generator or source. It accepts only manually entered
source/equipment metadata. Any source setpoint, offset, or amplitude entered into the simulator
is constrained to 0–5 V; a hardware connection must be made only under a separately verified safe
bench condition.

## Deferred work

Magnitude/phase response, -3 dB measurements, input-referred noise, notch behavior, leads-off,
recovery-time checks, distortion, isolation validation, and any human-signal validation are
deferred beyond Phase 3B.
