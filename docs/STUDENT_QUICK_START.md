# Student Quick Start

1. Install WVU Bioinstrumentation Studio and connect the Arduino UNO R4 WiFi.
2. Open the application and check the status in the **Board** card, for example **Ready on COM5**. The card always shows the one action that fits; **Verify Firmware** and **Restore WVU Firmware** are always available under **More board actions**.
3. If the Board card shows **No WVU firmware**, select **Restore WVU Firmware** and follow the steps in the restore window. If it asks you to **press the RESET button twice quickly**, do so before the countdown ends. Do not unplug the board while it says "Writing firmware".
4. Choose a writable **Project folder**.
5. Select the lab assigned by your instructor.
6. Enter an optional relative **Output folder** for this trial, such as `Participant01\Trial03`.
7. Choose duration, display units, and plot arrangement as instructed.
8. Check that **Hardware** is the Source in the recording bar at the top of the window (it is selected for you the first time the firmware is verified; **Simulator (practice)** records simulated data), then select **Start recording**.
9. Stop the recording when directed, or wait for a timed run to finish.
10. The **Last recording** card shows whether the `.bmeg`, `.csv`, and metadata files were saved (✓ or ✗). Use **Show in folder** to open the file in File Explorer, or **Copy path**. Any recording markers are included in the metadata.

The CSV contains raw counts and applicable derived voltage or engineering-unit columns. The raw BMEG data remain authoritative.

Recording filenames identify the selected lab and run without internal development-phase labels.

While recording, plot changes are display-only: the **Plot time window** (1, 2, 5, 10, or 30 s, or a custom 0.5–30 s) applies to all plots, the time axis shows elapsed recording seconds from 0, multi-signal legends identify waveform colors, and newest-value badges show the current rounded display value. **Pause display** freezes the plots while the recording continues; **Resume display** jumps to the newest data. You may rearrange plots, hide or show traces, change supported units, or use **Add marker** (with an optional short label) when instructed without changing the underlying recording; each marker appears as a labelled line on the plots. Plots of simulated data are marked **SIMULATED DATA**.

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
