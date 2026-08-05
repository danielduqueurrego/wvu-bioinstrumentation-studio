# Phase 1 export validation — 2026-08-05

Validated outputs from `recordings/20260805_154043_Phase1_A0_Run01` by streaming the
binary and CSV files; the complete recording was not loaded into application memory.

| Check | Result |
|---|---|
| BMEG magic | `BMEGREC1` |
| BMEG header source | hardware (`simulator: false`) |
| BMEG sample records | 60,850 |
| CSV header | `sample_sequence,timestamp_us,elapsed_seconds,channel,adc_counts,volts,status_flags` |
| CSV data rows | 60,850 |
| Sidecar `total_samples` | 60,850 |
| Recording status | `complete` |
| Sample sequence | contiguous, 2,997 through 63,846 |
| Timestamp order | strictly increasing |
| Timestamp-derived rate | 1000.000 Hz |
| Volts conversion | all rows equal `adc_counts * 5.0 / 4095.0` within 1e-6 |
| Metadata integrity counters | match session counters: zero CRC, loss, duplicates, disorder, and overflow |
| Metadata tool provenance | CLI 1.5.2-rc.1; UNO R4 core 1.6.0 |

No corrupt or truncated record was found. Automated tests separately reject malformed
or truncated BMEG data and verify metadata deserialization and CSV export.
