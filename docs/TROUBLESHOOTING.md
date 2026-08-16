# Troubleshooting

## Arduino not detected

Confirm that the Arduino UNO R4 WiFi is connected directly by USB, then select **Refresh Board**. Close Arduino IDE, Serial Monitor, Serial Plotter, and any other program using the board.

## Firmware update required

Select the detected board, use **Verify Firmware**, and then select **Restore WVU Firmware** if the app still reports that an update is required. Restore compiles and uploads the repository-controlled reference firmware, waits for the board to return, and verifies it before recording is enabled.

## Arduino port is busy

Close other serial applications, reconnect the board if necessary, select **Refresh Board**, then try again.

## Recording cannot start

Use a writable Project folder and an Output folder that is relative to it. Absolute output paths and paths containing `..` are rejected. The Advanced details section records the start stage and technical detail for instructor troubleshooting.

## Board disconnected during recording

The application finalizes the available raw data as incomplete and reports the disconnect. Reconnect the board, refresh it, verify firmware if needed, and start a new session; recordings are never concatenated across a disconnect.

## Arduino tools problem

Restart the application to retry first-run runtime preparation. If the problem continues, reinstall the course distribution and share Advanced details or the diagnostic log with the instructor.

## Reset board and retry

The explicit reset/retry action can observe a returning COM port that produces no protocol response on some systems. It never uploads firmware automatically. Use **Restore WVU Firmware** if ordinary verification cannot recover communication.

## Diagnostic information

The collapsed **Advanced details** section contains firmware/protocol, connection, and recent error information. Application logs are stored in the per-user application-data folder and can be shared with an instructor without including recordings.

## Live plot starts with a brief jump

The first hardware ADC conversion can settle after a recording starts. When that first frame is an identifiable converging transient, the live display omits it so the y-axis remains readable; the raw `.bmeg` and `.csv` files still retain every acquired sample. Live x-axis labels are elapsed recording seconds beginning at 0.
