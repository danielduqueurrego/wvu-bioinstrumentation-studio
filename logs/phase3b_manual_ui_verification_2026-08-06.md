# Phase 3B manual Validation UI verification — 2026-08-06

## Status: pending

No manual Phase 3B Validation-page viewport/scaling inspection has been performed during this
implementation run. Do not infer results from Phase 3A or earlier Firmware/Acquisition layout
tests.

| Viewport | Windows scaling | Result | Notes |
| --- | --- | --- | --- |
| 900 × 650 | not recorded | pending | Validate stacked panels, tables, focus, and no page overflow. |
| 1024 × 768 | not recorded | pending | Validate workflow controls and long IDs/equipment values. |
| 1366 × 768 | not recorded | pending | Validate run/criteria tables and diagnostics access. |
| 1920 × 1080 | not recorded | pending | Validate wide panel allocation and status labels. |
| Maximized | not recorded | pending | Validate maximum-width behavior. |

The Validation page constrains horizontal scrolling, if needed, to the run/criteria table wrapper;
this implementation detail is not a manual acceptance result. Required manual checks remain:
Student permission blocking, Instructor acknowledgement, draft/resume/finalize/retire,
export/import, keyboard navigation, focus visibility, long text wrapping, and no page-level
horizontal overflow.
