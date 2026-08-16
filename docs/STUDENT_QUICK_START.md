# Student Quick Start

1. Install WVU Bioinstrumentation Studio and connect the Arduino UNO R4 WiFi.
2. Open the application and confirm the Board and Firmware status at the top of the window.
3. If firmware needs attention, select **Restore WVU Firmware** and wait for it to complete.
4. Choose a writable **Project folder**.
5. Select the lab assigned by your instructor.
6. Enter an optional relative **Output folder** for this trial, such as `Participant01\Trial03`.
7. Choose duration, display units, and plot arrangement as instructed.
8. Select **Connect, configure, and start recording**.
9. Stop the recording when directed, or wait for a timed run to finish.
10. Find the `.bmeg`, `.csv`, metadata, and any event sidecar in the shown Project/Output folder.

The CSV contains raw counts and applicable derived voltage or engineering-unit columns. The raw BMEG data remain authoritative.

While recording, plot changes are display-only: the **Plot time window** applies to all plots from 0.5 to 30 seconds, the time axis shows elapsed recording seconds from 0, multi-signal legends identify waveform colors, and newest-value badges show the current rounded display value. You may rearrange plots, hide or show traces, change supported units, or use **Add marker** when instructed without changing the underlying recording.

For pulse oximetry, the application records raw RED, DARK 1, IR, and DARK 2 measurements for both detectors. It does not calculate SpO2, heart rate, or other physiological results.

Teaching use only — not a medical device.
