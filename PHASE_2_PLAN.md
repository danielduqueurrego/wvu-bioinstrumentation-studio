# Phase 2 implementation plan — firmware workspace and reliable upload

## Scope

Implement a single-file UNO R4 WiFi firmware workspace without changing the Phase 1 acquisition
protocol or adding biomedical interpretation. Preserve the current controlled reference firmware
and its D4/D5/D6-low safety behavior.

## Steps

1. Add a Rust `firmware_workspace` module for safe one-INO project creation, open/save/save-as,
   template copying, metadata round trips, source hashes, atomic writes, recent-project records,
   and structured project errors.
2. Extend the Arduino CLI adapter with captured result logs on failure, core/environment status,
   compiler diagnostic parsing, and argument-array compile/upload operations. Keep all temporary
   build outputs outside student project folders.
3. Add a serialized firmware-job controller that shares the existing session ownership boundary:
   reject active recording, disconnect an idle session before upload, discover only the supported
   board, compile, invoke CLI upload, poll for the returning application port, and verify declared
   WVU-protocol projects with the production handshake/identity path. Non-WVU projects finish as
   uploaded-but-incompatible rather than failed.
4. Add read-only controlled templates: Blank, A0 example, protocol reference, safe digital output,
   and ASCII serial diagnostic. Only project copies are editable; reference copies originate from
   the single controlled repository sketch.
5. Expose project, environment, compile, upload, restore-reference, status, diagnostics, and
   compatibility commands through Tauri. Persist job logs under an application log directory.
6. Replace the Firmware placeholder with a responsive CodeMirror 6 editor plus project controls,
   environment/board panel, build console, explicit upload confirmation, and visible compatibility
   state. Keep Acquisition disabled in the UI when the last upload is non-WVU or unverified.
7. Add deterministic Rust and frontend tests, then validate with CLI compile/upload on the UNO
   alone: upload non-WVU ASCII template, demonstrate incompatibility, restore the controlled
   reference, verify identity, and run a 30-second raw-A0 acquisition/export validation.
8. Update controlled docs and acceptance logs. Commit only if the full in-app workflow and final
   acquisition pass; otherwise leave the worktree commit-ready with the exact blocker.

## Risks

- Windows COM re-enumeration is variable; the uploader will use bounded discovery and record
  original, bootloader, and final ports instead of assuming COM12.
- The existing standalone reset path is not reliable enough to be the uploader mechanism. Arduino
  CLI's proven UNO upload path remains authoritative, followed by independent protocol proof.
- Tauri has no file-dialog plugin in the current shell, so Phase 2 will use explicit project-folder
  fields and an Explorer action rather than silently choosing user paths.
