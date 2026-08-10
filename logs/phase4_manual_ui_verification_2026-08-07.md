# Phase 4 Acquisition UI manual verification — 2026-08-07

Status: pending interactive inspection.

| Viewport | Windows scaling | Status | Notes |
| --- | --- | --- | --- |
| 900 × 650 | not recorded | pending | verify stacked profile setup, trace toggles, marker, Start/Stop and no page overflow |
| 1024 × 768 | not recorded | pending | verify four-trace EMG layout |
| 1366 × 768 | not recorded | pending | verify BP and pulse-ox controls/legend |
| 1920 × 1080 | not recorded | pending | verify wide plot use and long labels |
| maximized | not recorded | pending | verify responsive plot and no left-width constraint |
| 125% / 150% | not tested | pending | do not infer scaling from prior phases |

Automated frontend checks cover profile-derived channel labels, trace show/hide behavior, pulse-ox
preview data, marker UI, and responsive layout classes. A manual desktop rendering check remains
required before Phase 4 can be marked accepted.
