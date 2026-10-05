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
10. Find the `.bmeg`, `.csv`, and metadata files in the shown Project/Output folder. Any recording markers are included in the metadata.

The CSV contains raw counts and applicable derived voltage or engineering-unit columns. The raw BMEG data remain authoritative.

Recording filenames identify the selected lab and run without internal development-phase labels.

While recording, plot changes are display-only: the **Plot time window** applies to all plots from 0.5 to 30 seconds, the time axis shows elapsed recording seconds from 0, multi-signal legends identify waveform colors, and newest-value badges show the current rounded display value. You may rearrange plots, hide or show traces, change supported units, or use **Add marker** when instructed without changing the underlying recording.

For pulse oximetry, the firmware waits the configured settling interval after each RED, DARK 1, IR, or DARK 2 transition before recording the raw values for both detectors. It does not calculate SpO2, heart rate, or other physiological results.

## Instructor-provided lab installer

If your instructor provides the Op Amp Lab installer, keep these two files in the same folder:

- `install_op_amp_lab_profile.bat`
- `install_op_amp_lab_profile.ps1`

Close WVU Bioinstrumentation Studio, then double-click `install_op_amp_lab_profile.bat`. The installer window remains open so you can confirm whether installation succeeded.

Alternatively, run the PowerShell installer directly from the folder containing it:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\install_op_amp_lab_profile.ps1
```

The script requires no administrator access. It installs **Op Amp Lab 2.0.1** only for the current Windows user, preserves other labs, and creates a backup before changing an existing catalog. The per-user catalog is stored under `%LOCALAPPDATA%\WVU Bioinstrumentation Studio\profiles`. Reopen the application after the script reports success.

Teaching use only — not a medical device.
