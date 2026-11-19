# Lab Configuration

Course labs are versioned acquisition definitions. Every recording embeds a snapshot of the selected lab, so later edits never change historical data.

## Factory labs

| Lab | Inputs/outputs | Rate | ADC | Default plots |
| --- | --- | ---: | ---: | --- |
| ECG — Course Capture | A0 ECG | 1000 frames/s | 14 bit | ECG |
| EMG + Force — Course Capture | A0 raw EMG, A1 rectified, A2 envelope, A3 pressure | 1000 frames/s | 14 bit | one plot per signal |
| Blood Pressure + PPG — Course Capture | A0 PPG, A1 MPXV, A2 XGZP, D4 green | 200 frames/s | 14 bit | PPG, MPXV, XGZP |
| Pulse Oximetry — TX + RX Raw Capture (1.1.0, LED-sequenced) | A0 TX, A1 RX, D5 red, D6 IR | 1000 records/s (one per LED state; 250/s per state) | 14 bit | TX by LED state; RX by LED state |
| General Analog — Development | 1–6 unique A0–A5 inputs | 1000 frames/s | 14 bit | one plot per selected input |
| Op Amp Lab | A0 Vmid, A1 Vout | 100 frames/s | 14 bit | Vmid; Vout |

Op Amp Lab (`wvu.bmeg420l.op_amp.course.capture.v1`, category `course_op_amp`) is a bench-only factory lab since version 1.1.0. Its CSV fields are `vmid_counts` and `vout_counts`, and both channels offer counts and volts. It replaces the Op Amp Lab 2.0.1 that the retired v1.0.2 student installer stored under General Analog's lab ID; see [Catalog file](#catalog-file) for the one-time migration of that copy.

The pulse-ox lab (`wvu.bmeg420l.pulseox.txrx.raw.course.capture.v1`) is version **1.1.0** and uses the **LED-sequenced** mode (`pulseox_sequenced`): the LEDs step `RED → DARK 1 → IR → DARK 2` with a 1000 µs dwell, and at the end of every state the board reads both photodetectors (TX on A0, RX on A1) and records one row: `[led_state, tx_counts, rx_counts]`. Both channels are shown by LED state in the live plot by default. It needs firmware build `0x00010007`, so every board needs one **Restore WVU Firmware** after the update. Its notices, export name, and category are those of 1.0.0, and it still calculates nothing: raw RED/DARK/IR/DARK values only, no SpO2.

Version 1.0.0 used the legacy 4-state mode (`pulseox_4state`: one 8-field record per cycle, about 250 cycles/s). It is published and stays in `profiles/legacy/pulseox_tx_rx_course_capture_1.0.0.profile.json`. It is not compiled in, but **Install lab file…** accepts that file as a rollback, and **Restore course default** returns to 1.1.0.

In both modes the firmware advances state deadlines on one absolute schedule rather than scheduling from the end of ADC and bookkeeping work, and RED and IR are never driven HIGH at the same time. Each record carries its measured timestamp; the requested rate remains configuration metadata rather than a replacement for measured timing.

An LED-sequenced recording's CSV has one row per LED state, with the state in two columns before the channels. For the built-in lab the header is:

```text
record_sequence,t_us,cycle_index,led_state,led_state_name,tx_counts,tx_V,rx_counts,rx_V
```

[Calibration and Units](CALIBRATION_AND_UNITS.md#csv) describes every column.

Recording metadata preserves the selected profile's bench-only status, human-connection authorization, course-notice acknowledgement, and non-medical validation status. The acquisition screen does not ask students to classify the connected signal separately.

## Factory lab versions

Factory labs are compiled into the application from `profiles/*.profile.json` (the `FACTORY_LAB_FILES` list in `src-tauri/src/profiles.rs`), and each one is checked against its canonical SHA-256 hash when the application starts. They follow version policy P2:

- **A factory lab version is always `MAJOR.MINOR.0`.** Any change to a factory lab's content, including a notice, a channel label, or the firmware requirement, is published as the next MINOR version (`X.Y+1.0`). A published version never changes.
- **An edited lab always has PATCH ≥ 1.** **Save changes as new version** numbers a revision one patch above the highest revision with the same MAJOR.MINOR (General Analog skips 2.0.1; see [Instructor revisions](#instructor-revisions)). A new lab (Duplicate, Blank) starts locally at 1.0.0 under its own new lab ID, so when such a lab becomes a factory lab, its first factory version is one MINOR above the revision it was made from: a lab saved as 1.0.0 and edited to 1.0.2 is built in as 1.1.0. Either way a factory version never has the ID and version of a local revision.
- **`required_firmware.build` is the minimum WVU firmware build whose features the lab uses**, not the build current when the lab was written. It must be a build the application accepts (`ACCEPTED_LAB_FIRMWARE_BUILDS` in `profiles.rs`: `0x00010007` and every earlier build down to `0x00010001`). When a newer firmware build ships, factory labs that do not use its new features keep their build, version, and hash: with build `0x00010007`, the five other factory labs still name `0x00010006`, and only the LED-sequenced pulse-ox lab 1.1.0 names `0x00010007`. The connected board must still run exactly this application's build (see [Troubleshooting](TROUBLESHOOTING.md#older-or-newer-wvu-firmware)).
- **`profiles/factory-labs.lock.json` records every factory lab version ever shipped** as `profile_id`, `profile_version`, and `canonical_hash`, one entry per line. It is append-only: an entry that has been committed is never changed or removed. (Before you commit, `git restore profiles/factory-labs.lock.json` undoes an entry the tool appended for a change you are discarding.)
- **Every published version stays available as a file**: the lab's current file in `profiles/`, or an earlier version in `profiles/legacy/<file>_<version>.profile.json` (for example `profiles/legacy/ecg_course_capture_1.0.0.profile.json`). Legacy files are not compiled in and must match their lock entries. `fix` and `promote` keep the version a file held there before they replace it, so you do not copy it yourself.

Rust tests enforce this: every compiled-in lab is in the lock with the same hash and is the newest locked version of its lab; every `profiles/*.profile.json` is compiled in; every factory lab is `MAJOR.MINOR.0`, `built_in`, and `locked`; the lock lists no ID and version twice; every lock entry is held by a file; and the hashes of the labs v1.0.2 shipped stay unchanged (read from their current or legacy files), which guards the canonical serialization every stored lab and recording snapshot was hashed with.

When a lab PC updates to a release with a new factory version (for example ECG 1.1.0), a local revision that an instructor made active (for example 1.0.2) stays active; **Restore course default** switches to the new factory version, and editing that creates 1.1.1. The pulse-ox lab follows the same rule: a PC whose instructor made a local 4-state revision active keeps recording in the legacy 4-state mode (build `0x00010007` still supports it) until **Restore course default** selects 1.1.0.

## How to change a built-in lab

Use `scripts/update_factory_labs.ps1` (PowerShell 7 or Windows PowerShell 5.1, with the Rust toolchain from the [Developer Guide](DEVELOPER_GUIDE.md#prerequisites)). It runs the maintainer tool `src-tauri/examples/factory_labs.rs` with `cargo run --locked --example factory_labs`; the first run compiles the application library, which takes a few minutes. Exit codes: 0 success, 1 problems found or change refused, 2 usage error.

| Command | What it does |
| --- | --- |
| `check` | Validates every `profiles/*.profile.json` and `profiles/legacy/*.profile.json`: the lab rules (including those for a lab saved now), its canonical hash, `MAJOR.MINOR.0`, `built_in`, `locked`, one file per lab ID, that each file is compiled in and each compiled-in file exists, the lock, and that a file still holds every version in the lock. |
| `fix <file> [--bump-minor]` | Re-hashes a factory lab file in `profiles/` after a hand edit and records its version in the lock. Only the version and hash values in the file change; the rest of its layout is kept. Different content at a version already in the lock is refused; `--bump-minor` publishes it as the next MINOR version and appends that to the lock. |
| `promote <exported.lab.json> [--name <file-name>]` | Turns a lab exported from Lab Manager into a factory lab file: source `built_in`, status `locked`, the same normalization as **Save changes as new version**, and the next factory version: one MINOR above the lab's published versions and the export's own version (ECG 1.0.0 edited to 1.0.1 gives 1.1.0; a new lab ID exported as 1.0.x gives 1.1.0). It replaces the lab's current factory file, or writes `profiles/<name>.profile.json` for a new lab (named after the display name unless `--name` is given), appends the version to the lock, and prints the `FACTORY_LAB_FILES` line a new lab needs. Content identical to the current factory version is refused. |

The `-BumpMinor` and `-Name` spellings work too. `fix` and `promote` stop before writing anything when the change is refused.

**Replaced versions are kept.** Before `fix` or `promote` gives a lab's file a new version, the published version the file held is written to `profiles/legacy/<file>_<version>.profile.json`. It comes from the file itself, or, when the file has already changed (a hand edit, or a course default file copied over it), from the copy committed in Git (`git cat-file blob HEAD:profiles/<file>`). If neither holds the published version, nothing is written and the error asks you to put the published file in `profiles/legacy/` yourself, for example from an earlier commit. Commit the legacy file with the lab file and the lock.

**Option A: edit in Lab Manager, then promote** (recommended, because the editor applies every rule):

1. Start a development sandbox (`pwsh -NoProfile -File scripts\start_dev_sandbox.ps1`) so your real catalog is untouched. In Instructor mode, **Manage Labs** → **Edit** the lab → **Save changes as new version** (for example ECG 1.0.1), and try it with the Simulator.
2. **Export** that revision, for example to `Desktop\wvu.bmeg420l.ecg.course.capture.v1_1.0.1.lab.json`.
3. Promote it:

   ```powershell
   pwsh -NoProfile -File scripts\update_factory_labs.ps1 promote "$HOME\Desktop\wvu.bmeg420l.ecg.course.capture.v1_1.0.1.lab.json"
   ```

   ```text
   Wrote C:\...\profiles\ecg_course_capture.profile.json
     wvu.bmeg420l.ecg.course.capture.v1 1.1.0 canonical hash <64 hexadecimal digits>
     replaces factory version 1.0.0 (still recorded in the lock)
     kept the published version it replaced as C:\...\profiles\legacy\ecg_course_capture_1.0.0.profile.json
     appended to C:\...\profiles\factory-labs.lock.json
   Next steps:
     1. The file is already compiled in (FACTORY_LAB_FILES); a rebuild picks up the new version.
     2. Run `scripts/update_factory_labs.ps1 check`.
     3. Update the Factory labs table in docs/LAB_CONFIGURATION.md and CHANGELOG.md, then run the checks in docs/DEVELOPER_GUIDE.md.
     4. Commit the lab file, the lock, and any file kept in profiles/legacy/ together.
   ```

   For a new lab ID, step 1 instead prints a line such as `factory_lab_file!("strain_gauge_course_capture.profile.json"),` to add at the end of `FACTORY_LAB_FILES` (it expands to `include_str!("../../profiles/strain_gauge_course_capture.profile.json")`). Append new labs last. A new lab has no earlier version to keep.
4. Run `check`, update the table above and `CHANGELOG.md`, run every check in the [Developer Guide](DEVELOPER_GUIDE.md#setup-and-checks), and commit the lab file, the lock, and the legacy file together.

**Option B: edit the file by hand:**

1. Edit `profiles/ecg_course_capture.profile.json`, leaving `profile_version` and `canonical_hash` as they are.
2. Run `fix`. Version 1.0.0 is already published, so the first attempt is refused and names the next version:

   ```powershell
   pwsh -NoProfile -File scripts\update_factory_labs.ps1 fix profiles\ecg_course_capture.profile.json
   ```

   ```text
   error: profile validation failed: wvu.bmeg420l.ecg.course.capture.v1 1.0.0 is already published with hash 3c598866..., and a published factory lab never changes. Re-run with --bump-minor to publish the change as 1.1.0
   ```

   ```powershell
   pwsh -NoProfile -File scripts\update_factory_labs.ps1 fix profiles\ecg_course_capture.profile.json --bump-minor
   ```

   ```text
   wvu.bmeg420l.ecg.course.capture.v1: 1.0.0 -> 1.1.0 (new MINOR version)
   wvu.bmeg420l.ecg.course.capture.v1 1.1.0 canonical hash <64 hexadecimal digits>
     file: updated
     lock: new entry appended
     kept the published version it replaced as C:\...\profiles\legacy\ecg_course_capture_1.0.0.profile.json
   ```

   The file was already edited, so the published 1.0.0 is taken from the copy committed in Git. Edit a committed file; if 1.0.0 is in neither place, `fix --bump-minor` refuses and changes nothing.
3. Run `check` and the full checks, then commit the lab file, the lock, and the legacy file together. Running `fix` on a file whose content matches its lock entry only repairs a damaged hash value; it never changes the version.

**Option C: Save as course default file… in Lab Manager** (Instructor mode, no Rust toolchain needed to make the file):

1. Save and try the revision in Lab Manager as in Option A.
2. Select **Save as course default file…** on that revision. The command is `export_course_default_file` (`ProfileStore::export_course_default`); Instructor mode is enforced in Rust. The Save dialog suggests the lab's compiled-in file name, or, for a new lab ID, a name made from the display name that no other factory lab uses. The name must end in `.profile.json`.
3. The application writes the file `promote` would write from an export of the same revision, in the same layout: source `built_in`, status `locked`, the Save normalization, a fresh hash, and the next factory version from the lock and course default compiled into the application and the revision's own version (a new lab ID saved as 1.0.x gets 1.1.0). A revision with the same content as the shipped course default is refused. The file is written to a temporary file and then moved into place; the catalog is not changed. `lab_write_audit.log` records `course_default_export` with the revision and the new version.
4. Copy the file into `profiles/`, replacing the lab's current file. Run `check`: it reports that the version is not in the lock yet and that no file holds the replaced published version any more. Then run `fix profiles\<file>`, which records the version without changing the file and keeps the replaced published version in `profiles/legacy/` from the copy committed in Git. For a new lab, add the `factory_lab_file!` line that Lab Manager shows. Rebuild, run `check` and the full checks, and commit the lab file, the lock, and the legacy file together.

If the repository's lock has a newer version than the application that wrote the file, `fix` refuses it. Run `fix --bump-minor`, or run `promote` on the file instead.

A course default file is for the repository, not for students: every file saved from one build of a lab gets the same version, whatever its content, so **Install lab file…** refuses it. To give students a change before the new build is installed, **Export** the revision and share that `.lab.json` file (see [Lab files for students](#lab-files-for-students)).

## Lab files for students

**Install lab file…** in the Lab card installs a lab file in Student or Instructor mode. It is a second way into the catalog, with Import lab's parsing, validation, and conflict rules:

1. `inspect_lab_file { source }` reads the file without changing anything. It returns the lab's ID, version, name, mode, inputs, rate, and ADC resolution, whether that exact version is already installed and active, the active lab it would replace (a course default included), and the reason an install would be refused. It applies every rule `install_lab_file` applies, in the same order, so a refusal is shown before the student is asked.
2. The page asks for confirmation. When the file replaces a lab, it is a warning that says how the instructor can undo it: with **Restore course default** when the replaced lab is the shipped course default (`replaces.course_default`), otherwise by making the replaced version active again in Manage Labs (**Restore course default** would select the shipped default, not that version).
3. `install_lab_file { source }` stores the lab as `imported`, re-hashed and checked with the rules for a lab saved now, and makes it the active version of its lab ID. The write is a normal transactional catalog write under the catalog lock.

It is refused:

- while a recording runs (the page disables the button, and Rust checks the session again);
- while the catalog is unavailable or read-only;
- for a file whose own hash does not match, or that is not `locked`;
- for the Op Amp Lab 2.0.1 copy of the retired v1.0.2 installer;
- for a course default file (`built_in`) that is not a factory lab version this application ships with that content, such as a file from **Save as course default file…**;
- for a lab ID and version that is installed with different content, is stored but cannot be loaded, or was retired on this computer;
- for a lab that breaks a rule for a lab saved now (for example a frame rate the firmware does not support, or CSV columns that repeat).

When the identical version is installed and active, the file is reported as already installed and nothing changes. When it is installed but not active (for example after **Restore course default**), installing the file makes it active again; nothing is added to the catalog. `lab_write_audit.log` records `student_install` with the lab ID, the version it replaced, and the new version, and the application log records `LAB_FILE_INSTALLED`. **Restore course default** (or **Retire** on the installed version) undoes it when it replaced the course default; otherwise **Make active** on the replaced version in Manage Labs does. Labs are stored per Windows account, so an install and its undo apply to the account they are made in.

Keep the teaching-use notices: every factory lab must stay `not_medical_device` with visible safety notices, and validation refuses a lab without them.

## Instructor revisions

In Instructor mode, **Manage Labs** lists the catalog. **Edit**, **Duplicate selected lab**, and **Blank simultaneous lab** open an in-memory draft that is not written until **Save changes as new version**. Only these explicit actions write the catalog:

- **Save changes as new version**
- **Import lab**
- **Install lab file…** (in the Lab card, also in Student mode; see [Lab files for students](#lab-files-for-students))
- **Retire**
- **Restore**
- **Make active** / **Use factory default**
- **Restore course default**
- **Reset local customizations**

Reading, selecting, acquiring, plotting, calibrating, or navigating does not create a version. Manage Labs is unavailable while a recording runs. Start sends the version of the lab the page shows, and the backend refuses to start if a different revision has become active.

The **Acquisition mode** menu offers **Simultaneous Analog** and **Pulse Oximetry — LED-sequenced**. **Pulse Oximetry — legacy 4-state** appears only while editing a lab that already uses that mode (an older local pulse-ox revision, or the 1.0.0 file installed as a rollback), so existing 4-state labs can still be edited; new pulse-ox labs use the LED-sequenced mode.

For simultaneous analog labs, an instructor can configure one to six unique A0–A5 channels, machine-safe channel/CSV names, rate, supported ADC resolution, conversion capability, visibility, and plot assignment. D4/D5/D6 can be configured only with supported safe behaviors: LOW, HIGH while recording, or the fixed pulse-ox sequence. The saved output list is exactly what is driven: an unchecked output stays LOW.

For **LED-sequenced** pulse-ox labs the same channel grid is used, with at most six channels and an extra **By LED state** column, plus:

- **RED output** and **IR output** (two different pins of D4–D6, both `acquisition_sequenced`; no other output);
- **State dwell (µs)** (1000–5000), which sets the record rate: `round(1,000,000 / dwell)` records/s, shown as rows per second for each channel and per LED state. Use at least 2000 µs when the lab has more than two channels: at 1000 µs the board then reads many rows late (see [Hardware](HARDWARE.md#pulse-oximetry));
- the CSV columns the lab will write.

Choosing the mode for a lab starts from TX on A0 and RX on A1 (`tx_counts`, `rx_counts`), both shown by LED state, 14-bit, 1000 µs, D5 RED and D6 IR. Add channels for other signals (A2–A5, for example a pressure sensor or a second photodetector); a channel added later is not shown by LED state unless you tick it. **By LED state** (`split_by_led_state`) is display only: it decides whether the live plot draws the channel as four traces (RED, DARK 1, IR, DARK 2) or as one trace at the full record rate. It never changes acquisition, the recorded data, or the CSV, and students can change it during a recording with **Show by LED state** in Live signals.

Rules for a revision saved or imported now (labs that are already installed keep loading unchanged):

- CSV fields use 1–64 letters, digits, or underscores (for example `01_counts`). Every generated column must be distinct, ignoring case: the raw field, its `_V` column, and the `_kPa`/`_mmHg` columns of an MPXV-capable channel, and none may reuse `record_sequence`, `t_us`, `cycle_index`, `led_state`, or `led_state_name`. If a student's linear calibration unit would repeat an existing column (for example volts next to `_V`), the CSV names that column `<name>_cal_<unit>`; unsafe names in older labs are quoted in the CSV header. (A stored lab that already uses `led_state` or `led_state_name` still loads; only a revision saved now must rename it.)
- Simultaneous frame rates are the firmware's 100, 200, 250, 500, or 1000 frames/s. The legacy 4-state cycle rate is `round(1,000,000 / (4 × dwell))` and dwell is 250–5000 µs. The LED-sequenced record rate is `round(1,000,000 / dwell)` and dwell is 1000–5000 µs. Dwell is a whole number of microseconds in both modes; each LED state gets `1,000,000 / (4 × dwell)` rows/s.
- LED-sequenced labs have 1–6 channels with unique pins, IDs, and CSV names, all in `ADC counts`; distinct `acquisition_sequenced` RED and IR outputs on D4–D6 and no other output; `required_firmware.build` `0x00010007` or later (checked before the lab's hash, so an older build is named as the reason); and plot defaults, if any, that put every channel in exactly one plot. `split_by_led_state` is allowed only in this mode.
- Legacy 4-state pulse-ox labs have no analog channels and only their RED and IR outputs. That mode never used a channel list, so the unused `Channel 1 / A0` that the v1.0.2 editor stored in pulse-ox labs is dropped when such a package is imported (and when the lab is next edited); it is not a reason to refuse the package. In both pulse-ox modes, RED/IR come from an optional `pulse_ox_role` (`"red"`/`"ir"`) on the output; without it, the label `Red LED` / `IR LED` (any letter case) decides.
- Names, labels, descriptions, and safety notices have length limits.
- `display.voltage_reference_v` must be positive but is informational only. Conversions use the ADC reference voltage entered in the Calibration & Units card (see [Calibration and Units](CALIBRATION_AND_UNITS.md#raw-data)).

A lab's `allowed_duration_modes` and `minimum_custom_duration_seconds` are enforced: the page offers only the allowed modes, and Start refuses any other duration. A preset duration is always allowed.

A new revision is numbered one patch above the highest existing revision with the same ID and MAJOR.MINOR (factory, retired, and local revisions all count), so editing after **Restore course default** or after activating an older revision works. General Analog 2.0.1 always counts as taken, so the first edit of General Analog 2.0.0 is 2.0.2 on every computer: 2.0.1 is the old Op Amp Lab copy, which the migration keeps (retired) on computers that ran the v1.0.2 installer, and a different 2.0.1 could not be installed there.

## Catalog file

The catalog (`lab_state.json`) is replaced atomically, and writes from two open windows are serialized; a window always continues from the other window's saved revisions. If one stored lab cannot be loaded, only that lab is hidden: it stays in the file, the application log records `LAB_PROFILE_SKIPPED`, and the Lab panel shows a notice in both modes. A stored lab with the same ID and version as a course default never replaces it. With the same content (for example a lab installed locally before it shipped as a factory lab) it is not needed: it is dropped from the file at the next catalog write, and the log records `LAB_PROFILE_SUPERSEDED`. With different content it is handled like any other lab that cannot be loaded: it stays in the file unchanged, and the notice names the course default it conflicts with. When `lab_state.json` is missing or unreadable after an interrupted save, a complete leftover copy is restored (an unreadable file is kept as `lab_state.corrupt-<time>.json`). Only a file that was read but does not parse is replaced this way; a catalog that cannot be read at all (for example while another program holds it) is never replaced, and only the factory labs are shown until the next start. Leftover copies that cannot be read are renamed to `lab_state.corrupt-<time>-<n>.json` and reported to instructors instead of being deleted. **Reset local customizations** first saves the catalog as `lab_state.reset-<time>.json` and reports that path.

**One-time migrations.** At startup, and only while it holds the catalog lock with a writable catalog (never when it reloads a catalog another window changed), the application applies catalog migrations that are not yet recorded in the file's `applied_migrations` list. That field is additive: it is omitted when empty, older versions ignore it, and lab hashes do not depend on it. There is one migration, `op_amp_builtin_v1`:

- **Trigger.** The catalog holds General Analog (`wvu.bmeg420l.general.analog.development.v2`) version 2.0.1 with the verified canonical hash `c73cc1f2941910696d88f934b7b4c09b8b110438a4886d7d799b13807f61471d` (as the v1.0.2 student installer stored it, source `imported`) or `bcc08868305ca55e81920e3a5805b020cfa612a0374e75fd5a290fa11f918ad9` (the same lab saved as an instructor revision), it is the active revision, and it is not retired.
- **Actions.** The catalog is copied to `lab_state.pre-migration-<time>.json`; that revision is retired (it stays in the file); General Analog's course default becomes active; the application log records `LAB_CATALOG_MIGRATED` with the details and `lab_write_audit.log` records `op_amp_builtin_v1`; and the Lab panel shows, in both modes, "The Op Amp Lab is now built in. The older Op Amp Lab copy that replaced General Analog was retired; a backup was saved to <path>."
- **Failures.** If the copy fails, nothing changes (`LAB_CATALOG_MIGRATION_DEFERRED`). The copy holds only the catalog's bytes, so a read-only catalog gives a normal, writable copy. If the write fails, this launch keeps the migrated lab list, the notice says it was not saved, the log records `LAB_CATALOG_MIGRATION_NOT_SAVED`, and the next start tries again; it reuses the copy that already holds the same catalog instead of adding another. Without the catalog lock nothing is written and the next start tries again.
- **Record.** `op_amp_builtin_v1` counts as applied once the migration has run, and for a catalog read (at startup or after another window changed it) where that revision is in the catalog but already inactive or retired. The next catalog write records it in `applied_migrations` (a catalog is not rewritten at startup just for this), so an instructor who restores or activates the revision later keeps it; so does an instructor who imports it with **Import lab** into a catalog that does not hold it (on a migrated catalog, Import lab reports it as already installed; use **Restore**). It is never recorded while that revision is still selected and the migration has not run (for example after a start without the catalog lock, or when another program selected it while the application was open), so the next start migrates it. It is not recorded while the revision is absent, so a copy that the old installer adds later is still migrated. **Reset local customizations** clears the list along with the local revisions.

## Compatibility and import/export

The connected firmware capabilities are checked before recording. A lab can be edited without a connected board, but an unsupported configuration cannot start. Imported lab JSON is validated and never silently overwrites a conflicting version; re-importing a lab that is already installed (for example an exported instructor lab now stored as imported) is reported as already installed. **Install lab file…** applies the same rules. Factory defaults are always available; local customizations are distinct from immutable shipped definitions.
