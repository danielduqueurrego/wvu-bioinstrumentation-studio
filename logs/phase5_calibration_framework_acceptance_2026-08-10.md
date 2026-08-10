# Phase 5 calibration framework acceptance

Status: implementation and deterministic simulator/unit verification passed; hardware and manual
UI evidence pending.

## Boundary

Raw ADC counts remain authoritative BMEG data. Calibration changes live display and streamed CSV
derivations only. The implementation adds no ECG interpretation, EMG MVC/fatigue/force, SBP/DBP,
SpO2, ratio-of-ratios, perfusion index, or physiological conclusion.

## Implemented framework

- Stored Vref-aware counts-to-volts conversion for 12- and 14-bit records.
- MPXV kPa/mmHg equation with stored sensor-supply voltage; values are not silently clamped.
- Local, profile/channel-scoped calibration presets with save/load/delete.
- Ordinary least squares fit for manual points and synchronized BP MPXV/XGZP recording intervals.
- Frozen `RecordingMetadata.calibration` snapshots holding Vref, Vs, channel units, and active
  preset parameters.
- Streaming CSV columns beside raw counts. Existing BMEG records stay raw and legacy files remain
  readable under their documented 5.0 V assumption.

## Simulator production-path result

`phase4_multichannel_capture simulator bp 10` completed on 2026-08-10 through the common
controller/parser/writer path: 2,000 BP frames at 200.000 Hz, 215 valid packets, no CRC,
sequence, overflow, disconnect, or reconnect errors. D4 was HIGH during capture and LOW after
finalization. The temporary raw outputs were intentionally not retained in the repository.

## Pending acceptance

- User-assisted Acquisition UI matrix at 100% scaling.
- Release-app Calibration & Units interaction check after the rebuild.
- Complete Phase 5 build/package verification and commit only after those checks pass.

## UNO-only smoke result

Controlled firmware on Arduino UNO R4 WiFi `COM12`, serial `48CA4360243C`, passed the
production-parser identity probe (protocol 0.2; build `0x00010002`; device `0x554E4F34`; zero
CRC/invalid frames). With floating/unconnected analog inputs and no biomedical accessory:

| Profile | Records | Rate | Packets | Integrity | Digital result |
| --- | ---: | ---: | ---: | --- | --- |
| ECG | 9,980 | 1000.025 Hz | 1,030 | zero unexpected counters | D4/D5/D6 LOW |
| EMG + Force | 9,980 × 4 fields | 999.962 Hz | 1,030 | zero unexpected counters | D4/D5/D6 LOW |
| BP + PPG | 1,990 × 3 fields | 200.005 Hz | 232 | zero unexpected counters | D4 HIGH while acquiring, LOW final |
| Pulse ox raw | 2,490 × 8 fields | 249.965 Hz | 281 | zero unexpected counters | final D4/D5/D6 LOW |

All harness outputs were written beneath the ignored system temporary directory and not retained
in Git.
