# Known issues

- Physical USB unplug/replug has not been performed because it needs the user's manual
  participation. Automated mock-disconnect handling passes; the design requires an
  explicit new acquisition after a disconnect and never concatenates sessions.
- The 60-second acquisition evidence was collected through the production session
  worker/status API without manually manipulating the released GUI. The release app
  did launch successfully, but visual plot responsiveness remains a manual follow-up.
- Rust exists at `C:\Users\dd00055\.cargo\bin` but is not on the persistent PowerShell
  PATH. Prepend it for the session or repair the user PATH entry.
- Phase 1 intentionally has no firmware editor, pulse-ox sequence, calibration wizard,
  physiological interpretation, clinical SpO2, heart-rate analysis, or BP estimation.
