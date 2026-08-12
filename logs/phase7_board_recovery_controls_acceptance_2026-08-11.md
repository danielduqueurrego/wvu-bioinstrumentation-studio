# Phase 7 board-recovery controls acceptance — 2026-08-11

## Status

Focused manual acceptance pending. No firmware was changed during the software investigation.

## Reported blocker

The single-window release candidate detected an Arduino UNO R4 WiFi and reported **Firmware update
required** with Arduino tools ready, but the Board dropdown, Refresh Board, Verify Firmware, and
Restore WVU Firmware controls were unavailable.

## Root cause

A read-only startup firmware handshake that fails moves the shared serial session into `Faulted` so
connection diagnostics remain available. The single-window Board controls incorrectly required the
exact `Disconnected` state. `Faulted` owns no serial handle, but was therefore treated as an active
recording/operation and blocked the very recovery controls needed to restore the board.

## Corrected availability policy

The frontend now derives independent board-control predicates from selected-board presence,
recording/connection activity, active board operation, bundled Arduino-tool readiness, and firmware
status. Firmware status never gates recovery:

| Control | Enabled when |
| --- | --- |
| Board dropdown | no recording/connection and no board operation |
| Refresh Board | no recording/connection and no board operation |
| Verify Firmware | a supported board is selected, with no recording/connection or board operation |
| Restore WVU Firmware | a supported board is selected, bundled tools are ready, and no recording/connection or board operation is active |

Hardware acquisition remains stricter: it requires a selected board with verified WVU firmware as
well as normal session requirements.

## Automated evidence

- Rust: 87 tests passed, including a faulted-handshake recovery ownership regression.
- Frontend: 54 tests passed, including the Board recovery state matrix for compatible, update-
  required, incompatible/non-WVU/silent, no-board, missing-tools, recording-active, and
  operation-active states.
- `cargo fmt --check`, `cargo check`, `cargo clippy -- -D warnings`, `npm run check`, and
  `npm run build` passed.
- The release executable was rebuilt at
  `src-tauri/target/release/wvu_bioinstrumentation_studio.exe`.

## Required manual result

With a supported UNO R4 WiFi selected and firmware showing **Update required**:

1. Confirm the Board dropdown, Refresh Board, Verify Firmware, and Restore WVU Firmware controls
   are enabled.
2. Restore the WVU firmware; confirm an operation modal appears with no external console window.
3. Confirm the board re-enumerates, status becomes **Firmware ready**, Acquisition becomes
   available, and Refresh/Verify continue to work.
