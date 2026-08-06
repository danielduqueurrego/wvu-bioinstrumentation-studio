# Phase 1.1 controlled reference firmware restoration — 2026-08-06

## Scope

UNO R4 WiFi alone; no person, biomedical accessory, optical hardware, or pressure hardware was
connected. D4, D5, and D6 remain LOW in the repository sketch. No ESP32-S3 connectivity firmware
was installed or changed.

## Board and toolchain

| Item | Measured value |
|---|---|
| Board | Arduino UNO R4 WiFi |
| Application port before/after | COM12 / COM12 |
| USB serial | 48CA4360243C |
| FQBN | `arduino:renesas_uno:unor4wifi` |
| Arduino CLI | 1.5.2-rc.1 |
| Renesas UNO core | 1.6.0 |

`COM3` was present but had no UNO match and was never selected.

## Reproducible source defect and correction

The user had first used Arduino IDE to upload the exact repository sketch after an analog ASCII
test sketch replaced it. An independent production-parser probe showed the original repository
HELLO payload as:

```text
34 4F 4E 55 B8 0B 00 00 01 0C 01 00
```

This was not the required `(firmware_build, device_id, board, ADC, channels, reserved)` order.
Inspection reproduced the defect: `sendHello()` wrote both 32-bit values at offset `p`, leaving
the device-ID offset uninitialized. The controlled, protocol-v0.1 correction writes the device
ID at `p + 4` and sets the corrected reference build to `0x00010001`. A Rust regression test
asserts the documented offsets, and the host now rejects a valid-frame handshake whose identity is
missing or incompatible.

## Controlled compile/upload and independent identity proof

The corrected repository sketch was compiled with:

```powershell
arduino-cli compile --clean --fqbn arduino:renesas_uno:unor4wifi --output-dir logs\firmware_build_20260806_verified firmware\reference_unor4wifi
```

Result: exit 0; 53,508 bytes (20%) flash and 7,940 bytes (24%) RAM. The exact resulting
`reference_unor4wifi.ino.bin` was uploaded to rediscovered COM12 using Arduino CLI with
`--input-file --verbose --verify`; upload exit was 0. A later reset-retry characterization left
the protocol silent, so this same verified controlled binary was explicitly re-uploaded to leave
the board in a known application state.

The independent `phase1_capture probe` uses the production Rust `FrameParser`, sends one
CRC-valid PING, and read these CRC-valid frames after the final upload:

| Requirement | Result |
|---|---|
| HELLO | true |
| CAPABILITIES | true |
| PONG | true |
| CRC failures / invalid frames / skipped noise | 0 / 0 / 0 |
| Protocol version | 0.1 |
| Firmware build | `0x00010001` (65537) |
| Device ID | `0x554E4F34` (1431195444) |

The normal production `retry_handshake` then passed on COM12 in 1261 ms, receiving 64 bytes and
three valid frames. Upload exit status is therefore corroborated by the required binary identity,
rather than treated as proof by itself.

## Installer status

Arduino IDE was a successful initial recovery path, and the controlled Arduino CLI compile/upload
plus independent identity verification also passed in this run. The desktop UI does **not** yet
bundle an in-app reference-firmware installer; that feature is not claimed as accepted and is not
required for the normal Phase 1.1 recording path.
