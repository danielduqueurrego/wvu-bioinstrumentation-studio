# WVU Bioinstrumentation Studio 1.0.1

This maintenance release preserves the accepted BMEG 420L workflows while improving recording, startup, catalog, and deployment reliability.

## Fixed

- Long-running `Until stopped` acquisition is stable on Arduino UNO R4 WiFi; genuine serial and no-data failures retain detailed diagnostics.
- The initial hardware ADC settling transient no longer distorts the live plot scale. Raw BMEG and CSV samples remain unchanged and authoritative.
- Live plot time labels show elapsed recording seconds beginning at zero.
- Startup failures are logged and shown in a native Windows message instead of appearing only as a brief console window.
- Instructor lab-catalog changes are transactional, and an explicit factory reset can recover a malformed local catalog without silently redirecting data.

## Reliability and release maintenance

- Arduino runtime deployment is atomic and recoverable.
- Application, lab-catalog, and firmware-job logs are bounded.
- Release checks validate pinned firmware/runtime hashes and required third-party notices.

Requirements remain Windows and Arduino UNO R4 WiFi. Firmware protocol `0.3`, reference firmware build `0x00010003`, Arduino CLI `1.5.2-rc.1`, and Arduino Renesas UNO core `1.6.0` are unchanged.

Teaching use only — not a medical device. Follow BMEG 420L lab instructions and instructor safety procedures. Do not use this software for diagnosis or clinical decisions.
