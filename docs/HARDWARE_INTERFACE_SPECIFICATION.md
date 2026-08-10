# Hardware interface specification

## 1. Supported controller

- Arduino UNO R4 WiFi
- FQBN: `arduino:renesas_uno:unor4wifi`
- USB communication only for version 1
- One board at a time
- Supported ADC resolutions: 12 and 14 bits, as constrained by the active profile
- Expected analog input range for this project: 0–5 V

## 2. Configurable pin philosophy

The biomedical board exposes signals through headers. The application therefore uses profile-constrained configurable mappings rather than hard-coded pins.

### Analog inputs

Allowed values: A0–A5, unless a profile reserves a pin.

Typical signals:

- AD8232/filter output
- Notch output
- Pulse-ox reflection output
- Pulse-ox transmission output
- MPXV5100DP output
- Student-conditioned XGZP output

### Digital inputs

Typical signals:

- LOD+
- LOD-
- Event/synchronization input

Leads-off outputs are digital status signals even when physically routed to a header that can be connected elsewhere.

### Digital outputs

Typical signals:

- Red LED control
- Infrared LED control
- Green LED control

## 3. Default profile mappings

### ECG and EMG / force

- ECG: A0 = conditioned ECG output, 12 bit, 1000 frames/s
- EMG / force: A0 = raw EMG; A1 = analog rectified EMG; A2 = EMG envelope; A3 = pressure/force
  surrogate, 12 bit, 1000 synchronized frames/s

The individual analog conversions are ordered sequentially within a logical frame. They share a
frame sequence/timestamp but are not physically simultaneous conversions. The app preserves raw
counts and Arduino-input volts only; it performs no physiological analysis.

### Pulse oximetry

- Transmission TIA: A0
- Reflectance TIA: A1
- Green: D4
- Red: D5
- Infrared: D6

### Blood pressure + PPG

- PPG: A0
- MPXV/reference pressure: A1
- XGZP/instrumented pressure: A2
- Green LED: D4, active HIGH only while this profile is acquiring

Defaults are editable within profile rules.

## 4. Pulse-oximetry electrical facts

- LED control is active high.
- Only one LED may be active at a time.
- All LED outputs must be low in idle/fault/reset states.
- Measured currents:
  - Green: 6.72 mA
  - Red: 4.24 mA
  - Infrared: 6.16 mA
- Series resistors:
  - Green: 330 ohm
  - Red: 680 ohm
  - Infrared: 560 ohm
- ADA4352-2 outputs: 0–5 V
- TIA gain is selected using an integrated DIP switch and entered manually in software.

## 5. Initial pulse-ox timing

Phase 4 fixed raw cycle rate: approximately 250 cycles/s (1 ms per state, 4 ms per cycle).

```text
RED on:  sample TX/RX
DARK 1:  sample TX/RX
IR on:   sample TX/RX
DARK 2:  sample TX/RX
```

The raw record retains all eight values; ambient-subtracted signals are optional display previews
only. The firmware never drives D5 and D6 HIGH together and forces D4–D6 LOW when idle, stopped,
or faulted. It does not compute SpO2 or heart rate.

## 6. Clipping

For 12-bit ADC data:

- Full scale: 4095 counts
- Preliminary hard clipping warning: <= 41 or >= 4054 counts (approximately 1% from rails)
- Preliminary caution zone: <= 205 or >= 3890 counts (approximately 5% from rails)

Thresholds are configurable according to the current course exercise; the runtime application
does not manage formal hardware-characterization evidence.

## 7. Pressure channels

### MPXV5100DP

Supported display:

- Counts
- Volts
- Nominal kPa
- Nominal mmHg
- Calibrated pressure

Nominal conversion must be clearly labeled and may not be confused with a student calibration.

### XGZP160201S

Students build the bridge excitation and amplifier circuit. The application records:

- Counts
- Volts
- Student-entered circuit notes

No built-in XGZP pressure conversion in v1.
