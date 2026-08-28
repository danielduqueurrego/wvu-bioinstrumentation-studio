# WVU Bioinstrumentation Studio 1.0.2

This release improves pulse-ox timing, recording provenance, and recording filenames while preserving the accepted BMEG 420L acquisition and visualization workflows.

## Fixed

- Pulse-ox RED, DARK 1, IR, and DARK 2 use one absolute firmware schedule so ADC work, packet construction, and USB transmission do not accumulate into the cycle interval.
- Synchronized records preserve their firmware-measured timestamps instead of reconstructing uniformly spaced host timestamps.
- Bounded raw-batch and nonblocking transmit queues keep USB backpressure outside the sampling path and report overflow explicitly.
- Final recording metadata includes measured interval statistics and pulse-ox timing checks for each LED state.
- The unnecessary human-signal connection confirmation was removed; profile safety provenance remains recorded.
- Automatic `.bmeg`, `.csv`, and metadata filenames use the selected lab's stable export name rather than internal development-phase labels.

After installing this release, use **Restore WVU Firmware** once so the connected Arduino UNO R4 WiFi receives reference firmware build `0x00010006`.

Requirements remain Windows and Arduino UNO R4 WiFi. This build uses firmware protocol `0.4`, Arduino CLI `1.5.2-rc.1`, and Arduino Renesas UNO core `1.6.0`.

Teaching use only — not a medical device. Follow BMEG 420L lab instructions and instructor safety procedures. Do not use this software for diagnosis or clinical decisions.
