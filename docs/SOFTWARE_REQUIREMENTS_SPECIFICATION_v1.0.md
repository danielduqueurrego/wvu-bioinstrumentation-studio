# Software Requirements Specification v1.0

## 1. Purpose

WVU Bioinstrumentation Studio is a Windows desktop application for the BMEG 420L biomedical instrumentation laboratory. It provides firmware editing, compilation, upload, synchronized USB acquisition, live plotting, calibration support, recording, diagnostics, and export for an Arduino UNO R4 WiFi and the course biomedical instrumentation hardware.

The system is teaching and engineering equipment, not a medical device.

## 2. Users

### Student user

A student must be able to:

- Select an instructor-approved lab profile.
- Assign exposed hardware signals to permitted Arduino pins.
- Create an editable copy of one approved `.ino` sketch.
- Compile and upload the sketch.
- View raw signals in real time.
- Start and stop a recording.
- Enter group/session identifiers.
- Add event markers.
- Review and export a recording.
- Restore the approved firmware template.

### Instructor/developer user

An instructor must additionally be able to:

- Inspect and modify profile JSON.
- Unlock advanced diagnostics.
- Select allowable sample-rate ranges and pins.
- Inspect profile integrity, firmware, timing, and packet-loss diagnostics.
- View build, upload, protocol, timing, and packet-loss logs.
- Package approved firmware templates and offline dependencies.

## 3. Platform requirements

- Windows 11, 64-bit
- Arduino UNO R4 WiFi
- One available USB data port
- One connected board at a time
- Arduino CLI integration
- Normal application operation without an internet connection after installation

## 4. Functional requirements

### FR-001 Application startup

The application shall start without requiring an Arduino board. It shall expose simulator mode when no supported device is available.

### FR-002 Board discovery

The application shall enumerate serial ports and use Arduino CLI board discovery when available. It shall identify the UNO R4 WiFi and show the COM port, board name, FQBN, and connection state.

### FR-003 Compatibility check

Before acquisition, the application shall verify:

- Supported board type
- Firmware protocol version
- Firmware/profile compatibility
- Configured pin validity
- Available output directory
- Required session metadata
- Locked profile schema/integrity and any profile-required course acknowledgement

### FR-004 Sketch editing

The application shall provide a single-file `.ino` editor with syntax highlighting, line numbers, search, undo/redo, compile-error navigation, restore-template, save-as, and modified-template status.

For the UNO R4 WiFi classroom workflow, a project shall contain exactly one source file named
`<ProjectName>.ino` and a versioned `project.json` in the matching project folder. The application
shall validate Arduino/Windows-safe names, prevent source-path traversal, use an explicit
overwrite confirmation, preserve UTF-8 source without hidden transformations, and warn before
discarding unsaved changes. Controlled templates shall be copied to a student project; they shall
not be modified in place.

### FR-005 Compile and upload

The application shall call Arduino CLI as a subprocess, capture structured output where supported, report build size, show actionable errors, and upload to the selected UNO R4 WiFi. A controlled-firmware
installation shall not be considered verified from uploader exit status alone: the host shall receive
CRC-valid protocol identity frames and verify the expected build/device identity before acquisition.

Compile and upload shall be separate user actions. Upload shall require a current successful
compile of the saved source, explicit confirmation, one selected supported board, and no active
acquisition/recording. The uploader shall release the application serial session first, handle
bounded reset/bootloader/application-port re-enumeration without assuming a fixed COM number, and
record exact argument arrays, output, timing, exit code, board identity, and verification result.
A non-WVU sketch may report successful upload, but it shall disable Acquisition and explain that
the controlled reference must be restored before binary acquisition is possible.

### FR-006 Pin assignment

Analog acquisition inputs shall be configurable among A0–A5, subject to profile restrictions. Digital control/status pins shall be configurable among valid UNO R4 WiFi digital pins. Duplicate or conflicting assignments shall be rejected.

### FR-007 Acquisition

The application shall:

- Start and stop acquisition explicitly.
- Receive versioned binary packets.
- Validate packet integrity.
- Detect missing, duplicate, out-of-order, and corrupt packets.
- Write raw recording data independently of the plotting path.
- Keep recording memory bounded.

### FR-008 Visualization

The application shall:

- Plot live signals at approximately 20–30 display updates per second.
- Keep acquisition rate independent of display rate.
- Use a bounded display history.
- Support visibility, deterministic color, configurable plot groups with independent y axes,
  zoom, and time-window controls.
- Mark clipping and data-loss events.

### FR-009 Recording

The application shall support timed recordings up to at least 600 seconds and a user-selected
**Until stopped** mode. It shall write continuously to disk, keep memory bounded, check available
storage before and during an until-stopped recording, warn below 1 GiB free, and safely finalize
before available storage falls below a 250 MiB critical threshold.

### FR-010 Session metadata

Required session fields:

- Group ID
- Lab profile
- Run number
- Date/time
- Board/port identity
- Firmware version
- Protocol version
- Profile version
- Pin assignments
- ADC resolution
- Requested sample rate
- Measured sample rate
- Packet/sample loss counts
- Duration mode, requested duration when timed, actual duration, and stop reason
- Initial and final observed free disk space when available
- Immutable acquisition-profile snapshot, integrity hash, lock/source state, and acknowledgement

Optional fields:

- Student experiment/session identifier
- Main-board serial
- Module serial
- Notes
- Operator
- Environmental information

### FR-011 File naming

Default filename:

`YYYYMMDD_HHMMSS_<GroupID>_<Lab>_<Profile>_RunNN`

The application shall sanitize invalid Windows filename characters, prevent path traversal, warn before overwrite, and propose the next run number.

### FR-012 Export

The application shall export:

- `.bmeg` binary recording
- `.csv` tabular data
- `.metadata.json`
- `.events.csv`

MATLAB `.mat` export is out of scope.

### FR-013 Diagnostics

The application shall display and export:

- Port and board identity
- Firmware/protocol versions
- Requested/measured sample rate
- Packet counts
- Missing samples
- CRC failures
- Duplicate/out-of-order packets
- Buffer overflows
- Reconnects
- Build/upload logs

### FR-014 Offline operation

Once installed, all required classroom functions shall work without internet access. A later release package shall include pinned Arduino CLI/core/tool dependencies, approved templates, help content, and the required Windows WebView runtime strategy.

### FR-015 Safety state

On firmware startup, acquisition stop, command timeout, communication failure, or reset, all pulse-oximetry LED control pins shall be driven low. Only one optical LED may be active at a time.

## 5. Lab-specific requirements

### ECG & EMG

- Raw signal input: configurable analog pin
- Optional notch-output input: configurable analog pin
- Leads-off signals: digital inputs
- Default ECG sample rate: 1000 samples/s
- Default EMG sample rate: 2000 samples/s
- Default ADC resolution: 12 bits
- Display/export: counts and volts
- No derived heart rate or EMG envelope
- Current course profiles preserve raw ECG/EMG capture variables at their locked profile settings.
  They are teaching tools, not medical devices; no clinical or physiological interpretation is
  available. Formal analog-module characterization is outside this application's runtime scope.

### Pulse Oximetry

- Reflection output: configurable analog pin
- Transmission output: configurable analog pin
- Red, infrared, green control: configurable active-high digital outputs
- Measured LED currents:
  - Green: 6.72 mA
  - Red: 4.24 mA
  - Infrared: 6.16 mA
- Series resistors:
  - Green: 330 ohm
  - Red: 680 ohm
  - Infrared: 560 ohm
- ADA4352-2 output range: 0–5 V
- TIA gain is selected by DIP switch and entered manually
- Default optical frame rate: 100 Hz
- Normal pulse-ox mode: dark, red, dark, infrared
- Optional multicolor mode adds green
- Store raw illuminated and dark measurements
- Optional derived dark-corrected preview
- No clinical SpO2 output

### Blood Pressure

- MPXV5100DP: configurable analog input
- XGZP160201S conditioned output: configurable analog input
- MPXV display modes: counts, volts, nominal kPa, nominal mmHg, calibrated pressure
- XGZP display modes: counts and volts only
- Respiratory-flow features are out of scope

## 6. Nonfunctional requirements

### NFR-001 Responsiveness

The UI shall remain responsive during acquisition, plotting, recording, and export.

### NFR-002 Resource use

Initial low-resource acceptance target:

- 8 GB RAM
- Four-core CPU
- Integrated graphics
- SSD
- No discrete GPU requirement
- Idle memory target under 175 MB
- Bounded memory during 10-minute acquisition

### NFR-003 Data integrity

Initial design target:

- Zero missing samples during a normal 10-minute wired test
- Zero missing samples during a one-hour engineering stress test
- All integrity failures explicitly counted and reported

### NFR-004 Accessibility

The interface shall support keyboard operation, scalable text, adequate color contrast, non-color-only status indicators, and a minimum 1366 × 768 layout.

### NFR-005 Maintainability

The project shall use:

- Clear module boundaries
- Versioned schemas/protocols
- Lockfiles
- Formatting and linting
- Automated tests
- No hidden global state for acquisition
- Documented commands
- No silent exception swallowing

## Phase 4 course-capture amendment

Phase 4 supersedes the earlier one-channel course-capture assumptions without changing the
medical-device boundary. The controlled application shall support the UNO R4 WiFi with protocol
v0.2 for one to six sequentially sampled analog inputs in a synchronized logical frame and one
fixed pulse-ox four-state cycle mode. Course profiles shall capture the following raw variables:

- ECG: A0 at 12 bit / 1000 frames/s.
- EMG + force: A0, A1, A2, A3 at 12 bit / 1000 frames/s.
- BP + PPG: A0, A1, A2 at 12 bit / 200 frames/s with D4 green active only while acquiring.
- Pulse ox raw TX/RX: A0/A1 at 14 bit; D5 RED, DARK, D6 IR, DARK; approximately 1 ms/state and
  250 cycles/s, preserving all eight state values per cycle.

The app shall preserve raw counts, timing, channel map, controlled firmware identity, profile
snapshot, markers, and integrity counters in BMEG, CSV, and metadata. It shall not calculate
physiological values, apply hidden filtering, or make clinical claims. Optional Phase 3B physical
characterization remains an instructor tool and is not a normal-course-recording prerequisite.

## Phase 5 calibration-and-units amendment

The application shall preserve raw ADC counts and add optional derived engineering display/export
values without changing BMEG records. It shall compute Arduino-input volts using the frozen ADC
resolution and configurable recorded Vref. For course MPXV channels it may compute kPa and mmHg
with the documented local transfer equation and stored Vs. For BP A2 it may fit and save a local
linear `MPXV_mmHg = slope × XGZP_volts + offset` calibration from a selected synchronized interval
or two or more manual points, reporting slope, offset, R², and paired sample count without an
automatic accept/reject threshold. It shall not calculate SBP, DBP, SpO2, heart rate, EMG force,
fatigue, or any physiological conclusion.

## 7. Deferred requirements

- macOS/Linux
- Wi-Fi acquisition
- Multiple simultaneous boards
- Clinical SpO2
- Automatic blood-pressure estimates
- Respiratory flow
- `.mat` export
- Cloud accounts or online data storage
