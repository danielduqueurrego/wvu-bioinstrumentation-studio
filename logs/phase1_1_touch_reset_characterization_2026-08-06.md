# Phase 1.1 touch-reset characterization — 2026-08-06

## Scope and safety

UNO R4 WiFi only; no biomedical accessory or person was connected. No ESP32-S3 connectivity
firmware was restored or changed. A reproducible repository HELLO-payload defect was corrected
without changing protocol v0.1; the resulting controlled application binary was explicitly
re-uploaded after reset testing left the application port silent.

## Board identity

- Initial and final observed application port: `COM12`
- Board: Arduino UNO R4 WiFi
- FQBN: `arduino:renesas_uno:unor4wifi`
- USB VID/PID: `2341:1002`
- USB serial: `48CA4360243C`
- Arduino CLI: 1.5.2-rc.1; UNO R4 core: 1.6.0
- Unrelated `COM3` was present and was never selected as a reset candidate.

## Confirmed observations

1. An Arduino analog ASCII test sketch had replaced the WVU reference firmware. The user then
   uploaded the repository reference sketch through Arduino IDE; that established hardware/USB
   functionality but did not, by itself, prove the firmware's binary identity.
2. The production-parser probe reproduced a repository source defect in `sendHello()`: it wrote
   `firmware_build` and `device_id` to the same payload offset. The observed HELLO began with
   `34 4F 4E 55 B8 0B 00 00`, so the expected build was overwritten by the device marker and the
   device-ID field was uninitialized. The corrected `p + 4` write and build `0x00010001` were
   compiled and uploaded as the controlled reference firmware.
3. Independent Rust-parser probe and normal production handshake then both passed on COM12:
   64 bytes, three valid frames, zero CRC failures, HELLO/CAPABILITIES/PONG true, build
   `0x00010001`, and device `0x554E4F34`.
4. During a separate idle recovery test, the Rust 1200-bps touch action rediscovered COM12 but
   received zero bytes after its bounded post-reset handshake. An exact .NET `SerialPort COM12,
   1200` open/close repeated the zero-byte outcome. No changed COM port or `2341:006d` bootloader
   port was observed. Re-uploading the already verified controlled binary restored the protocol
   identity immediately.

## Best-supported diagnosis

Two distinct conditions were observed. The initial silent application was explained by a different
analog test sketch. The post-IDE repository sketch then exposed a genuine HELLO payload-offset
defect, not a failed upload; the corrected controlled binary is independently verified. Separately,
the explicit 1200-bps recovery action can return the same COM port but leave the verified protocol
silent until an explicit controlled upload. The cause of that recovery failure (USB/bootloader
timing, control-line behavior, or a board-specific state) is unproven. It is not evidence that the
ESP32-S3 connectivity firmware is required or should be restored.

## Implemented recovery policy

`Reset board and retry` is visible only for an idle, discovered UNO after applicable protocol
failures. It closes the session, opens **that identified board only** at 1200 bps, closes it,
fast-polls Windows USB serial enumeration, matches by USB serial before port/VID/PID, ignores
unknown ports, waits for application settling, and performs a bounded normal handshake. It never
uploads firmware and never resets during recording. The result includes original/final port,
bootloader/disappearance/reappearance observations, frame statistics, failure category, and next
action. `Retry handshake` is separate and never touches the board.

## Acceptance status

Mock/selection and handshake retry tests pass. Normal application-path handshake and a 121-second
Until-stopped hardware capture pass after the corrected controlled binary is installed. A real
successful reset/re-enumeration could not be reproduced, so reset hardware acceptance remains an
open recovery defect. Physical unplug/reconnect was later verified separately; see
`logs/phase1_physical_disconnect_verification_2026-08-06.md`.

## Follow-up characterization — 2026-08-07

With the same independently verified controlled firmware on COM12, a production
`retry_handshake` first succeeded: 64 bytes, 3 valid frames, HELLO/CAPABILITIES/PONG true, zero
CRC failures, build `0x00010001`, device `0x554E4F34`, in 1288 ms. One subsequent idle
`reset_and_retry` touch reset closed the identified COM12 session, touched only COM12 at 1200 bps,
and returned the same identified application port without upload. It observed reappearance but no
disappearance or bootloader port, then failed after 12010 ms with zero bytes, zero valid frames,
three PING attempts, and `PortOpenNoBytes`. One separate no-reset retry remained silent after
8001 ms with the same zero-byte diagnostics.

This reproduces the reset recovery limitation without selecting COM3, without a recording active,
and without firmware upload. The data do not distinguish control-line behavior from a
board/bootloader state, so no timing/control-line change was made. Reset/retry remains a
documented nonblocking limitation; normal handshake passed before the touch reset.
