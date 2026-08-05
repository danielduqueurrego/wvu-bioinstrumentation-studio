# WVU Bioinstrumentation Studio — Codex Starter

This bundle is the controlled starting point for developing **WVU Bioinstrumentation Studio**.

**Subtitle:** Firmware, Acquisition, and Calibration for BMEG 420L

## Intended use

This software and the associated electronics are for teaching and engineering characterization. They are **not medical devices** and must not be represented as diagnostic, therapeutic, or clinical monitoring equipment.

Do not connect a person to the biomedical instrumentation hardware while it is simultaneously connected to grounded bench instruments or other non-isolated equipment. Initial Codex hardware tests must use the Arduino UNO R4 WiFi by itself or safe bench signals only.

## How to begin

1. Extract this bundle into a new local folder, preferably:
   `C:\Users\<your-user>\Documents\wvu-bioinstrumentation-studio`
2. Open PowerShell in that folder.
3. Run:
   `powershell -ExecutionPolicy Bypass -File .\scripts\check_environment.ps1`
4. Review `environment-report.txt`.
5. Connect one Arduino UNO R4 WiFi using a known data-capable USB-C cable.
6. Close Arduino IDE Serial Monitor, PuTTY, or any other program holding the COM port.
7. Open the folder in Visual Studio Code.
8. Start Codex from the repository root.
9. Paste the entire contents of `codex\CODEX_START_PROMPT.md`.

## What Codex should accomplish first

The first Codex task is deliberately limited to a tested vertical slice:

- Inspect and document the local toolchain.
- Create the Git repository and application scaffold.
- Detect the connected UNO R4 WiFi.
- Build and upload a safe reference firmware sketch.
- Stream one analog channel over a versioned binary USB protocol.
- Display a bounded live plot.
- Save a short recording and export CSV plus JSON metadata.
- Add a device simulator and automated tests.
- Produce a Phase 1 report with exact commands and measured results.

Do not begin pulse-oximetry human measurements, pressure-cuff measurements, or electrode measurements during this first task.
