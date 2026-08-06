# Validation evidence schema v1

## Scope and safety boundary

Validation evidence documents an engineering bench check of the analog interface between a
locked ECG or EMG profile, its module output or safe source, the UNO R4 WiFi input, and the
recording pipeline. It is not human-use authorization, electrical-isolation evidence, a medical
device record, or physiological validation. No person or electrode system may be connected while
using this workflow.

## Evidence document

`validation.json` is UTF-8 JSON with `schema_version: 1` and these top-level fields:

- stable `validation_id`;
- exact locked `profile_id`, `profile_version`, and `profile_hash`;
- `status`: `draft`, `finalized`, or locally retired from active review;
- `validation_type`: `analog_interface`;
- UTC creation time and `created_by_mode: instructor_authoring`;
- UNO R4 WiFi, serial/COM-port, firmware build/device, and module identity metadata;
- equipment and test-condition metadata;
- separate raw-data `tests` / runs, transparent metrics, and instructor-defined criterion results;
- explicit acceptance summary, optional expiry, notes, and SHA-256 integrity block.

Finalization is permitted only after baseline, DC sweep, sine, saturation-margin, and at least
three separate repeatability runs are present, every supplied criterion passes, and the instructor
explicitly accepts the summary. Editing finalized evidence is rejected; create a new draft or
revision instead. Retiring an evidence record writes its ID to a local `retired.json` index, which
removes it from active selection without changing the finalized evidence bytes or existing
recording provenance.

## Canonical integrity

Canonical bytes are deterministic serialization of the typed evidence document with
`integrity.canonical_hash` empty. The stored lowercase SHA-256 digest detects accidental or
tampered changes. It is **not** a cryptographic signature, identity assertion, authorization
mechanism, or substitute for instructor review.

## Profile and firmware matching

A finalized record can make a profile show **Bench validated** only when its profile ID, version,
and hash exactly match the active locked profile and its firmware build/device exactly match that
profile's controlled firmware requirement **and the evidence represents a physical bench run**.
Finalized simulator evidence stays visible for software-path review but leaves the profile
**Unvalidated**; it cannot substitute for physical ECG/EMG interface evidence. Otherwise the
application reports draft, unvalidated, expired, profile-mismatch, or firmware-mismatch status. A
validation result never changes the bench-only/no-human safety restriction.

## Raw-data linkage

Each run stores paths to separate raw `.bmeg`, metadata JSON, and CSV files plus sample count,
metrics algorithm (`phase3b.raw_metrics.v1`), source conditions, metrics, and criteria. A frozen
`validation_context` is embedded in the BMEG JSON header and metadata sidecar before acquisition
begins. The fixed `BMEGREC1` sample-record layout is unchanged; files without validation context
remain legacy/general recordings.

Validation-aware CSV files retain the normal raw-data columns and profile columns, then add
`validation_id`, `validation_test_type`, and `validation_run_number`. CSV voltage is still exactly
`adc_counts * 5.0 / 4095.0`.

## Package format

A finalized package is a new directory containing:

```text
validation.json
summary.csv
manifest.json
```

`manifest.json` lists SHA-256 digests for the other two files and identifies the profile,
firmware, application version, and package creation time. Large raw recordings are referenced,
not duplicated. Import verifies schema, manifest hashes, final evidence hash, and selected locked
profile/firmware match before storing evidence.
