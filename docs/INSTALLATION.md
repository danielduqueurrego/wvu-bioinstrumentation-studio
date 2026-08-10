# Installation

## Requirements

- Windows 11 x64 (Windows 10 x64 may also work when the required WebView2 runtime can be installed).
- Arduino UNO R4 WiFi for hardware acquisition.

No Arduino IDE, Arduino CLI, Renesas board package, Node.js, Rust, Git, or development tools are required for students.

## Install

1. Run `WVU Bioinstrumentation Studio_1.0.0_x64-setup.exe`.
2. Accept the per-user installation prompts.
3. Start **WVU Bioinstrumentation Studio** from the Start menu.

On first start, the application prepares its included Arduino tools in its own local application-data folder. It does not modify an existing Arduino IDE installation or its configuration. The installer includes an offline WebView2 installer so ordinary course use can proceed without an Internet connection after installation.

## Verify

Connect the Arduino UNO R4 WiFi, start the application, and confirm:

- Arduino: Connected
- Firmware: Ready
- Arduino tools: Ready

If firmware needs attention, choose **Firmware** then **Restore WVU Firmware**. This replaces the sketch currently on the board.

## Uninstall

Use Windows **Installed apps** to uninstall WVU Bioinstrumentation Studio. Uninstalling does not delete student recordings, sketches, or saved local calibrations by default.
