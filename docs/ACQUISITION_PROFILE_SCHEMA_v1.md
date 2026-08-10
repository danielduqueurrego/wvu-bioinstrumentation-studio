# Acquisition profile schema v1

## Safety boundary

Profiles are teaching/engineering configuration packages, not medical-device configurations.
Locked course profiles require their controlled UNO firmware identity and preserve raw ADC
counts/Arduino-input volts. They do not authorize diagnosis or clinical use. Formal hardware
characterization is outside the runtime class application scope and never gates course capture.

## Package shape

An acquisition profile is a UTF-8 JSON document with `schema_version: 1`. The controlled
fields are `profile_id`, semantic `profile_version`, display/category/source/status,
UNO R4 WiFi/FQBN target, firmware requirement, acquisition, display, safety, export, and
integrity blocks. Unknown optional fields are retained in deterministic key order when practical.

`profile_id` is stable and uses lowercase ASCII letters, digits, `.`, `_`, and `-`.
`profile_version` is `MAJOR.MINOR.PATCH`. A finalized locked profile/version is immutable. An
Instructor Lab Editor draft is in-memory; an explicit Save atomically creates the next patch
revision. Users do not need to choose semantic-version components manually.

Phase 6 additions remain optional for older snapshots: `acquisition.digital_outputs` describes
the allowed controlled output pins and safe behavior; `plot_defaults.groups` maps signal IDs to
their default display-only plot groups; channel entries may declare allowed conversion types,
default unit, and default visibility; and `associated_sketch` records a named controlled or local
Arduino sketch reference. These fields are frozen into a recording snapshot but never replace raw
ADC data or embed arbitrary source code.

## Integrity

The canonical bytes are deterministic JSON serialization of the typed profile with
`integrity.canonical_hash` set to the empty string. SHA-256 of those bytes is stored as the
lowercase 64-character `canonical_hash`. This detects accidental/tampered profile content. It is
**not** cryptographic authorship authentication, authorization, or a substitute for review.

## Built-in locked packages

| ID | Name | Mapping / ADC / logical rate | Signal label | SHA-256 |
| --- | --- | --- | --- | --- |
| `wvu.bmeg420l.general.analog.development.v2` | General Analog — Development | A0 default; instructor lab supports 1–6 unique A0–A5 / 14 bit default (12 or 14 supported) / supported rate | profile-defined | `4b04390a944d34f28b1b334c37e794ee4ef017532a1f25e558954b884a8bc1af` |
| `wvu.bmeg420l.ecg.course.capture.v1` | ECG — Course Capture | A0 / 14 bit / 1000 frames/s | `ecg_counts` | `49e33fbbaa280686f2f1876e81349f3086b5e7d5205e1b9251d74b2d47185510` |
| `wvu.bmeg420l.emg.force.course.capture.v1` | EMG + Force — Course Capture | A0–A3 / 14 bit / 1000 frames/s | four synchronized raw fields | `87d5940dfcf206cedd5b80f1608761746ecc9ac58d1f14b936328816ce688322` |
| `wvu.bmeg420l.blood_pressure.ppg.course.capture.v1` | Blood Pressure + PPG — Course Capture | A0–A2, D4 green / 14 bit / 200 frames/s | three synchronized raw fields | `18e9d0fc8e90612d0747e0bf455b5fcfc034d29c084078987342565cdc820d45` |
| `wvu.bmeg420l.pulseox.txrx.raw.course.capture.v1` | Pulse Oximetry — TX + RX Raw Capture | A0/A1, D5 red, D6 IR / 14 bit / 250 cycles/s | eight state-preserved raw fields | `6386f36f85f9522335d63a8d63ed12663ab5802968bb4c13b6d5d26d64cf1cb1` |

All require UNO R4 WiFi FQBN `arduino:renesas_uno:unor4wifi`, protocol 0.3, build
`0x00010003`, and device `0x554E4F34`. All allow timed (10/30/60/300/600 s plus custom ≥10 s)
and Until-stopped runs. Values are raw ADC counts; direct Arduino-input volts use the explicitly
recorded reference-voltage assumption. See `COURSE_ACQUISITION_PROFILES.md` for field order and
the teaching-use boundary.

## Recording provenance and compatibility

At start, the controller freezes `ProfileSnapshot { captured_utc, bench_notice_acknowledged,
profile }`. It is embedded in the BMEG JSON header and the `.metadata.json` sidecar. v0.2 BMEG
records are record-major synchronized arrays whose field count comes from the frozen profile;
v0.1 single-count records remain readable. Readers treat absent profile provenance as a
legacy/general-development recording and never infer a course modality.

v0.2 CSV begins with `record_sequence,t_us` (or `cycle_index,t_us` for pulse ox) followed by the
frozen profile's raw fields. Phase 5 may append direct-voltage and documented engineering-unit
columns beside the raw fields. The `RecordingMetadata.calibration` snapshot—not the immutable
profile—records Vref, Vs, selected display units, and active calibration parameters. Legacy CSV
remains readable under its documented 5.0 V conversion assumption.

## Local authoring workflow

Student mode is the default and exposes only active locked lab revisions. Five factory course labs
are resolved from bundled immutable definitions on every startup; they require no import and are
never rewritten by startup or normal reads. Instructor authoring requires an explicit local
acknowledgement, logs the mode change locally, and is not strong authentication. Instructors can
edit a current lab in an in-memory draft, duplicate it, explicitly save one automatic new locked
revision, import/export, retire/restore local revisions, activate a revision, and restore the
factory default. The backend validates all pins, output behavior, sample rate, ADC resolution,
pulse-ox resource conflicts, base-version freshness, and the canonical hash before it activates a
revision. Reads, selection, acquisition, calibration, plotting, navigation, and compatibility
evaluation never create a version. Old recordings retain their original snapshot even if the
active lab changes later.

The operating-mode UI has one native radio-group value: `student` or `instructor_authoring`.
Acknowledgement is a separate checkbox. Selecting Instructor without acknowledgement leaves
Student selected; selecting Student does not require clearing acknowledgement; and clearing
acknowledgement during Instructor authoring immediately returns the UI and local workflow mode to
Student. The local mode-change log records only completed backend transitions.

## Historical evidence compatibility

Older recordings may contain optional Phase 3B validation metadata. Current readers preserve that
metadata for backward-compatible deserialization but the class application does not create,
display, or use validation evidence. Acquisition profile integrity remains independent of any
historical evidence.
