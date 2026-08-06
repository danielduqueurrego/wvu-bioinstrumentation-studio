# Phase 1.1 responsive UI verification — 2026-08-06

## Source/build and launch evidence

- Tauri configuration: 1280 × 840 default; 900 × 650 logical-pixel minimum.
- The primary shell uses responsive Grid/Flex layout with `minmax`, `auto-fit`, `clamp`, wrapped
  controls/status cards, and content vertical overflow rather than page horizontal overflow.
- At widths below 900 px, navigation becomes a compact row; below 650 px it becomes two columns
  and recording actions stack. Paths/errors use wrapping plus full-value tooltips.
- `LivePlot.svelte` owns a cleanup-safe `ResizeObserver`; it coalesces changes with
  `requestAnimationFrame` and calls `uPlot.setSize` without recreating the plot.
- `npm run check` completed with zero errors/warnings and the current MSI/NSIS Tauri release build
  succeeded. The packaged executable launched and closed cleanly at its configured minimum size.

## Manual Windows matrix

The agent environment cannot access the interactive desktop surface used by the released Tauri
window, so it cannot make credible visual assertions about clipping, scaling, or in-session plot
resize. Each requested manual result therefore remains **not performed**, rather than inferred
from source/CSS:

| Viewport / scaling | Home | Firmware | Acquisition | Diagnostics | Result |
|---|---|---|---|---|---|
| 900 × 650 at 100% | Not performed | Not performed | Not performed | Not performed | Pending manual review |
| 1024 × 768 at 100% | Not performed | Not performed | Not performed | Not performed | Pending manual review |
| 1366 × 768 at 100% | Not performed | Not performed | Not performed | Not performed | Pending manual review |
| 1920 × 1080 / maximized at 100% | Not performed | Not performed | Not performed | Not performed | Pending manual review |
| 125% or 150% Windows scaling | Not performed | Not performed | Not performed | Not performed | Pending manual review |

The remaining manual checks are: no page-level horizontal overflow, no clipped controls/long
paths/errors, Stop button reachability, approved-logo aspect ratio, and live plot resize while an
acquisition is active. This is a documentation/visual verification gap, not a known normal-run
data-integrity defect.
