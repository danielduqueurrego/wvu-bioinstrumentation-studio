# Known issues and release follow-up

- The explicit **Reset board and retry** action can rediscover a returning UNO R4 WiFi port yet
  receive no firmware response. Normal connect, upload, restore, and acquisition remain separate
  supported paths. The action never uploads firmware automatically.
- The student release-candidate clean-install test remains pending until it is run in Windows
  Sandbox, a clean VM, or a fresh Windows user that has no Arduino IDE or Arduino15 directory.
- The final student UI viewport matrix and zero-external-console-window observation remain pending
  manual release-candidate inspection.
- The Windows installers are currently unsigned unless an approved institutional signing workflow
  is supplied. No ad-hoc signing certificate is created by this project.
- Formal analog-module characterization is outside the class application scope. It does not gate
  course capture and does not authorize diagnostic or clinical use.
- The application intentionally does not calculate physiological results such as heart rate,
  SpO2, SBP/DBP, EMG activation/fatigue, force, or clinical interpretation.
