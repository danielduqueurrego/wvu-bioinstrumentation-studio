# Phase 5 export-unit acceptance

Status: automated round-trip coverage and UNO raw-export smoke passed; release-app calibrated
column selection remains pending.

CSV retains every raw count column, then adds direct `_V` columns using the frozen Vref. When an
active MPXV formula is captured, it additionally writes `mpxv_kPa`/`mpxv_mmHg` or `pressure_kPa`.
`xgzp_mmHg` is emitted only when a frozen compatible saved linear calibration is active. BMEG
remains raw-count authoritative and older no-calibration metadata deserializes safely.

The UNO-only ECG, EMG, BP, and pulse-ox export harnesses completed with matching raw BMEG/CSV
record counts and profile headers. The BP CSV raw + voltage header path is additionally covered by
the automated metadata/CSV round-trip test. UI selection of calibrated columns remains pending.
