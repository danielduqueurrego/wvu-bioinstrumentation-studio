# Phase 6 pulse-ox authoring acceptance — 2026-08-10

Status: **automated configuration and accelerated-soak coverage complete; simulator UI and hardware smoke pending.**

- The only authorable pulse acquisition mode is fixed `RED ON → DARK 1 → IR ON → DARK 2`.
- Instructor settings are restricted to distinct TX/RX A0–A5, distinct RED/IR D4–D6, 12/14-bit
  ADC, labels/default plots, and 250–5000 µs dwell.
- Cycle rate is derived from dwell (`1 / (4 × dwell)`) for provenance/display; raw timestamps are
  authoritative.
- Protocol v0.3 encodes the configured resources and retains all eight raw fields. RED/IR mutual
  exclusion is checked by profile validation and enforced by firmware safe-output handling.
- The automated pulse-ox soak covers 150,000 raw cycles (an accelerated 10-minute equivalent at
  250 cycles/s) with eight stored raw fields/cycle and bounded display history. A remapped TX=A2,
  RX=A3, RED=D4, IR=D6 configuration is also encoded and checked by the host test suite.

Pending: simulator and UNO v0.3 captures with non-default mappings/dwell, raw field/header
read-back, LED LOW after Stop, and manual UI inspection.
