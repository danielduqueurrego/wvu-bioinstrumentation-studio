# Phase 2 compile/upload acceptance — 2026-08-06

## Environment

- Arduino CLI: `arduino-cli 1.5.2-rc.1` at `C:\arduino-cli\arduino-cli.exe`
- Core: `arduino:renesas_uno 1.6.0`
- Board: Arduino UNO R4 WiFi, FQBN `arduino:renesas_uno:unor4wifi`
- Original/final application port: COM12 / COM12
- Board serial: `48CA4360243C`
- Unrelated port COM3 was present and ignored.

## Production-workflow sequence

The feature-gated `phase2_firmware_capture` invokes the same `FirmwareWorkspace`,
`FirmwareWorkflow`, and `SessionController` used by Tauri commands. It created a temporary
`A0AsciiDiagnostic` student project, then performed the following real CLI jobs.

1. Compile A0 ASCII diagnostic: success, 6,757 ms, 52,032 bytes program (19%), 6,740 bytes RAM
   (20%); no diagnostics.
2. Explicit confirmed upload of that current artifact to COM12: success, 6,276 ms. Arduino CLI
   reported its 1200-bps touch reset and `New upload port: COM12`. The project is declared
   `non_wvu`; the workflow reported upload success and set compatibility to `non_wvu_sketch`.
   It did not attempt a false binary-handshake failure and Acquisition remained disabled.
3. Explicit confirmed **Restore WVU reference firmware**: reference compile success, 6,838 ms,
   53,508 bytes program (20%), 7,940 bytes RAM (24%); upload success, 6,471 ms. CLI again used
   a 1200-bps touch and returned COM12.
4. Post-upload production-parser verification: HELLO/CAPABILITIES/PONG true; 64 received bytes;
   3 valid frames; CRC failures 0; protocol 0.1; build `0x00010001`; device `0x554E4F34`.
   Compatibility changed to `wvu_protocol_compatible`.

Application workflow logs were written outside the project/source tree:

- `C:\Users\dd00055\AppData\Local\WVU Bioinstrumentation Studio\firmware_workspace\logs\firmware_job_1_20260806_171422.json`
- `C:\Users\dd00055\AppData\Local\WVU Bioinstrumentation Studio\firmware_workspace\logs\firmware_job_2_20260806_171432.json`
- `C:\Users\dd00055\AppData\Local\WVU Bioinstrumentation Studio\firmware_workspace\logs\firmware_job_3_20260806_171443.json`

They retain the argument arrays, output, timestamps, source hashes, serial/port identity,
CLI/core versions, and verification result. The raw CLI text is the source of truth for a
bootloader transition; no distinct bootloader COM port was surfaced by this same-port run.

## Result

Passed. The in-app workflow code—not Arduino IDE—performed both tested uploads and restored the
controlled reference before the acquisition coordination check.
