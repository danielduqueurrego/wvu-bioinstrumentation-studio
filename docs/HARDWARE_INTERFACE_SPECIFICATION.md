# Hardware interface specification

## 1. Supported controller

- Arduino UNO R4 WiFi
- FQBN: `arduino:renesas_uno:unor4wifi`
- USB communication only for version 1
- One board at a time
- Default ADC resolution: 12 bits
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

### ECG & EMG

- Signal: A0
- Optional notch: A1
- LOD+: D2
- LOD-: D3
- Event marker: D4

For Phase 3A, the locked raw ECG/EMG profiles use only A0 at 12 bits and 1000 samples/s for a
simulator, UNO-alone, or safe 0–5 V direct bench-input test. Optional notch and leads-off inputs
are deferred. No person, electrode system, or module-to-person connection is authorized.

### Pulse Oximetry

- Reflection: A0
- Transmission: A1
- Red: D4
- Infrared: D5
- Green: D6

### Blood Pressure

- MPXV or XGZP conditioned output: A0
- Optional second pressure channel: A1
- Event marker: D4

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

Default optical frame rate: 100 Hz (10 ms/frame).

### Standard mode

```text
All off / dark settle: 250 us
Dark acquisition:      500 us
Red settle:             250 us
Red acquisition:        750 us
All off / dark settle:  250 us
Dark acquisition:       500 us
IR settle:              250 us
IR acquisition:         750 us
All off / idle:         remainder of 10 ms
```

During each acquisition window, begin with four conversions per available detector channel and average them in normal mode. Diagnostic mode may preserve individual conversions.

### Multicolor mode

Add a third dark/green state while maintaining one active LED at a time. The exact timing remains profile-versioned and must be measured on assembled hardware before being considered final.

## 6. Clipping

For 12-bit ADC data:

- Full scale: 4095 counts
- Preliminary hard clipping warning: <= 41 or >= 4054 counts (approximately 1% from rails)
- Preliminary caution zone: <= 205 or >= 3890 counts (approximately 5% from rails)

Thresholds are configurable until multi-board validation is complete.

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
