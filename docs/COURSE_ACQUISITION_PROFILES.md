# Course acquisition profiles

These profiles support BMEG 420L teaching capture. They are not medical-device configurations,
do not produce diagnoses, and retain raw ADC counts as the authoritative data.

| Profile | Analog pins | Digital pins | Logical rate | ADC | Recorded raw variables |
| --- | --- | --- | --- | --- | --- |
| General Analog — Development | instructor draft: 1–6 unique A0–A5 | none | supported list, up to 1000 frames/s | 12 or 14 | profile-defined channel fields |
| ECG — Course Capture | A0 = ECG | none | 1000 frames/s | 12 | `ecg_counts` |
| EMG + Force — Course Capture | A0 raw EMG; A1 analog rectified EMG; A2 envelope; A3 pressure/force surrogate | none | 1000 frames/s | 12 | `raw_emg_counts`, `rectified_emg_counts`, `emg_envelope_counts`, `pressure_counts` |
| Blood Pressure + PPG — Course Capture | A0 PPG; A1 MPXV/reference pressure; A2 XGZP/instrumented pressure | D4 GREEN, active HIGH only while acquiring | 200 frames/s | 12 | `ppg_counts`, `mpxv_counts`, `xgzp_counts` |
| Pulse Oximetry — TX + RX Raw Capture | A0 transmission TIA; A1 reflectance TIA | D5 RED; D6 IR, active HIGH by state | 250 cycles/s (1 ms/state) | 14 | `red_TX`, `dark1_TX`, `ir_TX`, `dark2_TX`, `red_RX`, `dark1_RX`, `ir_RX`, `dark2_RX` |

The app displays raw counts or direct Arduino-input volts only. It does not calculate heart rate,
SpO2, blood pressure, EMG activation, fatigue, calibration, or any clinical result. Follow BMEG
420L lab safety procedures; do not use the application for diagnosis or clinical decisions.

Built-in profiles are locked and include a SHA-256 integrity check. This detects accidental or
tampered content, not authorship or authorization. Instructor mode can create a separate general
development draft with a custom unique A0–A5 map; it cannot alter a built-in profile in place.

Each recording freezes the selected profile snapshot, firmware identity, analog/digital mapping,
logical rate, ADC resolution, markers, stop reason, and integrity counters. BMEG is authoritative;
CSV preserves the raw record/cycle sequence and profile-specific field order.
