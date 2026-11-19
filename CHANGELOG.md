# Changelog

All notable user-facing changes are documented here.

## Unreleased

This section covers the `review-fixes` and `release-1.1-prep` branches since 1.0.2, for maintainers. The application version is still 1.0.2. The reference firmware is now **build `0x00010007`** (protocol still `0.4`), so **every board needs one Restore WVU Firmware**, and **every lab computer must be updated before any board is restored** (a 1.0.x computer would put build `0x00010006` back). **Hardware verification is pending.** The checklist is in [docs/AGENT_HANDOFF.md](docs/AGENT_HANDOFF.md#hardware-verification-checklist-for-review-fixes).

### Firmware build 0x00010007 and LED-sequenced pulse oximetry

- **New acquisition mode `pulseox_sequenced`** (CONFIGURE mode 2, CAPABILITIES mode bit `0x04`). The LEDs step RED, DARK 1, IR, DARK 2 as before; at the end of every state the board reads 1–6 inputs of A0–A5 while that state's LEDs are unchanged and records one row per state: `[led_state, ch1..chN]`. Dwell is 1000–5000 µs, so the row rate is `round(1,000,000 / dwell)` (1000 rows/s at 1 ms) and each LED state gets a quarter of the rows. A pulse-ox photodetector can now be recorded together with other analog signals in one lab.
- **The built-in pulse-ox lab is version 1.1.0 in the new mode**: TX on A0 (`tx_counts`) and RX on A1 (`rx_counts`), both shown by LED state, D5 red, D6 IR, 1000 µs, 1000 rows/s, firmware build `0x00010007`. Same lab ID, notices (raw only, no SpO2), and export name. Version 1.0.0 (legacy 4-state) is kept in `profiles/legacy/` and can be installed with **Install lab file…** as a rollback. The other five factory labs and their hashes are unchanged (they keep firmware build `0x00010006`).
- **Sequenced CSV:** `record_sequence,t_us,cycle_index,led_state,led_state_name,` then each channel's counts, `_V`, and calibration columns, as in simultaneous labs. `led_state` is 0–3, `led_state_name` is `RED`/`DARK1`/`IR`/`DARK2` (`UNKNOWN` for any other code), and `cycle_index = record_sequence / 4`. `led_state` and `led_state_name` are reserved CSV names for new labs. The BMEG header and the sidecar record `led_sequenced_layout`. The legacy 4-state CSV and record layout are unchanged.
- **The legacy 4-state mode stays fully supported** in firmware and app, for existing local labs and old recordings. Lab Manager offers it only when editing a lab that already uses it; new pulse-ox labs use the LED-sequenced mode.
- **Lab Manager:** a shared channel grid (`ChannelGridEditor`) for simultaneous and LED-sequenced labs, with a **By LED state** column (up to 6 channels), RED and IR selects, the dwell, and a rows-per-second estimate. LED-sequenced labs are checked as in Rust: 1–6 raw-count channels with unique pins, IDs and CSV names, distinct RED/IR outputs on D4–D6 and no others, build `0x00010007` or later, and plot defaults, if any, that put every channel in exactly one plot.
- **Live signals:** a channel shown by LED state is drawn as four traces ("TX — RED", …), each blank outside its state; other channels are one trace at the full row rate. **Show by LED state** switches this per channel during a recording; it is display only. Units and calibration follow the lab channel, the startup-transient filter works per LED state, and display decimation keeps whole four-state cycles so a plot never shows only one LED state. Lab `plot_defaults` are now honoured in every mode.
- **Units are decided per signal:** the eight raw fields of a 4-state lab offer counts and volts; every other channel follows its lab's `allowed_conversions`, also in pulse-ox labs.
- **Firmware timing and safety:**
  - A minimum-settle guard in both pulse-ox modes: a late loop lengthens the state that just started so its sample is at least half a dwell after its LED edge; states are never skipped or reordered.
  - Skipped sampling deadlines get their own sample flag `0x0010` and a count in STATUS; `0x0004` now means only a dropped batch. STATUS grows to 46 bytes with the skipped-deadline count and the minimum observed settle.
  - A CONFIGURE received during acquisition stops it first, like STOP. Each queued batch keeps its own period and field count.
  - Simultaneous mode refuses any D5 or D6 output bit.
  - A table-driven CRC, batch encoding that waits while a sample is due within 120 µs, at most 64 command bytes per loop pass, and a hardware watchdog of about 1.4 s (the sketch asks for 1000 ms; the core rounds it up to the next hardware timeout).
  - Flash about 60.1 kB (22%) and RAM 12,344 bytes (37%) with runtime revision 4. The exact flash count varies by a few bytes with the build folder (60,080–60,088 bytes so far), because the binary embeds source paths.
- **Recording metadata:** `timing_diagnostics.firmware_skipped_deadlines` and `minimum_pulse_settle_us`; `integrity.deadline_skip_flagged_records` and `integrity.led_state_mismatches` (an LED-sequenced record whose state is not `record_sequence % 4`). All additive. Advanced details shows the last recording's skipped deadlines, minimum settle, and per-state timing, and the Last recording quality line counts skipped deadlines and LED-state mismatches.
- **Older and newer WVU firmware:** a board with another build of the WVU firmware is reported as **Older WVU firmware** ("Restore WVU Firmware") or **Newer WVU firmware** (this computer's app is older than the board's firmware: update the app, or restore), instead of a generic incompatible-firmware message. Both withdraw Start readiness, and Retry connection is not offered for them. Lab files may name any WVU build from `0x00010001` to `0x00010007`.
- The Simulator supports the new mode (state-dependent values, with a ~1.2 Hz pulse on RED and IR in pulse-ox labs) and reports build `0x00010007`.

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
- Records of batches the firmware flagged after dropping a batch (and, up to build `0x00010006`, after skipping a sampling deadline) are counted as `integrity.overrun_flagged_records`.
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

### Arduino tools (runtime revision 4)

- The bundled offline Arduino runtime is **revision 4**: Arduino CLI **1.6.0**, the stable release, instead of the pre-release 1.5.2-rc.1, with the same Arduino UNO R4 core 1.6.0 (SHA-256 `5baa8a5f…`). OpenOCD and dfu-util were removed: the UNO R4 WiFi uploads with bossac and never used them. The first start after the update extracts the new runtime once. In the offline smoke test it compiled firmware build `0x00010006` unchanged (flash 58,644 bytes, RAM 11,712 bytes) and later build `0x00010007` (flash about 60.1 kB, RAM 12,344 bytes).
- The generated `arduino-cli.yaml` turns off Arduino CLI's update notification and its Arduino Cloud lookup of detected boards.
- **Long Windows profile folders.** The bundled compiler fails when the runtime folder has 60 or more characters, which `%LOCALAPPDATA%\WVU-BMEG\rt` reaches with profile folder names longer than 24 characters (1.0.x is affected too). The app then keeps the runtime in `%ProgramData%\WVU-BMEG-<id>\rt`, a folder only that Windows account (with SYSTEM and Administrators) can open; an existing folder of that name is used only if it is a plain folder owned by that account (or SYSTEM or Administrators) whose protected access list allows no other account. If that is not possible, the Board card says **The Arduino tools cannot be set up for this Windows account** and points to IT support (new category `runtime_path_too_long`) instead of a compiler error. Known limitation: a profile folder name with characters outside the Windows "language for non-Unicode programs" (for example Chinese characters on an English computer) still makes the compiler fail ([Troubleshooting](docs/TROUBLESHOOTING.md#arduino-tools-cannot-be-set-up-for-this-windows-account)).
- The archive root carries Arduino CLI's own GPLv3 `LICENSE.txt` as `arduino-cli-LICENSE.txt` and the GNU GPL-2.0 and LGPL-2.1 texts. `BUNDLED_COMPONENTS.txt` now lists every bundled component with its version, license, path and upstream source.
- **Corresponding source.** Each release publishes `WVU-Bioinstrumentation-Studio-<version>-third-party-sources.zip` beside its installers: the source of every GPL and LGPL tool in the runtime (with vendored Go modules and the toolchain's statically linked libraries), build directions and a written offer. `release/third-party-sources-runtime3.json` describes the bundle for 1.0.0 to 1.0.2; it says plainly that the exact OpenOCD and dfu-util build sources are held by Arduino and that requests are forwarded to Arduino. Attaching it to those releases is left to the maintainer.
- Maintainer tools: `scripts/build_arduino_runtime.ps1` rebuilds the runtime with offline compile proofs and pinned tool versions; `scripts/build_third_party_sources.ps1` builds the source bundles and, with `-VerifyBundle`, checks a finished one offline; `scripts/check_third_party_sources.ps1` validates their manifests without downloading (CI runs it); `release/arduino-runtime-entries.json` is the one list of required runtime entries, license hashes and removed tools for the release script and the notice audit; `src-tauri/examples/runtime_smoke.rs` compiles the reference sketch offline with a runtime extracted by the application's code. The release script builds the source bundle, or verifies a given one in full (`SHA256SUMS.txt`, every pinned component and vendor tree), and stages it in `dist\`.
- Unit tests no longer write to the developer's own `application.log`.

### Labs & files

- Lab catalog:
  - Writes are atomic and serialized across processes.
  - Interrupted saves are recovered, and unreadable files are kept aside, never deleted.
  - Reset local customizations saves a backup first.
  - One invalid stored lab no longer hides the others.
  - A stored lab with the same ID and version as a course default but different content is no longer erased at the next save: it stays in the catalog file and the Lab panel notice names the course default it conflicts with. An identical copy is dropped quietly.
- A new revision is numbered one patch above the highest existing revision, so editing after **Restore course default** works.
- Factory lab versions follow policy P2: always `MAJOR.MINOR.0`, a content change bumps MINOR, and edited local revisions always have PATCH ≥ 1. A new lab ID (saved locally as 1.0.x) becomes factory version 1.1.0, so a factory version never has the version of a local revision. `profiles/factory-labs.lock.json` records every factory lab version shipped, and each one stays available as a file (`profiles/legacy/` for earlier versions).
- **Op Amp Lab is built in** (`wvu.bmeg420l.op_amp.course.capture.v1` 1.0.0, category `course_op_amp`): Vmid on A0 (`vmid_counts`) and Vout on A1 (`vout_counts`), 100 frames/s, 14-bit, bench-only, one plot per signal. It has its own lab ID, so General Analog stays in the Lab list. The Simulator gives it a steady Vmid and a Vout that swings around it. The other factory labs and their hashes are unchanged, apart from the pulse-ox lab's move to 1.1.0 (above).
- One-time startup migration `op_amp_builtin_v1`: a catalog whose active General Analog revision is the Op Amp Lab 2.0.1 copy from the v1.0.2 student installer (hash `c73cc1f2…`) or the same lab saved as an instructor revision (`bcc08868…`) is first copied to `lab_state.pre-migration-<time>.json`; that revision is then retired and General Analog's course default is active again. The Lab card says so, with the backup path, in Student and Instructor mode, and the log records `LAB_CATALOG_MIGRATED`. The catalog records it in a new, additive `applied_migrations` field (once it has run, or when that revision is already inactive or retired; never while it is still selected and not migrated), so it runs once and a later deliberate reactivation is kept. If the catalog cannot be written, the migration is retried at the next start with the same backup.
- **Install lab file…** in the Lab card (Student and Instructor mode, disabled while recording) installs a `.lab.json` file from the instructor. It first shows the lab and asks; when the file replaces a lab, including a course default, the question is a warning that the instructor can undo it with **Restore course default**. It uses Import lab's validation and conflict rules, stores the lab as `imported`, makes it active and selects it, and records `student_install` in `lab_write_audit.log`. A version that is installed but not active is made active again. Rust refuses it while a recording runs, while the catalog is unavailable, for a retired version, for a course default file, for a lab that breaks the rules for a lab saved now (reported before the student confirms), and for the older Op Amp Lab file. New commands: `inspect_lab_file { source }` (read-only) and `install_lab_file { source }`.
- **Save as course default file…** in Lab Manager (Instructor mode, local revisions) writes a factory lab file for the repository's `profiles/` folder, with the same normalization and next `MAJOR.MINOR.0` version as `scripts/update_factory_labs.ps1 promote`, and lists the next steps (copy, `check`/`fix`, the `factory_lab_file!` line for a new lab, rebuild). The file is for the repository, not for students (Install lab file refuses it; Export is what students install). The catalog is not changed; `lab_write_audit.log` records `course_default_export`. New command: `export_course_default_file { profileId, profileVersion, destination }`. `list_instructor_labs` entries gain an additive `course_default_file_name`.
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
- `.bat`/`.cmd`/`.ps1` files are checked out with CRLF.
- The v1.0.2 student Op Amp Lab installer (`scripts/install_op_amp_lab_profile.bat`, `.ps1` and `.zip`), `scripts/package_op_amp_lab_installer.ps1`, and their CI steps were removed: the Op Amp Lab is built in. The installer's embedded lab is kept byte for byte as `src-tauri/src/testdata/op_amp_2.0.1_imported.lab.json` for the hash and migration tests.
- The release script requires PowerShell 7.2, checks every exit code, and uses `npm ci` and `--locked` builds.
- `scripts/start_dev_sandbox.ps1` runs a debug build against disposable app data, optionally with WebView2 remote debugging.
- `scripts/update_factory_labs.ps1` (`check`, `fix`, `promote`) changes a built-in lab safely: it re-hashes a hand-edited factory lab file, publishes a content change only as a new MINOR version, turns a Lab Manager export into a factory lab file, and keeps the lock append-only. Tests require every `profiles/*.profile.json` to be compiled in and recorded in the lock.
- `build.rs` embeds the source commit.
- Unused mobile icons and crate types were removed. `package.json` is private and requires Node 22 or later.
- The documentation was updated to the current behavior.

### Release details

- A root `NOTICE` gives the copyright line "Copyright 2026 West Virginia University Department of Chemical and Biomedical Engineering". The installers now install `LICENSE.txt` and `NOTICE.txt` beside the bundled-component notices, without a license-acceptance page, and the release folder includes `NOTICE`. The consistency check keeps the copies byte-identical to the root files.
- The executable's version resource and both installers name the department as publisher (CompanyName, Installed-apps Publisher, MSI Manufacturer) and copyright holder (LegalCopyright, NSIS branding text), and the Installed-apps entries link to the repository. The bundle's short and long descriptions state teaching use only, not a medical device, as package metadata; the current Windows installers do not display them. The MSI upgrade code is pinned to the value earlier MSIs used, so it cannot change with the product name.
- **Upgrading 1.0.x under the new publisher.** 1.0.x used Tauri's default publisher `wvu`, taken from the identifier, and Tauri's NSIS template finds the previous install folder, and a previous MSI install, by publisher. With the stock template a 1.1 setup would start the 1.0.x uninstaller without its folder ("Unable to uninstall"), ignore a custom 1.0.x folder, and miss a 1.0.x MSI install. The NSIS setup is therefore built from a custom template, `src-tauri/nsis/installer.nsi`: Tauri CLI 2.11.4's template (kept unmodified beside it) plus marked regions that fall back to the `wvu` registry key, then to the folder of the registered uninstaller, and accept Publisher `wvu` when looking for an MSI install. It never starts the old uninstaller without a folder. The MSI needs no change: the pinned upgrade code makes a 1.1 MSI replace a 1.0.x MSI, although a custom 1.0.x MSI folder is not reused. The consistency check fails when the Tauri CLI version changes ("re-sync src-tauri/nsis/installer.nsi") or when the template differs from upstream outside the marked regions; the [Developer Guide](docs/DEVELOPER_GUIDE.md#re-syncing-the-nsis-template) describes the re-sync. The four upgrades from 1.0.2 still need a Windows Sandbox run ([Release Checklist](docs/RELEASE_CHECKLIST.md#upgrade-from-the-previous-release)).
- `Cargo.toml` and `package.json` list the repository, homepage, and author.
- `SECURITY.md` points to GitHub private vulnerability reporting and lists the supported versions. `CONTRIBUTING.md` states that contributions are licensed under Apache-2.0 (Section 5).
- Issue forms: the bug report asks for the app version, installer type, firmware build, and lab; new forms for lab requests and for third-party source-code requests; blank issues are off, and security reports are pointed to the private form.
- The README's AI-assisted-development disclosure also names Anthropic's Claude (through Claude Code).

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
