# Phase 7 Start Recording recovery acceptance — 2026-08-12

## Status

Software correction, a direct UNO transport capture, and automated verification are complete.
Focused release-app recording acceptance remains pending instructor confirmation. No firmware
upload or restore was initiated during this Start-path investigation.

## Reported blocker

After the controlled WVU firmware was restored and verified, Start was first blocked by a stale
idle fault. After that condition was corrected, the UI reached the Tauri command but showed only
the generic message **Recording could not start**.

## Trace and root cause

The frontend initially used one broad `canStart` condition. A historical `Faulted` acquisition
snapshot made that condition false even after firmware compatibility had become Ready. The Start
button was then disabled; its handler only had a generic output-folder/duration fallback. In
addition, periodic status polling could overwrite a blocked-start message with stale terminal
diagnostics.

The backend correctly keeps a failed verification in `Faulted` while recovery controls are needed,
but `begin_session` accepts only `Disconnected`. Successful verification/restoration did not have
an explicit common transition that cleared an idle terminal fault before a new recording.

The later generic-error failure was separate and occurred before the command body, filesystem, or
serial path ran. The frontend correctly sent an IPC object named `request`, while the Rust command
used a destructured `StartProfileHardwareRequest` function parameter. Tauri 2 names a destructured
command parameter from its type, generating the required key `startProfileHardwareRequest` rather
than `request`. Its generated IPC argument decoder therefore rejected the call as a missing
`startProfileHardwareRequest` value. The frontend catch converted that exact rejection to the
generic student message.

| Guard / condition | Observed effect before correction | Corrected behavior |
| --- | --- | --- |
| `session.state === Disconnected` | A stale `Faulted` snapshot prevented Start | Successful verification/restoration normalizes an idle session to `Disconnected`; start defensively does the same after validation. |
| Hardware firmware compatibility | Required for acquisition, correctly | Still required only for hardware acquisition. |
| Project/Output folder and duration | Included in a single opaque predicate | Each now has a dedicated student-facing blocked-start explanation. |
| Busy/active session | Could be indistinguishable from other causes | Start reports that the Arduino is busy and prevents duplicate in-flight requests. |
| Tauri Start argument | Frontend sent `request`; a destructured Rust parameter required `startProfileHardwareRequest` | The command now takes a named `request: StartProfileHardwareRequest`, matching the frontend payload. |

## Corrected lifecycle

```text
failed verification -> Faulted diagnostics retained
successful Verify/Restore -> release idle worker/serial state -> Disconnected
Start -> Connecting -> Configuring -> Recording
failure -> cleanup -> a later Verify/Restore/Start may begin a fresh session
```

`prepare_for_new_recording` rejects genuinely active sessions, joins a completed worker, clears only
idle transient fault data, and never interrupts an acquisition. It is used after successful firmware
verification/restoration and defensively by recording-start commands.

The Start command now records bounded stage events in the per-user application log. Synchronous
preflight failures return a structured `stage`, `code`, technical detail, and student-safe message.
The frontend shows the concise message and retains the exact stage/detail, timestamp, selected
board, and lab under **Advanced details**. Transport stages log `SERIAL_OPEN`, `HANDSHAKE`,
`CONFIGURE`, `CONFIG_ACK`, `START`, and `RECORDING_ACTIVE`; asynchronous transport errors remain
available through `SessionStatus.last_error`.

## User-visible behavior

Clicking Start now either begins with **Connecting to Arduino…** followed by **Configuring
recording…**, or shows a direct explanation for no selected board, firmware not ready, invalid
Project/Output folder, invalid duration, required acknowledgement, busy operation, or stale fault.
Known backend failures now map to folder, board/port, firmware, calibration, or busy-session
messages. The generic fallback is used only for unknown failures and directs the student to
**Advanced details** rather than discarding the technical cause.

## Hardware transport evidence

The controlled UNO R4 WiFi on COM5 completed an ECU-only, floating-A0 ECG transport run through
the same `SessionController` acquisition engine used by the application:

| Item | Result |
| --- | --- |
| Profile | ECG — Course Capture, A0, 14-bit, 1000 Hz |
| Duration | 10 seconds timed |
| Records | 9,990 |
| Measured rate | 1000.024 Hz |
| Packets | 1,031 |
| CRC / missing / duplicate / out-of-order / overflow | 0 / 0 / 0 / 0 / 0 |
| Completion | complete / `timed_complete` |
| Digital outputs | 0 while active and final |

The log recorded: `SERIAL_OPEN_BEGIN`, `SERIAL_OPEN_OK`, `OPEN_RECORDING_FILE_OK`,
`HANDSHAKE_OK`, `CONFIGURE_SEND`, `CONFIG_ACK_RECEIVED`, `START_SEND`, and
`RECORDING_ACTIVE`. This proves the verified firmware accepts the current ECG configuration; it
does not replace the focused release-UI manual test.

## Automated evidence

- Rust session regression: a recovered idle fault returns to `Disconnected`; normalization rejects
  an active session.
- Frontend readiness regression: Ready hardware starts; no board, firmware, path, duration, busy,
  and fault states each provide an explicit reason; an in-flight start is blocked.
- `cargo fmt --manifest-path src-tauri\Cargo.toml -- --check`: passed.
- `cargo check --manifest-path src-tauri\Cargo.toml`: passed.
- `cargo test --manifest-path src-tauri\Cargo.toml`: **90 passed**, 0 failed.
- `cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets -- -D warnings`: passed.
- `npm run check`: passed with 0 errors and 0 warnings.
- `npm test`: **15 files / 68 tests passed**.
- `npm run build`: passed.
- Canonical `npm run tauri build`: invoked after the frontend production build. The tool wrapper
  reached its 184-second timeout while NSIS was still completing, but the child process completed
  afterward and produced the rebuilt release executable and temporary bundle output. Final
  NSIS/MSI/ZIP staging and checksums remain deliberately deferred pending manual release acceptance.

## Rebuilt production executable

```text
Path: src-tauri\target\release\wvu_bioinstrumentation_studio.exe
Last modified: 2026-08-12 10:22:12 -04:00
SHA-256: 6BB5C71D2A0F0A7C0A413392A4ED6F1C15819B9A996112C4DC986A3335ED656B
```

The executable was launched after confirming that no process was listening on the Vite development
port (1420). It is the Tauri-built production binary, not a direct Cargo artifact.

## Required focused manual acceptance

1. Launch the rebuilt Tauri executable, select **ECG — Course Capture**, Hardware, a writable
   Project folder, Output folder `Test1`, and 10 seconds.
2. Confirm **Firmware ready**, click Start, and verify immediate connection/configuration feedback,
   recording samples/plot updates, and files beneath `<Project>\Test1`.
3. Repeat with Output folder `Test2` without restarting.
4. Run a short EMG multi-channel recording.
5. Click Verify Firmware, then start another short recording. It must not retain a stale session
   fault or compatibility result.
6. If any Start fails, open **Advanced details** and record the Stage, Code, and Detail values.
