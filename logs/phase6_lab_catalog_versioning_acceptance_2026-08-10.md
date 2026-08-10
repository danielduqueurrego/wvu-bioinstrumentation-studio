# Phase 6 lab catalog and versioning acceptance — 2026-08-10

Status: **automated verification and focused manual catalog verification passed.**

## Original blocker

After an explicit EMG `1.0.1` save, opening an editor draft inserted an unsaved `1.0.2` draft into
the in-memory catalog. The historical-list view displayed it as a historical version even though
the instructor had not saved it. The previous catalog could also allow persisted active/retired
state to hide bundled course profiles.

## Corrected model

- ECG, EMG + Force, Blood Pressure + PPG, Pulse Oximetry, and General Analog are immutable factory
  definitions bundled with the application and effective on every startup without import.
- Local state contains only explicit instructor/imported revisions, retirement state, explicit
  active overrides, and completed save-request IDs.
- Edit, Duplicate, and Blank Lab return detached drafts only. Field edits, dialog open/close,
  selection, navigation, acquisition, calibration, plotting, compatibility checks, and restart do
  not write the catalog.
- An explicit Save verifies the editor base version, allocates one next patch version, atomically
  persists it, and records a local audit line. Repeating the same request ID returns the original
  revision instead of creating another.
- Exact import collisions are no-ops; same ID/version with different content is rejected instead
  of being silently incremented. Factory defaults cannot be retired.
- **Reset local customizations** is an Instructor-only confirmed action that preserves factory
  labs and recordings while removing local overrides/custom labs.

## Automated evidence

- Repeated empty-catalog initialization (100 iterations) exposed exactly the five factory labs and
  performed no `lab_state.json` write.
- Detached draft/retry test: one explicit save yielded `1.0.1`; replaying the same request ID
  yielded `1.0.1` again and one audit line.
- Original reproduction regression: save EMG `1.0.1`, then perform 20 repeated list/select/edit
  preview operations and restart. The catalog remained exactly `1.0.0`, `1.0.1`; no `1.0.2`.
- Stale-base save and exact factory-import collision both failed without adding a revision.
- Local-customization reset restored all five factory labs and the factory ECG `1.0.0` active
  definition.

## Focused manual verification — passed

At the user’s confirmation, Reset local customizations restored the five factory labs without
import; one explicit EMG edit created only `1.0.1`; subsequent selection, simulator use, units,
plot groups, navigation, repeated Manager opens, and restart did not create `1.0.2`.

## Follow-up default adjustment

After the catalog verification, all five shipped profiles were changed to default to 14-bit ADC
resolution. Existing recording snapshots and instructor-created revisions remain unchanged; a
factory-default reset exposes the revised factory definitions.
