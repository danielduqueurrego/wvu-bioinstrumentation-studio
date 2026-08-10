# Phase 4 multi-channel acquisition plan

## Scope

Implement synchronized UNO R4 WiFi course acquisition for ECG, EMG + force, blood pressure +
PPG, and raw TX/RX pulse oximetry. The work remains teaching use only; it adds no clinical
interpretation or automatic physiological calculation.

## Implementation steps

1. Version the USB protocol at minor `0.2`, add explicit simultaneous-frame and fixed
   pulse-ox-cycle configuration/record layouts, and retain v0.1 parser/read compatibility.
2. Replace the firmware's single-channel sampler with deterministic configured-channel frame
   reads and the fixed RED/DARK/IR/DARK A0/A1 cycle. Keep D4/D5/D6 LOW except where a configured
   active acquisition explicitly needs D4, D5, or D6.
3. Introduce synchronized multi-field records in Rust recording/session code, migrate metadata
   and CSV export, keep `BMEGREC1` legacy files readable, and record markers separately in
   metadata.
4. Refactor acquisition profiles into mode-specific channel/pin maps, add locked course profiles,
   and keep draft remapping limited to instructor authoring.
5. Extend the simulator, bounded display buffer, plot, and Acquisition UI for trace controls,
   pulse-ox preview, markers, and responsive layouts.
6. Add protocol/profile/export/session/UI regression tests; complete accelerated simulator soaks
   and UNO-alone smoke captures before documenting results and committing the feature branch.

## Risks and controls

- UNO analog reads are sequential within one logical frame, not physically simultaneous. This
  ordering will be stated in protocol/profile documentation and one shared timestamp will identify
  the frame.
- Firmware wire compatibility changes at protocol minor 0.2. The host continues to read existing
  v0.1 BMEG files but v0.2 hardware requires the controlled Phase 4 firmware identity.
- LED outputs are active HIGH. The firmware's safe-output routine must run on startup, stop,
  malformed configuration, protocol error, watchdog timeout, and disconnect timeout.
- Full-rate raw data stay streaming to BMEG; display data remain bounded and delivered in batches.

## Follow-up: plotting, discovery, and startup verification

1. Replace the self-resetting trace-ID array with one profile-scoped visibility map. Derive both
   checkbox state and uPlot series from that map, and rebuild it only when the active plot profile
   changes.
2. Add an Overlay/Stacked display choice. Both layouts consume the same bounded session snapshot;
   stacked plots receive one signal each so their y axes scale independently.
3. Move board discovery to a root-level cache. Scan once after the shell renders and only on an
   explicit Refresh boards action or a specific reconnect/upload workflow—never because a view
   mounts or navigation changes.
4. Add an application-level indeterminate operation modal for discovery, verification, and
   firmware workflow stages. It reports real stages but no invented percentage.
5. When the startup scan finds exactly one supported board, select it and perform a bounded,
   non-mutating firmware identity handshake. Reuse the result across all pages.
6. Add focused pure-UI state tests, rebuild the release application, then repeat the Phase 4
   simulator and manual acceptance checks before committing.
