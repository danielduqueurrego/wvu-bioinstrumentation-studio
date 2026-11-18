# Troubleshooting

## Arduino not detected

Confirm that the Arduino UNO R4 WiFi is connected directly by USB, then select **Refresh Board**. Close Arduino IDE, Serial Monitor, Serial Plotter, and any other program using the board.

## No WVU firmware (firmware update required)

Select the detected board, use **Verify Firmware**, and then select **Restore WVU Firmware** if the Board card shows **No WVU firmware**. Verify and Restore are always available in the Board card (as its main action or under **More board actions**), whatever the firmware status. Restore compiles and uploads the repository-controlled reference firmware, waits for the board to return, and verifies it before recording is enabled. The restore window shows its steps: Prepare, Compile, Switch board to upload mode, Upload, Reconnect, Verify.

A board already running the WVU firmware must be switched to upload mode by hand: when the restore window shows **Press the RESET button twice quickly**, double-press the board's RESET button before the countdown (90 s) ends. If the time runs out nothing is written; select **Restore WVU Firmware** again.

Connect only the board you are restoring. Restore is refused while more than one Arduino UNO R4 WiFi is connected, because the board changes its COM port during the upload and could otherwise be confused with another one. Disconnect the other UNO boards, select **Refresh Board**, and try again.

**Cancel restore** is available until the upload starts. Once the app shows "Writing firmware — do not unplug the board", the restore cannot be canceled (Escape does not cancel it either) and the window stays open until it finishes; keep the board connected.

The **Last firmware operation** card in the Board card keeps the result of the last restore or failed verification: what happened, what to do next, the final COM port, and the log file. **Copy diagnostics** copies the details, including the technical codes, for your instructor.

## Arduino port is busy

The Board card shows **Port busy**. Close Arduino IDE, Serial Monitor, Serial Plotter, or Arduino Cloud Agent, reconnect the board if necessary, then select **Close other serial apps, then Verify**.

## Recording cannot start

Use a writable Project folder and an Output folder that is relative to it. Absolute output paths and paths containing `..` are rejected. The Advanced details section records the start stage and technical detail for instructor troubleshooting.

## Board disconnected during recording

The application finalizes the available raw data as incomplete and reports the disconnect. Reconnect the board, refresh it, verify firmware if needed, and start a new session; recordings are never concatenated across a disconnect.

## Arduino tools problem

If the message under the recording bar says **The Arduino tools need attention**, select **Repair Arduino tools** (in the message or in the Board card). Repair extracts the offline Arduino tools included with the application again (nothing is downloaded); it waits until no recording or board operation is running. The app also checks the prepared tools at every start and extracts them again if a file is missing or changed, for example after an antivirus scan.

The tools are kept in `%LOCALAPPDATA%\WVU-BMEG\rt`, outside the installation folder, so reinstalling alone does not replace them; use Repair first. If the app or Repair reports that the included tools are missing or incomplete (Repair is not offered then, because it cannot help), reinstall the course distribution and share Advanced details or the diagnostic log with the instructor.

## Board did not answer

After a failed connection, select **Retry connection** (a new handshake that neither resets the board nor uploads firmware), then **Verify Firmware**. If the board still does not answer, use **Restore WVU Firmware**, which handles the board's double-reset upload mode. The former "Reset board and retry" action was retired because it could not reach the native-USB WVU firmware.

## Diagnostic information

The collapsed **Advanced details** section contains firmware/protocol, connection, and recent error information. Application logs are stored in the per-user application-data folder and can be shared with an instructor without including recordings.

## Live plot starts with a brief jump

The first hardware ADC conversion can settle after a recording starts. When that first frame is an identifiable converging transient, the live display omits it so the y-axis remains readable; the raw `.bmeg` and `.csv` files still retain every acquired sample. Live x-axis labels are elapsed recording seconds beginning at 0.
