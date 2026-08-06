# Phase 3A plan — locked ECG and EMG acquisition profiles

## Scope and safety boundary

Phase 3A adds profile selection, provenance, and bench-only acquisition constraints. It does not
authorize human-connected recordings, add physiological interpretation, alter pulse-ox behavior,
or change the controlled UNO R4 WiFi protocol/firmware identity. Hardware acceptance will use
only the UNO R4 WiFi alone with floating A0 or a safe 0–5 V bench signal.

## Implementation plan

1. Replace the obsolete minimal profile loader with a versioned acquisition-profile module.
   - Embed three immutable, version-controlled built-ins: General A0 — Development, ECG Module —
     Raw Output, and EMG Module — Raw Output.
   - Use deterministic typed JSON serialization, sorted unknown optional fields, and SHA-256 over
     the canonical document excluding the hash field.
   - Validate schema, semantic version, IDs, UNO/FQBN, firmware identity, pin/rate/ADC settings,
     safety fields, duration settings, plot bounds, uniqueness, and locked integrity.

2. Add a local profile store and explicit workflow mode.
   - Student is the default and can select only valid locked profiles.
   - Instructor authoring requires an explicit local acknowledgement, logs mode changes, supports
     draft duplication/edit/validation/finalization, preserves retired packages, and never treats
     the acknowledgement as authentication.
   - Imported/exported JSON is validated and confined to controlled profile storage; no profile
     input is executable.

3. Bind approved profiles to the Rust session controller.
   - Replace fixed acquisition constants with a validated `ProfileSnapshot` captured once before
     session start.
   - Build the CONFIGURE payload from that snapshot, enforce matching firmware identity and
     capabilities for hardware, and keep legacy entry points using the General A0 snapshot.
   - Keep the same bounded parser, reader, display buffer, writer, storage guard, stop path, and
     simulator transport.

4. Extend recording/export provenance without breaking legacy recordings.
   - Add an optional profile snapshot to `RecordingMetadata`, which is already embedded in BMEG's
     versioned JSON header and copied to the sidecar.
   - Preserve reading files without the field as legacy/general-development recordings.
   - Stream CSV as before and add profile ID/version/signal-label comment metadata before the
     unchanged spreadsheet-compatible column header.

5. Add profile-aware Tauri commands and Acquisition UI.
   - Expose profile listing/details, mode changes, draft workflow, profile import/export, and
     profile-aware hardware/simulator starts.
   - Default to Student mode; display lock/source/hash/requirements/details; explain protected
     values; require one session-local bench-only acknowledgement before ECG/EMG start.
   - Keep the wide data-layout behavior, bounded 25 Hz polling, and no per-sample frontend events.

6. Add test and acceptance coverage.
   - Cover canonical hash/validation/immutability/drafts/finalization/retirement, start binding and
     snapshot freeze, legacy/new exports, wrong firmware/acknowledgement failures, and bounded
     profile-aware simulator sessions.
   - Run General-A0 simulator acceptance, then bench-only 30-second ECG and EMG hardware captures
     through the production controller after firmware compatibility verification.
   - Record only observed manual viewport/scaling results; request the user's physical UI review
     and never infer Windows scaling.

7. Update controlled documentation, profile schema documentation, and dated evidence logs. Commit
   only after automated, simulator, and bench acceptance checks pass; physical/human tests remain
   out of scope.

## Main risks and mitigations

| Risk | Mitigation |
| --- | --- |
| A profile changes a protected acquisition setting | Validate before use; locked profiles and snapshots are immutable; reject mismatch rather than silently adapting. |
| BMEG compatibility regression | Keep the fixed raw-record layout and add only an optional JSON-header metadata field with serde defaults. |
| Hash implementation is mistaken for authentication | Make the UI/docs say SHA-256 is integrity only; instructor mode remains a local workflow guard. |
| Frontend changes widen scope or hurt responsiveness | Use small typed Tauri commands, 25 Hz polling, existing responsive grids, and no sample events. |
| Bench profile labels are mistaken for clinical use | Repeat the bench-only/non-medical/no-human warning in profile data, UI, metadata, and acceptance logs. |
