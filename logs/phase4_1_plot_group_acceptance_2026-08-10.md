# Phase 4.1 configurable plot-group acceptance

Status: automated implementation verification passed; manual acceptance remains pending.

## Display-only model

- Each profile signal is assigned to exactly one plot group.
- Signal visibility is independent of its group assignment and never changes the recorded data.
- `Overlay all` makes one group; `One plot per signal` creates one group per signal.
- Reducing plot count deterministically merges removed-group assignments into the last retained
  group. Increasing count creates assignable empty slots, which do not render blank plots.
- Every rendered group uses the same bounded display snapshot/time domain and its own uPlot
  y-axis autoscale.

## Defaults

| Profile | Default rendered groups |
| --- | --- |
| ECG | 1 — ECG |
| EMG + Force | 4 — one per signal |
| Blood Pressure + PPG | 3 — one per signal |
| Pulse Oximetry | 2 — RED/IR TX and RED/IR RX previews |
| General Analog | one per selected channel |

## Manual evidence still required

- Exercise regrouping and hide/show during live EMG, BP, and pulse-ox acquisition.
- Verify no blank/zero-height plot or page-level horizontal overflow at the required viewport
  matrix.

## Automated performance evidence

The Rust suite passed the existing accelerated simulator soaks after this refactor:

- six channels at 1000 logical frames/s for a ten-minute equivalent with bounded display history;
- pulse-ox raw cycles for a ten-minute equivalent with all eight raw fields retained;
- repeated simulator sessions, recording finalization, and export checks.

The plot-group utility tests cover defaults, reassignment, deterministic merging, empty-group
suppression, visibility preservation, and both quick presets. Group changes are frontend-only and
do not restart the Rust session or alter stored fields.
