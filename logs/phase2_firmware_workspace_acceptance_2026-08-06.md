# Phase 2 firmware workspace acceptance — 2026-08-06

## Scope

UNO R4 WiFi only; no person, biomedical accessory, optical hardware, pressure hardware, or
external test board was connected. The controlled reference keeps D4, D5, and D6 LOW.

## Implemented and automated evidence

- Single-file project model: `<ProjectName>/<ProjectName>.ino` plus `project.json`.
- Controlled templates: blank UNO R4 WiFi, A0 ASCII example, byte-identical WVU reference,
  D4/D5/D6-LOW digital example, and ASCII serial diagnostic.
- CodeMirror 6 C++ editor with line numbers, bracket matching, find/replace, history,
  indentation, Ctrl+S/Ctrl+Shift+S, changed-state indication, and source-line diagnostics.
- Native directory selection, recent projects, Save/Save As, restore saved source, and Explorer
  reveal action.
- Managed Tauri state with one shared `SessionController` and one serialized firmware workflow.
- Arduino CLI environment panel, structured errors, command/output logs, cancellable child
  process, compile-output parsing, explicit upload confirmation, and reference restore.

`cargo test` passed 44 Rust tests, including project structure,
template byte identity, CLI JSON/compiler diagnostic parsing, upload preflight, identity checks,
same/different/ambiguous returning-port logic, and terminal-job status publication. `npm test`
passed 9 frontend control-policy tests. The full command matrix is recorded in the implementation
report and was rerun after these changes.

## Manual UI evidence

The static responsive layout uses a full-width editor and a CSS Grid side panel that stacks below
at medium/narrow widths; long paths and output wrap/scroll within their panels. A human visual
matrix at 900x650, 1024x768, 1366x768, 1920x1080, maximized, and 100%/125%/150% Windows scaling
has **not** been observed in this noninteractive session. It is pending and is not marked passed.
The MSI/NSIS-built release executable remained responsive for a five-second non-recording launch
check (32,829,440-byte working set). A screen-capture attempt could not access that process's
interactive desktop surface, so it is not visual evidence.

### User-supplied wide-window evidence and remediation

Six user-supplied release-application screenshots at 2048 x 828 pixels
(`Screenshot 2026-08-06 133140.png` through `133222.png`) provide the following **pre-fix**
evidence. Home, Firmware, Acquisition, and Diagnostics had reachable controls, no obvious
overlap, no clipped primary buttons, and no observed page-level horizontal scrollbar. Firmware
and Acquisition, however, were constrained to roughly an 850–900 pixel left-aligned workspace,
leaving substantial unused space to the right. This unnecessarily narrowed the CodeMirror editor,
the Firmware environment panel, the acquisition status area, and the live plot. The actual
Windows display-scaling setting was not supplied, so these screenshots do not pass a scaling row.

The source was corrected after that observation. Prose-oriented Home and Diagnostics now use an
intentional readable maximum width, while Firmware and Acquisition explicitly stretch to the full
available content grid. Firmware's editor/build layout now reserves a 304–400 pixel environment
rail and gives all remaining width to CodeMirror; its diagnostics console remains full workspace
width. The existing 1050-pixel stacking breakpoint and 900 x 650 navigation reflow are unchanged.
`npm run check`, `npm test` (9 tests), `npm run build`, and `cargo check` passed after the change.

### Post-fix manual inspection — user confirmed

The rebuilt release application was inspected after the correction. The following observed rows
passed; the Windows scaling value was represented as `[SCALING]` rather than a numeric setting, so
it is intentionally not credited to a 100%, 125%, or 150% scaling row.

| Window size | Windows scaling | Project/editor and environment | Console and Acquisition | Overflow/clipping | Result |
| --- | --- | --- | --- | --- | --- |
| 900 x 650 | not recorded (`[SCALING]`) | Editor remained visible; environment panel remained readable; no blank/collapsed editor | Console reachable; Acquisition remained usable | None reported | Passed |
| Wide/maximized | not recorded (`[SCALING]`) | Editor expanded into available width; environment panel remained readable | Console accessible; Acquisition expanded correctly | No page-level horizontal scroll, clipping, or overlap reported | Passed |

The source retains the narrow stacking/reflow behavior at the 1050-pixel breakpoint. The
specifically enumerated 1024 x 768, 1366 x 768, and 1920 x 1080 rows, plus numeric Windows
100%/125%/150% scaling rows, were not separately reported and remain pending documentation—not
passed by inference. No post-fix blocking layout defect was reported.

## Outcome

Phase 2's firmware workspace, compile/upload/restore, acquisition coordination, and the
user-confirmed post-fix UI acceptance are passed. The unrecorded intermediate viewport and numeric
Windows-scaling rows remain a nonblocking documentation follow-up; they are not represented as
tested in this log.
