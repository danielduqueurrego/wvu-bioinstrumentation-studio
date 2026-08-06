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
- The Phase 2 firmware workflow now has a controlled in-app reference restore. Its real CLI
  compile/upload/identity verification sequence passed on COM12; the explicitly separate
  Phase 1.1 reset/retry recovery issue remains as described above.
- Phase 2's post-fix UI inspection passed at 900 × 650 and wide/maximized windows. The exact
  numeric Windows scaling and separate 1024 × 768, 1366 × 768, and 1920 × 1080 observations were
  not recorded; they remain a nonblocking verification/documentation follow-up rather than a
  reported layout defect. See `logs/phase2_firmware_workspace_acceptance_2026-08-06.md`.
- Phase 3A is accepted for its bench-only profile framework and focused manual verification at
  3440 × 1392 / 100% scaling. Separate 900 × 650, 1024 × 768, 1366 × 768, 1920 × 1080,
  maximized, 125%, and 150% viewport/scaling observations remain pending documentation follow-up;
  they are not claimed as tested. See `logs/phase3a_manual_ui_verification_2026-08-06.md`.
- Phase 3B simulator validation is available, but physical ECG and EMG module/interface evidence
  cannot be claimed until a safe 0–5 V bench source, module identifiers, and instructor-entered
  equipment/criteria are available. No person or electrode system may be used to close that gap.
  The Phase 3B viewport/scaling matrix is also pending manual inspection.
- Rust exists at `C:\Users\dd00055\.cargo\bin` but is not on the persistent PowerShell PATH.
  Prepend it for the session or repair the user PATH entry.
- Phase 1 intentionally has no firmware editor, pulse-ox sequence, calibration wizard,
  physiological interpretation, clinical SpO2, heart-rate analysis, or BP estimation.
