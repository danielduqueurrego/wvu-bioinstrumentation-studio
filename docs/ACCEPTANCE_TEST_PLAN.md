# Acceptance test plan

## Phase 1 required acceptance tests

### Environment

- Arduino CLI version is captured.
- UNO R4 core version is captured.
- Rust, Cargo, Node, npm, Git, and Windows versions are captured.
- The board is detected with its COM port and FQBN.

### Build and upload

- Reference firmware compiles for `arduino:renesas_uno:unor4wifi`.
- Upload succeeds to the connected board.
- Firmware identity is read back through the protocol.
- Build and upload logs are saved.

### Protocol

- Valid frames decode.
- Partial frames decode after more bytes arrive.
- Back-to-back frames decode.
- Noise before magic is skipped.
- Invalid length is rejected.
- CRC error increments a counter.
- Packet gaps, duplicates, and out-of-order packets are detected.
- Parser fuzz/property tests do not panic.

### Acquisition

- One analog channel streams at 1000 samples/s.
- Start and stop commands work repeatedly.
- Plot updates are batched rather than sample-by-sample.
- UI remains responsive.
- Memory does not grow without bound.
- Disconnect/reconnect is handled without crashing.
- A simulator can exercise the same host code without hardware.

### Recording

- A 60-second hardware or simulator recording is created.
- `.bmeg`, `.metadata.json`, and `.csv` are readable.
- Sample sequence and timestamp columns are present.
- Integrity counters are saved.
- Filename sanitization tests pass.

### Branding and safety

- Approved logo is used without alteration.
- UI uses the specified WVU digital colors.
- The teaching/not-medical-device notice is visible in About/Help.
- No human-subject test is performed.
- Pulse-ox LED outputs are low in any firmware created during Phase 1.

## Later engineering tests

- ECG: 1000 samples/s for 10 minutes.
- EMG: 2000 samples/s for 10 minutes.
- Aggregate: four channels at 2000 samples/s.
- One-hour wired stress test.
- Pulse-ox 100 Hz frame acquisition with no optical-frame loss.
- MPXV calibration and unit conversion verification.
- Offline Windows installation.
- Low-resource acceptance machine testing.

## Preliminary performance targets

- Cold start <= 5 seconds on the low-resource test machine.
- Idle memory < 175 MB.
- Typical acquisition CPU target < 20%.
- Display refresh 20–30 Hz.
- Zero sample loss in the normal 10-minute wired test.
- Zero sample loss in the one-hour engineering stress test.
