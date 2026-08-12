# Known issues and release follow-up

- The explicit **Reset board and retry** action can rediscover a returning UNO R4 WiFi port yet
  receive no firmware response. Normal connect, upload, restore, and acquisition remain separate
  supported paths. The action never uploads firmware automatically.
- The student release-candidate clean-install test remains pending until it is run in Windows
  Sandbox, a clean VM, or a fresh Windows user that has no Arduino IDE or Arduino15 directory.
- The final student UI viewport matrix and zero-external-console-window observation remain pending
  manual release-candidate inspection.
- The revised release candidate still requires a focused manual recovery check with a detected UNO
  whose firmware reports **Update required**: Board selection, Refresh Board, Verify Firmware, and
  Restore WVU Firmware must remain available after the read-only verification attempt faults.
- The same focused recovery check must confirm that a successful firmware restore/verification
  leaves a fresh recording available immediately, without restarting the application.
- The focused release-app recording check remains pending: after firmware recovery, verify ECG
  `Test1`, ECG `Test2` without restart, and a short EMG recording. Any failure must be captured
  from **Advanced details** as a recording-start stage, code, and detail rather than reported as a
  generic Start error.
- The Windows installers are currently unsigned unless an approved institutional signing workflow
  is supplied. No ad-hoc signing certificate is created by this project.
- Formal analog-module characterization is outside the class application scope. It does not gate
  course capture and does not authorize diagnostic or clinical use.
- The application intentionally does not calculate physiological results such as heart rate,
  SpO2, SBP/DBP, EMG activation/fatigue, force, or clinical interpretation.
