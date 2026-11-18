# Troubleshooting

The message center under the recording bar shows one problem at a time, usually with one button for the next step: **Retry connection**, **Verify Firmware**, **Restore WVU Firmware**, **Clear connection**, **Choose another folder**, **Repair Arduino tools**, or **Refresh Board**. The sections below are named after the messages you may see.

## Arduino not detected

The Board card says **No board selected**. Confirm that the Arduino UNO R4 WiFi is connected directly by USB, then select **Refresh Board**. Close Arduino IDE, Serial Monitor, Serial Plotter, and any other program using the board. When exactly one UNO R4 WiFi is found, it is selected and verified automatically. When several are found, choose one from the list.

## No WVU firmware (firmware update required)

Select the detected board and use **Verify Firmware**. If the Board card then shows **No WVU firmware**, select **Restore WVU Firmware**.

- **Always available.** Verify and Restore are always available in the Board card, as its main action or under **More board actions**, whatever the firmware status. Verify only reads from the board; it never uploads firmware.
- **What Restore does.** It compiles and uploads the repository-controlled reference firmware, waits for the board to return, and verifies it before recording is enabled. The restore window shows its steps: Prepare, Compile, Switch board to upload mode, Upload, Reconnect, Verify.
- **Double reset.** A board already running the WVU firmware must be switched to upload mode by hand. When the restore window shows **Press the RESET button twice quickly**, double-press the board's RESET button before the 90 s countdown ends. If the time runs out, nothing is written; select **Restore WVU Firmware** again.
- **One board only.** Connect only the board you are restoring. Restore is refused while more than one Arduino UNO R4 WiFi is connected, because the board changes its COM port during the upload and could otherwise be confused with another one. Disconnect the other UNO boards, select **Refresh Board**, and try again.
- **Canceling.** **Cancel restore** is available until the upload starts. Once the app shows "Writing firmware — do not unplug the board", the restore cannot be canceled (Escape does not cancel it either), and the window stays open until it finishes. Keep the board connected.
- **If the upload fails.** If the restore reports that the upload failed, close Arduino IDE, Serial Monitor, and Arduino Cloud Agent. Press the board's RESET button twice quickly, select **Refresh Board**, and restore again.
- **Results.** The **Last firmware operation** card in the Board card keeps the result of the last restore or failed verification: what happened, what to do next, the final COM port, and the log file. **Copy diagnostics** copies the details, including the technical codes, for your instructor.

## Arduino port is busy

The Board card shows **Port busy**. A board that was already **Ready** keeps its chip; the message under the recording bar or the **Last firmware operation** card says **Another program is using the board** instead. Close Arduino IDE, Serial Monitor, Serial Plotter, or Arduino Cloud Agent, and reconnect the board if necessary. Then select **Close other serial apps, then Verify** (or **Verify Firmware**). A busy port does not mean the firmware is wrong; do not restore because of it.

## Recording cannot start

If **Start recording** cannot be used, the checklist in the recording bar names what is missing: Board, Firmware, Lab, Folder, or Source. Select an item to jump to its control.

- **Folders.** Use a writable Project folder and an Output folder that is relative to it. Absolute output paths, paths containing `..`, and Windows-reserved names such as `CON` or `NUL` are rejected.
- **Hardware source.** **Hardware** needs a board whose firmware was verified on the selected COM port. **Simulator (practice)** needs neither.

Advanced details records the start stage and technical detail for instructor troubleshooting.

**Not enough free space.** If the Recording duration card reports that a timed recording and its CSV will not fit on the drive, **Start recording** still works, but the recording stops early to protect the drive. Free some space, choose another Project folder, or shorten the recording first. Start is refused only when less than 250 MiB is free. The message then says **The recording could not start** and gives the free space; nothing was recorded. Free space or choose another folder, then select **Clear connection**.

## The previous Arduino connection needs to be cleared

After a failed verification, connection, or recording, a hardware recording cannot start until the problem is cleared:

- **Board problems.** Use the message's button: **Retry connection** or **Verify Firmware**. A successful Verify clears the problem.
- **No need to verify again.** **Clear connection** (in the message or in the recording bar) returns to the ready state without verifying again. Use it after a save or folder problem, or when you only want to practise.
- **Simulator.** A Simulator recording can start even while this message is shown. Setup stays editable.

## The Arduino stopped sending samples

The board stopped delivering data for seven seconds. The recording ends instead of restarting itself, and the samples received so far are saved as an incomplete recording. Check the USB cable (connect directly, not through a hub if possible), select **Verify Firmware**, then start a new recording.

## The Arduino stopped the recording

The board reported a firmware error and stopped acquiring. Advanced details shows the error code; the codes are listed in [USB Protocol](USB_PROTOCOL.md#error_message-codes).

The most common cause is the board's command watchdog (code 7): the board received no command from the computer for five seconds, usually because the computer paused. The application keeps Windows from sleeping while it records, but closing a laptop lid or a forced sleep can still interrupt a recording.

The saved samples are kept. Verify the firmware, then start a new recording.

## Board disconnected during recording

The application finalizes the available raw data as incomplete and reports the disconnect. Reconnect the board, select **Refresh Board** (the COM number may change), verify the firmware, and start a new session. Recordings are never concatenated across a disconnect.

## The recording was not saved completely

The **Last recording** card marks each file ✓ Saved or ✗ Not saved and gives the reason.

- **Disk full or drive removed.** Check that the Project folder's drive is connected and has free space, or choose another folder. Then select **Clear connection**.
- **Recording stopped early.** When free space falls below 250 MiB plus room for the CSV, the recording stops early and is saved.
- **No CSV.** If only the CSV is missing, the raw `.bmeg` and the metadata are still complete. The metadata's `csv_error` says why.
- **`.bmeg.part` file.** A raw file that could not be finished keeps the name `.bmeg.part`. Files are never deleted or overwritten.

## Arduino tools problem

If the message under the recording bar says **The Arduino tools need attention** (while Source is **Simulator (practice)**, the Board card says it instead), select **Repair Arduino tools** (in the message or in the Board card).

- **What Repair does.** It extracts the offline Arduino tools included with the application again; nothing is downloaded. It is unavailable while a recording or board operation is running; finish or stop that first.
- **Automatic check.** The app also checks the prepared tools at every start and extracts them again if a file is missing or its size changed, for example after an antivirus scan. This check compares each file's presence and size, not its content.
- **Where the tools are.** They are kept in `%LOCALAPPDATA%\WVU-BMEG\rt`, outside the installation folder, so reinstalling alone does not replace them; use Repair first.
- **If Repair fails.** When the tools already in place are still complete (for example another program held a file, so nothing was replaced), they are kept and **Restore WVU Firmware** stays available; the information line says why the repair failed. Otherwise the Board card asks for Repair again.
- **If the tools are missing.** If the app or Repair reports that the included tools are missing or incomplete, reinstall the course distribution. Repair is not offered then, because it cannot help. Share Advanced details or the diagnostic log with the instructor.

Simulator recordings still work while the tools need attention.

## Board did not answer

After a failed connection, select **Retry connection**. This is a new handshake that neither resets the board nor uploads firmware. Then select **Verify Firmware**. If the board still does not answer, use **Restore WVU Firmware**, which handles the board's double-reset upload mode. The former "Reset board and retry" action was retired because it could not reach the native-USB WVU firmware.

## Diagnostic information

The collapsed **Advanced details** section contains firmware/protocol, connection, and recent error information. Application logs are stored in the per-user application-data folder (`%LOCALAPPDATA%\WVU Bioinstrumentation Studio\logs`) and can be shared with an instructor without including recordings.

## Live plot starts with a brief jump

The first hardware ADC conversion can settle after a recording starts. When that first frame is an identifiable converging transient, the live display omits it so the y-axis remains readable. The raw `.bmeg` and `.csv` files still retain every acquired sample. Live x-axis labels are elapsed recording seconds beginning at 0.

## Plots say SIMULATED DATA

The plotted data came from **Simulator (practice)**, which records simulated data. Its metadata says so too. The watermark follows the data on the plots, not the current Source: plots from a Simulator recording keep it after you select **Hardware**, until a new recording replaces them. To record from the board, select **Hardware** in the recording bar before **Start recording**. Hardware is selected automatically only the first time the firmware is verified ready, and never during or after a recording.
