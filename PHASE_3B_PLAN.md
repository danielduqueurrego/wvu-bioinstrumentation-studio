# Phase 3B implementation and acceptance plan

## Scope and safety boundary

Phase 3B adds a bench-only validation-evidence workflow for the locked ECG and EMG raw-output
profiles. It permits only the simulator, the UNO R4 WiFi, safe 0–5 V bench signals, and module
bench outputs. It will not add human-connected recording, electrode instructions, physiological
interpretation, clinical claims, automatic filtering, or electrical-isolation claims.

## Implementation plan

1. Add a versioned `validation` Rust module for draft/finalized/retired validation evidence,
   deterministic canonical JSON, SHA-256 integrity, profile/firmware matching, local
   instructor-only storage, and directory-package import/export with manifest hashes.
2. Implement transparent, raw-data-preserving metrics for baseline, DC, sine, saturation margin,
   repeatability, and instructor-defined criteria. Metric algorithms and versions will be stored
   with evidence; no raw sample is filtered or deleted.
3. Extend `RecordingMetadata` with optional validation-run context. Keep `BMEGREC1` fixed sample
   records unchanged so legacy recordings remain readable. Validation runs will use the existing
   `SessionController`, BMEG writer, parser, integrity monitor, and streaming CSV exporter.
4. Add deterministic validation simulator signal settings (DC, sine, clipping) to the existing
   transport path, with source parameters and seed captured in metadata.
5. Expose validation commands from the Tauri application state. All finalization, association,
   import, export, and retirement commands will independently require instructor mode and will
   reject invalid profile or firmware matches.
6. Add a responsive Validation view and navigation entry. The page will show safety acknowledgement,
   profile validation status, a draft workflow, test/run/criteria tables, evidence hashes, and
   import/export actions without widening the global page layout.
7. Add Rust and frontend regression coverage for evidence integrity, metric calculations,
   criteria, package tamper checks, permissions, metadata round trips, legacy reads, and responsive
   workflow controls.
8. Run simulator acceptance and export/package validation. Perform actual ECG and EMG bench
   acceptance only after a safe 0–5 V source/module setup is available; do not substitute invented
   hardware measurements.
9. Update schemas, workflow documentation, requirements/roadmap, known issues, and evidence logs.
   Commit only after the stated Phase 3B acceptance criteria are genuinely met.

## Main risks and controls

- Evidence hashes detect integrity changes but are not authentication; this wording remains
  explicit in UI and documentation.
- Evidence must not be mistaken for profile integrity or authorization for human use; all three
  concepts are shown separately.
- The current BMEG JSON header has a 65,535-byte limit. Validation context will contain stable
  identifiers and run metadata, while the full evidence remains a separate versioned JSON record.
- Safe module/source hardware and manually measured setpoints are external prerequisites for
  hardware acceptance. Until supplied, only simulator and software-path results can be marked
  passed.
