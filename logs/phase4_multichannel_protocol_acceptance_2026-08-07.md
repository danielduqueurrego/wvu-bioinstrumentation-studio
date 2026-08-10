# Phase 4 multi-channel protocol acceptance — 2026-08-07

Scope: UNO R4 WiFi alone and deterministic simulator only. No person, electrodes, module, or
external bench source was used.

- Controlled protocol advanced from v0.1 to v0.2; major remains 0 and legacy v0.1 BMEG records
  remain readable.
- Controlled identity: build `0x00010002`, device `0x554E4F34`, six analog channels, 12/14-bit
  ADC, simultaneous-frame and fixed pulse-ox-cycle capabilities.
- USB CDC configuration was raised to 921600 baud after a reproduced 1000 Hz hardware run showed
  2,512 firmware overflow flags and 839.7 Hz. The corrected 30-second ECG capture measured
  1000.007 Hz with zero firmware/host overflow and zero sequence/CRC failures.
- Rust automated coverage includes fragmented/back-to-back multi-channel batches, invalid field
  counts, 1/2/3/4/6 channel profile mapping, pulse raw order, sequence integrity, legacy reader,
  and output-safe configuration.
- Accelerated production-path simulator soaks passed: six fields at 1000 frames/s for a
  10-minute equivalent (600,000 records) and eight raw pulse fields at 250 cycles/s for a
  10-minute equivalent (150,000 cycles). Both retained a bounded 1,500-record display history
  and reported zero CRC, sequence, firmware, or host-buffer errors.

Reference: `docs/USB_PROTOCOL_SPECIFICATION_v0.2.md`.
