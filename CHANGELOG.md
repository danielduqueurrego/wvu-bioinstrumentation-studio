# Changelog

All notable user-facing changes are documented here.

## Unreleased

This section covers the `review-fixes` branch since 1.0.2, for maintainers. The application version is still 1.0.2. The reference firmware is unchanged (protocol `0.4`, build `0x00010006`, same source hash), so this change needs no firmware restore. **Hardware verification is pending.** The checklist is in [docs/AGENT_HANDOFF.md](docs/AGENT_HANDOFF.md#hardware-verification-checklist-for-review-fixes).

### Recording integrity

- Every recording is finalized, whatever ends it. STOP is always sent. A write or flush failure ends the capture as a fault instead of abandoning the file, and a failed free-space query is logged rather than fatal.
- Finalization follows a crash-safe order:
  1. Flush and fsync the BMEG.
  2. Rename it, with a bounded retry for OneDrive and antivirus.
  3. Write `.metadata.json` durably.
  4. Export the CSV as best effort.
  5. Rewrite the sidecar with `csv_filename` or `csv_error`.

  Flush, rename, and metadata problems are reported as `finalize_error` and logged as `FINALIZE_FAIL stage=…`. Raw data are never deleted.
- The raw `.bmeg.part` is created with `create_new`, run names skip any existing file or `.part` sibling, and final moves never replace another file.
- Labs without outputs (ECG, EMG, General Analog) no longer drop the batches in flight at Stop: the desktop waits for the STATUS that follows STOP.
- An idle-safe STOP is sent before CONFIGURE, so a board left acquiring by an earlier session is stopped first. Sample batches from an earlier session are ignored, its firmware errors are logged but not counted, and batches with the wrong field count are rejected.
- A save failure after Stop is now reported as a fault with its stage; only a cancelled session ends quietly.
- Elapsed and remaining time count from START, and `utc_start` is stamped just before START. Metadata adds `session_opened_utc` and `acquisition_started_utc`.
- Windows is kept awake while a recording runs.
- A final STOP whose write timed out, or that the board did not answer within 100 ms, is sent again (at most twice, within a bounded wait), so the outputs go LOW without waiting for the firmware's 5 s watchdog.
- The storage guard reserves room for the recording's CSV. The Recording duration card shows the estimate.
- Records of batches the firmware flagged after dropping a batch or skipping a sampling deadline are counted as `integrity.overrun_flagged_records`.
- The BMEG header size is checked before the board is configured.

### Board & firmware

- Start, Verify Firmware, Retry connection, Restore, and disconnect run off the main thread, so the window no longer freezes.
- One serial-ownership lease serializes recordings, Verify, Retry, Restore, and Repair Arduino tools.
- A hardware Start requires compatible firmware verified on the selected port (`verified_port`), not just a previous restore.
- Verify on a port that another program holds reports the busy port and keeps the earlier firmware result: a board not yet verified shows **Port busy**, and a board that was **Ready** keeps that chip while the failed Verify is reported as **Another program is using the board**.
- Restore changes:
  - Restore is refused while more than one UNO R4 WiFi is connected.
  - Cancel is refused once the upload starts.
  - The window cannot close during a restore.
  - Arduino CLI processes run in a kill-on-close Job Object with timeouts.
- "Reset board and retry" was retired, because it could not reach the native-USB firmware. Recovery is Retry connection, then Verify Firmware, then Restore WVU Firmware.
- A failed Arduino-tools preparation shows **Repair Arduino tools**. Runtime extraction uses a cross-process lock and checks the extracted files, and the generated `arduino-cli.yaml` quotes its paths.
- Firmware ERROR codes 1–7 get specific explanations. An unreadable CAPABILITIES payload fails the handshake.
- `fault_origin` (verification, connection, recording, storage, or request) keeps a failed Verify from being described as a failed recording, also right after a Simulator recording.
- A Retry connection or hardware Start that finds other firmware on the verified port withdraws **Ready**, so Start is not offered again until Verify or Restore succeeds.
- After a failed or canceled restore, the board that appears is selected but not verified, so the Board card keeps the restore's failure, log, and advice. Advanced details lists the last restore and its log.
- A failed **Repair Arduino tools** that left the working tools in place no longer marks them failed or disables **Restore WVU Firmware**.
- Closing the window during Verify or Retry connection closes it at once instead of announcing that a recording is being saved.
- Restore logs record the source commit (`git_revision`) when one is available. Tool versions are recorded as "unavailable" when they cannot be read.

### Labs & files

- Lab catalog:
  - Writes are atomic and serialized across processes.
  - Interrupted saves are recovered, and unreadable files are kept aside, never deleted.
  - Reset local customizations saves a backup first.
  - One invalid stored lab no longer hides the others.
- A new revision is numbered one patch above the highest existing revision, so editing after **Restore course default** works.
- Unchecking an output in the lab editor really turns it off (legacy `led_outputs` are normalized). Editing a pulse-ox lab no longer adds a phantom channel, and **Add channel** creates unique IDs. Pulse-ox packages exported by 1.0.2 still import.
- CSV names:
  - New and imported labs must use letters, digits, and underscores, unique across all generated columns.
  - Header cells are escaped and guarded against spreadsheet formulas.
  - A calibrated column that would repeat a name gets `_cal_`.
- Pulse-ox dwell and rate must agree, RED/IR can be set by an explicit role, and rates match the firmware. Text fields have length limits.
- A lab's allowed duration modes and minimum custom duration are enforced.
- Output folders reject Windows-reserved names and invalid characters. The default Documents folder is found through the Known Folder API, so OneDrive redirection works. A missing removable drive is reported, not recreated.
- Lab Manager:
  - Reset, Retire, and Discard confirmations wait for the instructor's answer, and a failed confirmation changes nothing.
  - Lab changes refresh the main window, and Start refuses a lab revision that changed after the page loaded it.
  - Lab Manager is unavailable while recording.
  - Exports are written atomically, and versions are canonical on save and import.
- Calibration:
  - One active preset per channel.
  - Bad presets are skipped and reported.
  - Degenerate or non-finite fits are refused.
  - A preset deleted in another window, or whose file was removed, leaves the list and its selection instead of being recorded; the list is read again when the window regains focus.
  - The XGZP fit window skips the first hardware sample by default and reports the Vref/Vs it used.
  - Manual points must be exactly two numbers per line.
  - Delete asks for confirmation.

### Interface

- Redesigned single window:
  - A sticky recording bar: state, SIMULATED tag, Source, Start, elapsed/remaining, frames, rate, marker label, Add marker, and Stop.
  - A readiness checklist.
  - One message center with a single next action.
  - Collapsible setup cards.
- Board panel: a state chip, Verify and Restore always reachable, a Last firmware operation card, and a restore stepper with a 90 s double-reset countdown.
- **Last recording** card: per-file ✓/✗, **Copy path**, **Show in folder**, and a data-quality line.
- Live signals: **Pause display**, time-window chips, labelled marker lines, stable channel colours, and a SIMULATED DATA watermark.
- Source switches to Hardware once, when firmware is first verified ready.
- Usability: real modal dialogs, visible focus rings, and plain-language messages. The default window is 1280 × 680.
- Fixed:
  - Changing the lab no longer loads the previous lab's calibrations.
  - A fault no longer locks setup, and the Simulator can start after a fault.
  - Verify handshakes no longer count as recordings (`recording_active`).
  - A Start that the Project folder's drive refused (for example less than 250 MiB free) says **The recording could not start** and why, instead of "The recording was not saved completely".
  - The Live signals **Lab outputs** line includes outputs that stored legacy labs define only through `led_outputs`.
  - Calibration inputs are validated.
  - Time-axis decimals follow the tick spacing, and newest-value badges are placed correctly on scaled displays.
  - Idle polling is throttled.

### Security & dependencies

- The CSP no longer allows `asset:` images. Dialog permissions are limited to open, save, and message. `list_boards` ignores a caller-supplied CLI path in release builds. **Show in folder** is restricted to the Project folder.
- Semver-compatible updates fix advisories in development tooling and in the Rust `rustix` and `tokio` crates; production npm dependencies have no known advisories. The remaining `zip` 2.x advisories and some development-tool advisories need semver-major upgrades and are deferred.
- A weekly audit workflow (`npm audit`, `cargo audit`) and Dependabot for npm, Cargo, and GitHub Actions.

### Developer tooling

- Rust is pinned to 1.100.0 in `rust-toolchain.toml`, and Cargo runs with `--locked` everywhere. CI actions are pinned by SHA, with a Rust cache and a job timeout.
- `scripts/check_release_consistency.ps1` checks versions, firmware identity and source hash, the CHANGELOG heading, and the Tauri npm/crate pairs. CI and the release script both run it.
- CI runs the Op Amp Lab installer under Windows PowerShell 5.1, PowerShell 7, and the `.bat`.
- `scripts/package_op_amp_lab_installer.ps1` rebuilds the installer ZIP reproducibly, and `.bat`/`.cmd`/`.ps1` files use CRLF.
- The release script requires PowerShell 7, checks every exit code, and uses `npm ci` and `--locked` builds.
- `scripts/start_dev_sandbox.ps1` runs a debug build against disposable app data, optionally with WebView2 remote debugging.
- `build.rs` embeds the source commit.
- Unused mobile icons and crate types were removed. `package.json` is private and requires Node 22 or later.
- The documentation was updated to the current behavior.

## 1.0.2

### Firmware

- The reference firmware moved to protocol `0.4`, build `0x00010006`. Release 1.0.1 used protocol `0.3`, build `0x00010003`. Protocol 0.4 adds measured per-record timestamps, a cumulative queue-overflow count, and per-state pulse-ox timing checks. The factory labs now require this build, so use **Restore WVU Firmware** once on every board after upgrading.

### Fixed

- Pulse-ox RED, DARK 1, IR, and DARK 2 measurements are now taken only after the configured phase-settling interval has elapsed.
- Decoupled pulse-ox and simultaneous-analog sampling from packet construction and USB writes with bounded raw-batch and nonblocking transmit queues plus explicit overflow reporting.
- Changed pulse-ox phase timing to one absolute schedule and added per-state ±5% timing diagnostics to recording metadata.
- Preserved firmware-measured timestamps for every acquired record instead of reconstructing uniformly spaced timestamps on the host.
- Added final recording timing diagnostics and profile-aware recording notes; profile safety provenance and medical-validation status remain explicit without an operator connection checkbox.
- Recording filenames now use the selected lab's stable export name and no longer contain development-phase labels.

## 1.0.1

### Fixed

- Stabilized long-running `Until stopped` acquisition on Arduino UNO R4 WiFi and retained precise serial/no-data diagnostics for genuine failures.
- Removed the initial ADC settling transient from the live display without changing authoritative BMEG/CSV samples.
- Changed live plot time labels to elapsed recording seconds starting at zero.
- Ensured startup failures are written to the application log and shown in a native Windows error dialog instead of appearing as a brief console flash.
- Made instructor lab-catalog writes transactional and allowed an explicit factory reset to recover from a malformed local catalog.

### Maintenance

- Added atomic, recoverable Arduino-runtime deployment and bounded application/catalog/firmware logs.
- Strengthened release validation, CI coverage, firmware/runtime hash checks, and bundled third-party notices.

## 1.0.0

### Added

- Synchronized ECG, EMG + pressure, blood pressure + PPG, and raw pulse-ox course acquisition.
- Project/trial folders, recording markers, and automatic BMEG/CSV/metadata output.
- Counts, volts, MPXV pressure conversion in kPa or mmHg (including EMG + Force pressure), and local linear-calibration presets.
- Instructor course-lab management with versioned revisions and factory defaults.
- Bundled Arduino tools, firmware verification, and WVU reference-firmware restoration.
- Safe live plot reconfiguration, configurable plot groups, and per-trace visibility during recording.
- Color-coded legends for multi-signal plots, a synchronized 0.5–30 s live plot window, and newest-value endpoint labels.
- Elapsed-seconds live plot axes starting at the first accepted recording frame, with the initial hardware ADC settling transient excluded from the display only; raw BMEG/CSV data remain unchanged.

### Known limitations

- The application is teaching software and does not provide physiological or clinical interpretation.
- Windows installers remain unsigned unless an approved institutional signing workflow is supplied.
