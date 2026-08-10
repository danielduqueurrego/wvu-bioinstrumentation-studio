# Phase 4 export acceptance — 2026-08-07

The feature-gated `phase4_multichannel_capture` harness uses the production SessionController and streams its
final BMEG through `BmegReader` and `export_bmeg_csv`. It verifies frozen profile provenance,
monotonic contiguous logical sequence/timestamps, profile field count, exact CSV header, matching
BMEG/CSV/metadata record counts, requested-rate tolerance, and zero unexpected integrity counters.

Observed headers:

```text
record_sequence,t_us,ecg_counts
record_sequence,t_us,raw_emg_counts,rectified_emg_counts,emg_envelope_counts,pressure_counts
record_sequence,t_us,ppg_counts,mpxv_counts,xgzp_counts
cycle_index,t_us,red_TX,dark1_TX,ir_TX,dark2_TX,red_RX,dark1_RX,ir_RX,dark2_RX
```

The BMEG v0.2 record body retains `u32` record sequence, `u64` timestamp, status flags, and all
raw `u16` fields. Existing v0.1 single-channel BMEG input remains covered by reader tests and is
not relabelled as a Phase 4 course capture. Markers are metadata-only annotations and do not alter
the raw record stream.
