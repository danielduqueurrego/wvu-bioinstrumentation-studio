# Acquisition profile schema v1

## Safety boundary

Profiles are teaching/engineering configuration packages, not medical-device configurations.
Locked course profiles require their controlled UNO firmware identity and preserve raw ADC
counts/Arduino-input volts. They do not authorize diagnosis or clinical use. The optional
Validation workflow remains bench-only; normal course capture does not require Validation evidence.

## Package shape

An acquisition profile is a UTF-8 JSON document with `schema_version: 1`. The controlled
fields are `profile_id`, semantic `profile_version`, display/category/source/status,
UNO R4 WiFi/FQBN target, firmware requirement, acquisition, display, safety, export, and
integrity blocks. Unknown optional fields are retained in deterministic key order when practical.

`profile_id` is stable and uses lowercase ASCII letters, digits, `.`, `_`, and `-`.
`profile_version` is `MAJOR.MINOR.PATCH`. A finalized locked profile/version is immutable; edits
begin with a separate instructor draft and finalization creates a new version.

## Integrity

The canonical bytes are deterministic JSON serialization of the typed profile with
`integrity.canonical_hash` set to the empty string. SHA-256 of those bytes is stored as the
lowercase 64-character `canonical_hash`. This detects accidental/tampered profile content. It is
**not** cryptographic authorship authentication, authorization, or a substitute for review.

## Built-in locked packages

| ID | Name | Mapping / ADC / logical rate | Signal label | SHA-256 |
| --- | --- | --- | --- | --- |
| `wvu.bmeg420l.general.analog.development.v2` | General Analog — Development | A0 default; instructor draft supports 1–6 unique A0–A5 / 12 or 14 bit / ≤1000 Hz | profile-defined | `d5ee6a65cbeb8c4b950586ad591cc7ac566718f38afeda74f5fb1a45ee71ef2d` |
| `wvu.bmeg420l.ecg.course.capture.v1` | ECG — Course Capture | A0 / 12 bit / 1000 frames/s | `ecg_counts` | `cd04267743fd38066aa8daf91b6eafa9550e4406e2ef3d0d59ceae04051b782d` |
| `wvu.bmeg420l.emg.force.course.capture.v1` | EMG + Force — Course Capture | A0–A3 / 12 bit / 1000 frames/s | four synchronized raw fields | `ddad92c2805f04b5af546bcd5bf9dc05b74b6e92163e99a279c3e7fbf13ddcd9` |
| `wvu.bmeg420l.blood_pressure.ppg.course.capture.v1` | Blood Pressure + PPG — Course Capture | A0–A2, D4 green / 12 bit / 200 frames/s | three synchronized raw fields | `14cf52a6b4c097474efd1303e872dcad5d54532518b6c0b8bb36d4a68166be7a` |
| `wvu.bmeg420l.pulseox.txrx.raw.course.capture.v1` | Pulse Oximetry — TX + RX Raw Capture | A0/A1, D5 red, D6 IR / 14 bit / 250 cycles/s | eight state-preserved raw fields | `2ada45d1e96dbd2747c0a6e897fff234e9441e599dc7077df529196cc62d021e` |

All require UNO R4 WiFi FQBN `arduino:renesas_uno:unor4wifi`, protocol 0.2, build
`0x00010002`, and device `0x554E4F34`. All allow timed (10/30/60/300/600 s plus custom ≥10 s)
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
frozen profile's raw field names. Legacy CSV remains unchanged.

## Local authoring workflow

Student mode is the default and exposes valid active locked profiles only. Instructor authoring
requires an explicit local acknowledgement, logs the mode change locally, and is not strong
authentication. It can duplicate a locked profile to a draft, edit allowed draft content,
validate/finalize a new locked version, import/export a validated locked package, and retire an
instructor-created version without deleting recordings that contain its snapshot.

The operating-mode UI has one native radio-group value: `student` or `instructor_authoring`.
Acknowledgement is a separate checkbox. Selecting Instructor without acknowledgement leaves
Student selected; selecting Student does not require clearing acknowledgement; and clearing
acknowledgement during Instructor authoring immediately returns the UI and local workflow mode to
Student. The local mode-change log records only completed backend transitions.

## Bench-validation association

Phase 3B evidence is deliberately not embedded into or used to change a profile package. The
Validation view reports a profile as unvalidated, draft, bench validated, expired, or mismatched
only after comparing a separate finalized validation document's profile ID/version/hash and
firmware build/device. The recording snapshot remains immutable. See
`docs/VALIDATION_EVIDENCE_SCHEMA_v1.md`; validation does not authorize human-connected use.
