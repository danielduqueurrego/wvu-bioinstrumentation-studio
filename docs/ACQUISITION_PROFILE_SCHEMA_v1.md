# Acquisition profile schema v1

## Safety boundary

Profiles are teaching/engineering configuration packages, not medical-device configurations.
Phase 3A ECG and EMG profiles permit only simulator signals, the UNO R4 WiFi alone, or a safe
0–5 V bench signal applied directly to A0. No human-connected recording is authorized.

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

| ID | Name | Pin / ADC / rate | Signal label | SHA-256 |
| --- | --- | --- | --- | --- |
| `wvu.bmeg420l.general.a0.development.v1` | General A0 — Development | A0 / 12 bit / 1000 Hz | `general_a0_raw_input` | `028bc317057b90f1f070cd215ce8d9d6200214819c32494a492a3af51661b77e` |
| `wvu.bmeg420l.ecg.raw.v1` | ECG Module — Raw Output | A0 / 12 bit / 1000 Hz | `ecg_module_raw_output` | `1cfd886ac74c0d9ee3a59213dec19176f86677cbcf508326f176adfc122d8c59` |
| `wvu.bmeg420l.emg.raw.v1` | EMG Module — Raw Output | A0 / 12 bit / 1000 Hz | `emg_module_raw_output` | `863f918ef494afb74fb15c8b878ab1009128431a00698eb706088e91c8502ea3` |

All require UNO R4 WiFi FQBN `arduino:renesas_uno:unor4wifi`, protocol 0.1, build
`0x00010001`, device `0x554E4F34`, raw ADC counts and direct Arduino input volts using
`counts * 5.0 / 4095.0`. All allow timed (10/30/60/300/600 s plus custom ≥10 s) and
Until-stopped runs. ECG/EMG require a session-local acknowledgement of the bench-only notice.

## Recording provenance and compatibility

At start, the controller freezes `ProfileSnapshot { captured_utc, bench_notice_acknowledged,
profile }`. It is embedded in the existing BMEG JSON header and the `.metadata.json` sidecar.
Sample records and BMEG magic `BMEGREC1` do not change. Readers treat absent profile provenance
as a legacy/general-development recording; they never infer ECG or EMG.

New profile-aware CSVs add `profile_id`, `profile_version`, and `signal_label` columns after the
existing spreadsheet-compatible raw-data columns. Legacy CSV remains unchanged.

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
