# Known issues

- Physical USB unplug/replug remains pending because it requires the user's manual participation.
  Automated terminal-disconnect finalization passes; the design requires an explicit new
  acquisition after a disconnect and never concatenates sessions.
- The requested manual responsive-layout matrix remains pending. Source/build evidence and a
  packaged-app launch pass, but this agent cannot inspect the interactive Tauri desktop surface at
  900 × 650, the larger viewports, or 125%/150% Windows scaling. See
  `logs/phase1_1_responsive_ui_verification_2026-08-06.md`.
- The explicit 1200-bps **Reset board and retry** recovery action can rediscover COM12 but then
  receive zero protocol bytes. The controlled firmware is currently restored and its normal
  handshake/Until-stopped acquisition pass; reset recovery remains unresolved and does not perform
  an automatic upload. See `logs/phase1_1_touch_reset_characterization_2026-08-06.md`.
- The desktop application does not yet bundle an in-app reference-firmware installer. Controlled
  Arduino CLI compile/upload plus independent protocol-identity verification passed; Arduino IDE
  is a documented manual fallback. See `logs/phase1_1_reference_firmware_restore_2026-08-06.md`.
- Rust exists at `C:\Users\dd00055\.cargo\bin` but is not on the persistent PowerShell PATH.
  Prepend it for the session or repair the user PATH entry.
- Phase 1 intentionally has no firmware editor, pulse-ox sequence, calibration wizard,
  physiological interpretation, clinical SpO2, heart-rate analysis, or BP estimation.
