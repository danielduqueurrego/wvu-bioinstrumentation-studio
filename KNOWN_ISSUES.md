# Known issues

- Physical UNO USB unplug/replug passed on 2026-08-07 with a controlled incomplete disconnect
  finalization, explicit COM12 rediscovery, protocol re-verification, and a separate successful
  post-reconnect recording. It does not resolve the separate Reset board and retry limitation.
  See `logs/phase1_physical_disconnect_verification_2026-08-06.md`.
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
- Formal ECG/EMG analog-module characterization is outside the runtime class application scope.
  Historical Phase 3B evidence remains in Git history; it neither gates course capture nor
  authorizes human use.
- Rust exists at `C:\Users\dd00055\.cargo\bin` but is not on the persistent PowerShell PATH.
  Prepend it for the session or repair the user PATH entry.
- The explicit 1200-bps Reset board and retry limitation is unchanged. Normal protocol v0.3
  acquisition and controlled firmware upload/restore work; recovery should not silently upload
  firmware.
- Phase 5 deliberately has no physiological interpretation, clinical SpO2, heart-rate analysis,
  SBP/DBP estimation, EMG activation/fatigue analysis, force conversion, or automatic filtering.
  Its MPXV and student-generated XGZP conversions are engineering-unit tools only.
- Phase 6 requires an explicit controlled-reference firmware update to protocol 0.3 / build
  `0x00010003` before an instructor-authored resource mapping can be captured on hardware. The
  application never performs that upload automatically; use the Firmware workspace’s explicit
  Restore WVU reference firmware action after reviewing the selected lab.
- Phase 6 catalog/versioning automated and focused user inspection passed. Factory course labs are
  bundled and require no import; catalog reads never create revisions. Remaining Phase 6 work is
  the separate controlled-firmware, simulator/UNO smoke, and full UI acceptance sequence. See
  `logs/phase6_lab_catalog_versioning_acceptance_2026-08-10.md`.
