# Student Quick Start

1. Install WVU Bioinstrumentation Studio and connect the Arduino UNO R4 WiFi.
2. Open the application and check the status in the **Board** card, for example **Ready on COM5**. The card always shows the one action that fits; **Verify Firmware** and **Restore WVU Firmware** are always available, as that action or under **More board actions**.
3. If the Board card shows **No WVU firmware**, select **Restore WVU Firmware** and follow the steps in the restore window.
   - Connect only this one UNO board.
   - If the window asks you to **press the RESET button twice quickly**, do so before the countdown ends.
   - Do not unplug the board while it says "Writing firmware".
4. Choose a writable **Project folder**.
5. Select the lab assigned by your instructor.
6. Enter an optional relative **Output folder** for this trial, such as `Participant01\Trial03`.
7. Choose duration, display units, and plot arrangement as instructed.
8. **Choose Hardware.** In the recording bar at the top of the window, check that **Source** is **Hardware**. It is selected for you the first time the firmware is verified. **Simulator (practice)** records simulated data, and its plots say **SIMULATED DATA**.
9. Select **Start recording**. If it is not available, the checklist in the recording bar says what is missing.
10. Stop the recording when directed with **Stop recording**, or wait for a timed run to finish.
11. Check the **Last recording** card. It shows whether the `.bmeg`, `.csv`, and metadata files were saved (✓ or ✗). Use **Show in folder** to open the file in File Explorer, or **Copy path**. Any recording markers are included in the metadata.

The CSV contains raw counts and applicable derived voltage or engineering-unit columns. The raw BMEG data remain authoritative.

Recording filenames identify the selected lab and run without internal development-phase labels.

While recording, plot changes are display-only and never change the underlying recording:

- **Plot time window.** Choose 1, 2, 5, 10, or 30 s, or a custom 0.5–30 s. The window applies to all plots, and the time axis shows elapsed recording seconds from 0.
- **Legends and values.** Multi-signal legends identify waveform colors, and newest-value badges show the current rounded display value.
- **Pausing.** **Pause display** freezes the plots while the recording continues; **Resume display** jumps to the newest data.
- **Arranging.** You may rearrange plots, hide or show traces, or change supported units.
- **Markers.** When instructed, use **Add marker** (with an optional short label). Each marker appears as a labelled line on the plots.

For pulse oximetry, the firmware waits the configured settling interval after each RED, DARK 1, IR, or DARK 2 transition before recording the raw values for both detectors. It does not calculate SpO2, heart rate, or other physiological results.

If something goes wrong, the message under the recording bar names the problem and usually offers one button for the next step. If that does not help, open **Advanced details** at the bottom of the window and show it to your instructor.

## Instructor-provided lab installer

If your instructor provides the Op Amp Lab installer, keep these two files in the same folder:

- `install_op_amp_lab_profile.bat`
- `install_op_amp_lab_profile.ps1`

Close WVU Bioinstrumentation Studio, then double-click `install_op_amp_lab_profile.bat`. The installer window remains open so you can confirm whether installation succeeded.

Alternatively, run the PowerShell installer directly from the folder containing it:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\install_op_amp_lab_profile.ps1
```

The script requires no administrator access and installs **Op Amp Lab 2.0.1** only for the current Windows user.

- **Lab list.** Op Amp Lab is a revision of **General Analog — Development**, so it takes that lab's place in your Lab list. The other labs are unchanged.
- **Backup.** The script creates a backup before changing an existing catalog. The per-user catalog is stored under `%LOCALAPPDATA%\WVU Bioinstrumentation Studio\profiles`.

Reopen the application after the script reports success.

Teaching use only — not a medical device.
