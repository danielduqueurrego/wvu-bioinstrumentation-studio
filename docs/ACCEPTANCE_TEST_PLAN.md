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
- A user-selected Until stopped recording remains active until manual Stop, disconnect, fault, or
  storage guard; it must not use a hidden duration limit.
- Long recording safeguards warn below 1 GiB free and finalize safely below 250 MiB free.
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

## Phase 2 required acceptance tests

### Project and editor

- Create each controlled template into a matching one-INO project folder.
- Open, save, Save As, restore saved source, and reject invalid/path-traversal names.
- Preserve UTF-8 source; show unsaved state and prevent compile/upload until the source is saved.
- Verify CodeMirror find/replace, undo/redo, bracket matching, line navigation, and keyboard save
  shortcuts without hidden source transformation.

### Compile and upload

- Report missing Arduino CLI/core with exact remediation while keeping editing usable.
- Compile a valid controlled reference and A0 example; capture size, RAM, warnings/errors, and
  parsable source locations.
- Reject upload while acquisition is active, while the project is unsaved, without confirmation,
  or without a current matching compile artifact.
- Upload an A0 ASCII/non-WVU sketch, report upload success, and disable Acquisition without
  describing the board as failed.
- Restore the controlled reference through the application workflow and independently require
  HELLO, CAPABILITIES, PONG, protocol v0.1, build `0x00010001`, device `0x554E4F34`, and zero
  CRC failures before Acquisition is enabled.
- Exercise same-port, changed-port, delayed-port, no-return, ambiguous-return, and unrelated-port
  handling with deterministic tests. Record the real observed reset/re-enumeration path.

### Firmware/acquisition coordination

- After verified reference restore, complete a 30-second A0-only, 12-bit, 1000 samples/s
  recording and validate BMEG/CSV/metadata counts, monotonicity, conversion, and integrity
  counters.
- Verify the Firmware view at the documented desktop viewport/scaling matrix before marking the
  interactive UI portion of Phase 2 accepted.

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
