# Phase 3A profile export validation — 2026-08-06

The `phase3a_profile_capture` production-controller harness streamed each BMEG, read the sidecar,
and counted CSV rows without holding a complete recording in memory.

| Profile | BMEG / CSV rows | Snapshot / acknowledgement | Sequence/timestamp/integrity | File sizes BMEG / JSON / CSV |
| --- | ---: | --- | --- | --- |
| General A0 development | 10,000 / 10,000 | General 1.0.0 / true | contiguous/monotonic; zero failures | 142,637 / 3,561 / 1,066,280 bytes |
| ECG raw output | 29,930 / 29,930 | ECG 1.0.0 / true | contiguous/monotonic; zero failures | 421,765 / 3,672 / 2,893,344 bytes |
| EMG raw output | 29,930 / 29,930 | EMG 1.0.0 / true | contiguous/monotonic; zero failures | 421,762 / 3,668 / 2,893,339 bytes |

Each BMEG JSON header and sidecar carries the immutable profile snapshot. Profile-aware CSV has
the existing raw columns plus `profile_id`, `profile_version`, and `signal_label`; its volts remain
the direct `counts * 5.0 / 4095.0` conversion. Automated tests also prove BMEG/metadata without
profile data remains readable as legacy/general-development data.
