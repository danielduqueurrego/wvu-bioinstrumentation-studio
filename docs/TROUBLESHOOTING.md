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

## Older or newer WVU firmware

The board runs the WVU firmware, but not the build this app needs. The app accepts exactly its own build (build `0x00010007` for this version), so **every board needs one Restore WVU Firmware** after the app is updated. Retry connection is not offered, because a retry gets the same answer.

- **Older WVU firmware.** The message says **The board has an older WVU firmware version**. Select **Restore WVU Firmware** to update the board; follow the steps in [No WVU firmware](#no-wvu-firmware-firmware-update-required), including the double reset.
- **Newer WVU firmware.** The message says **The board has a newer WVU firmware version**: this computer's app is older than the firmware on the board, usually because another computer was updated first and restored the board. Update the app on this computer. **Restore WVU Firmware** also works, but it puts this app's older firmware back on the board, and an updated computer will then ask for a restore again.
- **Version 1.0.x.** The app versions 1.0.0 to 1.0.2 do not know these messages: with a board restored by a newer app they report **Firmware update required** (the board's firmware build is incompatible) and point to **Restore WVU Firmware**. Restoring there puts the old firmware back. Update that computer instead.

**Rollout rule for instructors and IT:** update the app on **every** computer that connects to the course boards first (lab computers, instructors' laptops, and students' own installations), then restore each board once from an updated computer. Until both are done, a board and a computer that disagree show one of these messages and Start stays unavailable. A Simulator recording still works.

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
- **Where the tools are.** They are kept in `%LOCALAPPDATA%\WVU-BMEG\rt`, outside the installation folder, so reinstalling alone does not replace them; use Repair first. On an account with a long Windows profile folder name they are kept in `%ProgramData%\WVU-BMEG-<id>\rt` instead (see the next section).
- **If Repair fails.** When the tools already in place are still complete (for example another program held a file, so nothing was replaced), they are kept and **Restore WVU Firmware** stays available; the information line says why the repair failed. Otherwise the Board card asks for Repair again.
- **If the tools are missing.** If the app or Repair reports that the included tools are missing or incomplete, reinstall the course distribution. Repair is not offered then, because it cannot help. Share Advanced details or the diagnostic log with the instructor.

Simulator recordings still work while the tools need attention.

## Arduino tools cannot be set up for this Windows account

The included Arduino compiler fails when the folder that holds the tools has 60 or more characters. The usual folder, `C:\Users\<profile folder>\AppData\Local\WVU-BMEG\rt`, has 35 characters plus the Windows profile folder name, so profile folders with more than 24 characters are too long, for example `firstname.lastname.CAMPUSDOMAIN` or a long name with a `.000` suffix. In PowerShell, `$env:LOCALAPPDATA` shows the profile folder.

- **What the app does.** In that case it keeps the tools in `%ProgramData%\WVU-BMEG-<id>\rt` (`<id>` is derived from the Windows account), a folder that only this account, SYSTEM and Administrators can open. Nothing changes for the student.
- **When that is not possible.** If `%ProgramData%` cannot be used (for example the computer's policy blocks it, or a folder of that name already belongs to another account), the Board card (or, with Source = Hardware, the message under the recording bar) says **The Arduino tools cannot be set up for this Windows account**. Repair is not offered: it extracts the tools to the same folder. Advanced details shows the folder and its length.
- **For IT support.** Allow standard users to create folders in `%ProgramData%` (the Windows default), remove a stale `%ProgramData%\WVU-BMEG-<id>` folder that another account owns, or give the student an account with a shorter profile folder name. `application.log` records `ARDUINO_RUNTIME_ROOT` (fallback used), `ARDUINO_RUNTIME_SHARED_ROOT_REFUSED` (why the fallback folder was refused) or `ARDUINO_RUNTIME_ROOT_TOO_LONG`. Once the fallback is in use, an old `%LOCALAPPDATA%\WVU-BMEG\rt` left by version 1.0.x (about 700 MB) is no longer used and can be deleted; the app never deletes it. The `%ProgramData%\WVU-BMEG-<id>` folders (about 700 MB each, one per such account) are not removed by uninstalling the app or by deleting the Windows profile, so on shared computers that remove student profiles they build up; an administrator can delete them once the account is gone (the app extracts the tools again if the account comes back).
- **Versions 1.0.0 to 1.0.2.** They have the same compiler limit without the fallback: firmware restore fails with a compiler error such as `bits/c++config.h: No such file or directory`. Update the app on that computer.
- **Profile folder names with other alphabets (known limitation).** The included compiler also fails when the Windows profile folder name has a character that the computer's Windows "language for non-Unicode programs" cannot represent, for example Chinese characters on a computer set to English: firmware restore stops with a compiler error ending in `Invalid argument`, and the path in it shows `??` in place of those characters. Characters of that language, such as `ë`, `é` or `ñ` on an English computer, work. The app has no fallback for this case: use a Windows account whose profile folder name uses only such characters. Simulator recordings still work.

Simulator recordings still work.

## Board did not answer

After a failed connection, select **Retry connection**. This is a new handshake that neither resets the board nor uploads firmware. Then select **Verify Firmware**. If the board still does not answer, use **Restore WVU Firmware**, which handles the board's double-reset upload mode. The former "Reset board and retry" action was retired because it could not reach the native-USB WVU firmware.

## Diagnostic information

The collapsed **Advanced details** section contains firmware/protocol, connection, and recent error information. Application logs are stored in the per-user application-data folder (`%LOCALAPPDATA%\WVU Bioinstrumentation Studio\logs`) and can be shared with an instructor without including recordings.

## Live plot starts with a brief jump

The first hardware ADC conversion can settle after a recording starts. When that first frame is an identifiable converging transient, the live display omits it so the y-axis remains readable. In an LED-sequenced pulse-ox lab this check compares samples of the same LED state only, so the normal step between RED and DARK rows is never taken for a transient. The raw `.bmeg` and `.csv` files still retain every acquired sample. Live x-axis labels are elapsed recording seconds beginning at 0.

## Plots say SIMULATED DATA

The plotted data came from **Simulator (practice)**, which records simulated data. Its metadata says so too. The watermark follows the data on the plots, not the current Source: plots from a Simulator recording keep it after you select **Hardware**, until a new recording replaces them. To record from the board, select **Hardware** in the recording bar before **Start recording**. Hardware is selected automatically only the first time the firmware is verified ready, and never during or after a recording.
