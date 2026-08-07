# Physical USB disconnect/reconnect verification — conducted 2026-08-07

## Scope and safety

UNO R4 WiFi only, running the controlled WVU reference firmware. A0 was floating and explicitly
labeled uncalibrated. No person, electrode system, ECG/EMG module, biomedical accessory, or
external bench source was connected.

## Result: passed

Board identity: Arduino UNO R4 WiFi; FQBN `arduino:renesas_uno:unor4wifi`; USB serial
`48CA4360243C`; application port `COM12`; protocol 0.1; firmware build `0x00010001`; device
`0x554E4F34`.

1. A General A0 — Development, 12-bit, 1000 samples/s, Until-stopped recording was started.
2. The user unplugged the USB cable while it was active. The application remained responsive and
   transitioned to **Faulted** with `device_disconnected`; it reported no reconnect and one
   disconnect. Its exact serial error was `The device does not recognize the command. (os error 22)`.
3. The recording finalized as **incomplete** with stop reason **disconnect**. No samples were
   fabricated after the cable removal.
4. The user reconnected the board. Refresh rediscovered the same COM12 port; explicit new start
   was required. A fresh protocol handshake passed in 1287 ms (64 bytes, 3 valid frames,
   HELLO/CAPABILITIES/PONG all true, zero CRC failures) and a distinct 10-second timed session
   completed normally.

## Raw-file read-back

Both files were streamed and checked outside the UI: BMEG magic/record boundaries, contiguous
wrapping sample sequence, strictly monotonic timestamps, CSV row count, direct
`counts * 5.0 / 4095.0` conversion, and metadata sample count/finalization fields.

| Session | BMEG / CSV / metadata | BMEG samples / CSV rows | Host / board elapsed | Packets | Integrity | Finalization |
| --- | --- | ---: | --- | ---: | --- | --- |
| Disconnect | `20260807_142411_Phase1_A0_Run01` | 30,690 / 30,690 | 30.806 s / 30.689 s | 3,164 | all CRC, sequence, duplicate, overflow, and reconnect counters 0; disconnects 1 | incomplete / disconnect |
| Explicit post-reconnect session | `20260807_142655_Phase1_A0_Run01` | 9,970 / 9,970 | 10.016 s / 9.969 s | 1,029 | all CRC, sequence, duplicate, overflow, disconnect, and reconnect counters 0 | complete / timed_complete |

The real files are intentionally Git-ignored in `recordings/`; their sizes were 432,448 / 3,417,257
/ 3,716 bytes for the disconnected BMEG/CSV/metadata triplet and 142,358 / 1,065,811 / 3,705
bytes for the post-reconnect triplet.

## Display-duration correction

The pre-fix fault screen showed 1:14 while final metadata showed 30.806 seconds of host capture.
The raw data were correct; the status UI continued measuring from its initial session time after
finalization. `SessionController` now reports the finalized worker duration once a session is
Faulted or Disconnected. Automated regression
`finalized_status_keeps_capture_elapsed_time_stable` covers this correction.

Detection latency from physical unplug to the reported OS error was not independently timed, so it
is not claimed here. The file-finalization and explicit-new-session behavior were directly
verified.
