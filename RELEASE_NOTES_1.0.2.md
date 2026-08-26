# WVU Bioinstrumentation Studio 1.0.2

This patch release improves pulse-ox phase timing and recording filenames while preserving the accepted BMEG 420L acquisition and visualization workflows.

## Fixed

- Pulse-ox RED, DARK 1, IR, and DARK 2 values are sampled only after the complete configured phase-settling interval.
- Automatic `.bmeg`, `.csv`, and metadata filenames use the selected lab's stable export name rather than internal development-phase labels.

After installing this release, use **Restore WVU Firmware** once so the connected Arduino UNO R4 WiFi receives reference firmware build `0x00010004`.

Requirements remain Windows and Arduino UNO R4 WiFi. Firmware protocol `0.3`, Arduino CLI `1.5.2-rc.1`, and Arduino Renesas UNO core `1.6.0` are unchanged.

Teaching use only — not a medical device. Follow BMEG 420L lab instructions and instructor safety procedures. Do not use this software for diagnosis or clinical decisions.
