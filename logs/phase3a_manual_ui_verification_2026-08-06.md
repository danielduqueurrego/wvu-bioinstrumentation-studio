# Phase 3A manual UI verification — 2026-08-06

## Pre-fix observation

Manual inspection confirmed that General A0, ECG, and EMG were selectable; ECG/EMG settings were
locked at A0, 12 bit, and 1000 samples/s; bench-only/no-human notices and acknowledgement gates
were visible; and the instructor draft/finalize/retire flow was reachable. Wide/maximized layout
had no observed page-level horizontal overflow, clipping, or overlap. Actual Windows scaling was
not reported and is therefore not inferred here.

The operating-mode gate failed twice. Initially, starting in Student mode, clicking **Instructor
authoring** before checking the acknowledgement could show both radio controls selected. After an
intermediate mitigation, another manual run found the reverse stale state: selecting Student while
acknowledgement remained checked correctly changed the badge, permissions, and panel visibility to
Student, but left the Instructor radio visibly checked.

Root cause: manually derived `checked` properties combined with click/default-event suppression
and asynchronous backend commands allowed the browser's radio property to drift from the UI mode.

## Corrected implementation

`OperatingModeControl.svelte` uses one `operatingMode: 'student' | 'instructor_authoring'` bound
through native `bind:group` radios sharing `name="profile-operating-mode"`. The acknowledgement is
a separate boolean. Selecting Instructor without acknowledgement immediately restores the bound
Student value; clearing acknowledgement while in Instructor authoring immediately changes the
bound mode to Student. Badge, authoring-panel visibility, and backend mode transition all derive
from that one value. Backend commands continue to reject Student-mode authoring requests and only
log completed transitions.

Automated DOM tests use happy-dom and assert the `checked` properties for startup, blocked entry,
Instructor → Student while acknowledgement remains checked, re-entry, acknowledgement clearing,
and a keyboard-originated native change.

## Post-fix manual result: passed

At 3440 × 1392 logical window pixels and Windows display scaling 100%, the operator verified:

1. Startup showed Student only.
2. Selecting Instructor before acknowledgement left Student selected.
3. Acknowledgement followed by Instructor selection showed Instructor only.
4. Selecting Student while acknowledgement remained checked showed Student only and hid authoring
   controls.
5. Instructor could be selected again without stale radio state.
6. Clearing acknowledgement in Instructor mode returned immediately to Student only.

ECG/EMG A0/12-bit/1000 samples/s locks, their bench-only Start acknowledgement, and the
instructor draft/finalize/SHA-256/retire controls remained functional. No clipping, overlap,
page-level horizontal scrolling, or inaccessible controls were observed in this focused wide
Acquisition inspection.

| Window | Scaling | Result | Notes |
| --- | --- | --- | --- |
| 900 × 650 | not tested | pending | Must not be inferred from the wide inspection. |
| 1024 × 768 | not tested | pending | Must not be inferred from the wide inspection. |
| 1366 × 768 | not tested | pending | Must not be inferred from the wide inspection. |
| 1920 × 1080 | not tested | pending | Must not be inferred from the wide inspection. |
| 3440 × 1392 | 100% | passed | Focused Acquisition profile workflow; no overflow or clipping observed. |
| maximized | not separately recorded | pending | The 3440 × 1392 state is not claimed as a separate maximized row. |
| any size | 125% | pending | Not observed. |
| any size | 150% | pending | Not observed. |
