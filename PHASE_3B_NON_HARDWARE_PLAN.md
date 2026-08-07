# Phase 3B non-hardware validation plan

1. Confirm the Phase 3B implementation checkpoint and the single intended UNO R4 WiFi device.
2. Launch the packaged application and obtain user-observed Validation-page results for the required
   viewport/scaling matrix; make only demonstrated responsive-layout fixes.
3. Verify normal controlled-firmware handshake, then perform one safe idle reset/retry
   characterization without uploading firmware; preserve a failure as a documented limitation.
4. Start an A0/12-bit/1000 Hz Until-stopped recording only after the user confirms participation,
   ask for a physical USB unplug/reconnect, and validate the incomplete recording and distinct
   post-reconnect session.
5. Add regression tests for any behavior changed, run the required checks, update evidence without
   claiming ECG/EMG bench validation, then create and push a non-hardware checkpoint commit.

Safety boundary: no person, electrodes, ECG/EMG module, or external bench source is involved.
