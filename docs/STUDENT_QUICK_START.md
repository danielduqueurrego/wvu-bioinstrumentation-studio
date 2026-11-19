# Student Quick Start

1. Before your first lab, install the version of WVU Bioinstrumentation Studio that your instructor names (for example 1.1.0), over any older version, and connect the Arduino UNO R4 WiFi.
2. Open the application and check the status in the **Board** card, for example **Ready on COM5**. The card always shows the one action that fits; **Verify Firmware** and **Restore WVU Firmware** are always available, as that action or under **More board actions**.
3. If the Board card shows **No WVU firmware** or **Older WVU firmware**, select **Restore WVU Firmware** and follow the steps in the restore window. (**Newer WVU firmware** means the app on this computer is out of date: tell your instructor.)
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

For pulse oximetry, the board steps the LEDs RED, DARK 1, IR, DARK 2. At the end of each state it records one row with the raw values of every input of the lab (TX and RX in the built-in lab), read while that state's LEDs are still set. In the CSV, `led_state_name` says which state each row belongs to (`RED`, `DARK1`, `IR`, or `DARK2`), so each state has a quarter of the rows. In the live plot, **Show by LED state** draws a signal as four traces, one per state, or as one trace; it changes the display only. The application does not calculate SpO2, heart rate, or other physiological results.

If something goes wrong, the message under the recording bar names the problem and usually offers one button for the next step. If that does not help, open **Advanced details** at the bottom of the window and show it to your instructor.

## Install a lab file

Your instructor may give you a lab file (`.lab.json`) for a new or changed lab. Install it only when your instructor asks you to.

1. Make sure no recording is running. **Install lab file…** is disabled during a recording.
2. In the **Lab** card, select **Install lab file…** under **Selected lab**, then choose the file.
3. Read the question. It names the lab, its version, its inputs, and its rate.
   - If the lab is new, it is added to your lab list.
   - If it replaces a lab on your computer, the question says so, for example "This replaces General Analog — Development 2.0.0 on this computer. Your instructor can undo this with Restore course default."
4. Select **Install** (or **Install and replace**) to continue, or **Cancel** to change nothing.
5. The installed lab is selected, and the **Lab** card says what was installed.

The application refuses a file that is damaged, a file that conflicts with a different lab of the same version on your computer, a lab your instructor retired on this computer, a course default file (meant for the next application version), and the older Op Amp Lab file. It says why before asking you anything. When the lab is already installed and selected, it says so and changes nothing; when it is installed but another version is in use, installing selects it again. If a file is refused, show the message to your instructor. Recordings you already made keep their own lab snapshot and are not changed.

## Op Amp Lab

**Op Amp Lab** is built into the application. Select it in the **Lab** card like any other course lab. It records **Vmid** on A0 and **Vout** on A1 at 100 frames/s, and its CSV columns are `vmid_counts` and `vout_counts`, each with its `_V` voltage column. It does not replace **General Analog — Development**; both labs are listed.

The separate Op Amp Lab installer (`install_op_amp_lab_profile.bat` and `.ps1`) is no longer needed. Do not run it with this version: it would put the older copy back in place of General Analog.

If you ran that installer before, the application fixes your Lab list automatically the first time it starts:

- **What changes.** The older Op Amp Lab copy, which had taken General Analog's place, is retired, and **General Analog — Development** is back in the list. Your other labs, recordings, and calibrations are kept.
- **Backup.** Before the change, the application saves a copy of your lab list as `lab_state.pre-migration-<date and time>.json` in `%LOCALAPPDATA%\WVU Bioinstrumentation Studio\profiles`.
- **Notice.** The **Lab** card says what changed and where the backup is, until you close the application. This happens only once.
- **Older recordings.** Recordings made with the older copy keep their own lab snapshot and are unchanged.

Teaching use only — not a medical device.
