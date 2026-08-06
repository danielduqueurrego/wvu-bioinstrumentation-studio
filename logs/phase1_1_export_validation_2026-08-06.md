# Phase 1.1 export validation — 2026-08-06

The production acceptance harness streamed the finalized hardware BMEG and CSV; it did not load
the recording into a complete in-memory collection. It independently deserialized the metadata
sidecar and checked every BMEG sample and CSV data row.

```powershell
src-tauri\target\debug\phase1_capture.exe validate recordings\20260806_121524_Phase1_A0_Run01.bmeg
```

| Check | Result |
|---|---|
| BMEG read-back | Passed |
| Metadata JSON deserialization | Passed |
| BMEG / CSV records | 121,120 / 121,120 |
| Sample sequences | Contiguous 0 through 121,119 |
| Board timestamps | Strictly monotonic, 277,521,586 through 398,640,586 µs |
| Rate from first/last timestamp | 1000.000 Hz |
| Voltage conversion | Passed for every CSV row: `counts * 5.0 / 4095.0` (six-decimal CSV tolerance) |
| Duration / requested duration | `until_stopped` / absent |
| Stop reason / completion | `user` / `complete` |
| Metadata total samples | 121,120 |
| Metadata integrity counters | Match host summary; all zero except 12,480 valid packets |

The real recording files remain Git-ignored:

- `recordings/20260806_121524_Phase1_A0_Run01.bmeg` — 1,697,025 bytes
- `recordings/20260806_121524_Phase1_A0_Run01.metadata.json` — 1,684 bytes
- `recordings/20260806_121524_Phase1_A0_Run01.csv` — 5,350,504 bytes
