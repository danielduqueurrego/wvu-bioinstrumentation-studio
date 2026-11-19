# Instructor Guide

## Instructor mode and Manage Labs

1. In the **Lab** card, tick **I understand Instructor mode can change acquisition settings.**
2. Select **Instructor mode**. If you select it before ticking the box, the app stays in Student mode. The badge in the card changes to **Instructor mode**.
3. Select **Manage Labs** in the **Instructor Lab Editor** section that appears above **Selected lab**.

Student mode sees active labs and normal acquisition only, plus **Install lab file…** for a lab file you hand out (see [Push a lab to students](#push-a-lab-to-students)). Instructor mode is a safety step against accidental changes, not a password lock: anyone using the same Windows account can select it. Every mode change and catalog write is recorded in the `profiles` folder's audit logs. Manage Labs and the mode control are unavailable while a recording runs.

## Edit safely

Choose **Edit** on a lab (or **Edit selected lab**, or **Duplicate selected lab** / **Blank simultaneous lab** with a new lab ID) to open an in-memory draft. Nothing is written until you save. You can adjust:

- Labels, A0–A5 mappings, rates, and ADC resolution
- Allowed conversions and digital outputs
- Default plot groups

For pulse oximetry, the RED/DARK 1/IR/DARK 2 order is fixed. New pulse-ox labs use the **LED-sequenced** mode (see [Pulse ox with other analog signals](#pulse-ox-with-other-analog-signals)): its channels, RED/IR pins, dwell, labels, and plot defaults are editable within firmware limits. **Pulse Oximetry — legacy 4-state** is offered only while you edit a lab that already uses it; there the TX/RX pins, RED/IR pins, dwell, labels, and plot defaults are editable. The rules for channel names, rates, and outputs are in [Lab Configuration](LAB_CONFIGURATION.md).

Select **Save changes as new version** once to create the next revision and make it active. **Discard draft** drops the draft without creating a version. Closing Manage Labs with an unsaved draft asks for confirmation first. Existing recordings retain their embedded lab snapshot.

## Pulse ox with other analog signals

The built-in **Pulse Oximetry — TX + RX Raw Capture** lab (1.1.0) uses the LED-sequenced mode: the board steps the LEDs RED, DARK 1, IR, DARK 2, and at the end of every state it reads every input of the lab while that state's LEDs are unchanged and writes one row. Every input is therefore recorded in every LED state, so one lab can record a pulse-ox photodetector together with other analog signals.

To make such a lab, for example a photodetector on A0 and other signals on A1–A5:

1. In **Manage Labs**, select the pulse-ox lab and **Duplicate selected lab** (a new lab ID), or **Edit** it for a new revision of the course lab. A lab you start from another lab can be switched with **Acquisition mode** → **Pulse Oximetry — LED-sequenced**, which starts with TX on A0 and RX on A1.
2. In the channel grid, set channel 1 to the photodetector on **A0**, for example ID `ppg`, label `Photodetector`, CSV field `ppg_counts`. Keep **By LED state** ticked for it.
3. Change or **Remove** channel 2, then select **Add channel** for each other signal on **A1–A5** (at most six channels in all), with its own ID, label, CSV field, and **Calibration capability** (for example `pressure`, `pressure_counts`, MPXV pressure). Leave **By LED state** unticked for a signal that does not depend on the LEDs: it is then drawn as one trace at the full row rate. This choice affects the live plot only; every channel is recorded in every row either way.
4. Keep **RED output** on D5 and **IR output** on D6 (or choose two other pins of D4–D6), and set **State dwell (µs)**: 1000 µs gives 1000 rows/s, 250 per LED state. A longer dwell (up to 5000 µs) gives the photodetector more settle time and fewer rows.
5. Choose each channel's **Plot**, then **Save changes as new version** and try the lab with **Simulator (practice)**.

The lab needs WVU firmware build `0x00010007`; boards restored by this version of the application have it.

**Reading the recording.** The CSV has one row per LED state:

```text
record_sequence,t_us,cycle_index,led_state,led_state_name,ppg_counts,ppg_V,pressure_counts,pressure_V,…
```

`led_state` is 0–3 and `led_state_name` is `RED`, `DARK1`, `IR`, or `DARK2`; `cycle_index` numbers the RED-to-DARK 2 cycles. For post-processing, select the rows of one LED state (for example `led_state_name == "RED"`): each LED state's samples are at **a quarter of the row rate** (250 per second at 1000 µs), one per `cycle_index`, so a filter or spectrum of one state must use that sampling rate. Signals that do not depend on the LEDs can use every row at the full row rate. Within a row the inputs are read one after another, a few tens of microseconds apart (see [Hardware](HARDWARE.md#pulse-oximetry)). The application calculates no SpO2 or other physiological value. [Calibration and Units](CALIBRATION_AND_UNITS.md#csv) lists every column.

## Course defaults and versions

The Manage Labs table lists every revision with its source and status (Active, Historical, Retired) and these actions:

- **Make active** activates a local revision.
- **Use factory default** activates the shipped factory revision.
- **Restore** brings back a retired revision.
- **Retire** retires a local revision and asks for confirmation when that revision is active. Retiring the active local revision of a factory lab makes the course default active again; retiring the active revision of a lab with no factory version hides that lab from students until another revision is made active.
- **Export** writes a `.lab.json` package.
- **Save as course default file…** (local revisions only) writes the revision as a factory lab file for the next application build. See [Make a course default](#make-a-course-default).
- **Import lab** adds a package. Re-importing a lab that is already installed is reported as such.
- **Restore course default** returns a lab to its factory configuration. It is never automatic.
- **Reset local customizations** removes every local revision after saving a copy of the catalog (`lab_state.reset-<time>.json`) and asks for confirmation first. Export any lab you want to keep before using it.

Factory labs are always available. A local instructor revision overrides the factory definition only after an explicit save or activation.

When a lab changes, the lab list in the main window refreshes. A Start always records the revision the page shows: if the active revision changed in the meantime, Start is refused until the page has reloaded it.

## Push a lab to students

Students can install a lab file themselves, without Instructor mode:

1. On your computer, edit the lab in **Manage Labs** and select **Save changes as new version** (for example General Analog 2.0.2; General Analog skips 2.0.1, the version of the old Op Amp Lab copy that stays on computers that ran the v1.0.2 installer). Try it with the Simulator.
2. Select **Export** on that revision and share the `.lab.json` file with the class, for example through the course page.
3. Each student selects **Install lab file…** in the **Lab** card, under **Selected lab**, and chooses the file. The button is in Student and Instructor mode and is disabled while a recording runs.
4. The application shows the lab's name, version, inputs, and rate, and asks before installing. When the file replaces a lab on that computer, the question is a warning that says how you can undo it. For a shipped course default: "This replaces <lab> on this computer. Your instructor can undo this with Restore course default." For a local or installed revision: "… Your instructor can undo this in Manage Labs by making <lab> active again."
5. The installed lab is stored as **Imported**, becomes the active version of its lab ID, and is selected.

The same rules as **Import lab** apply:

- The file's hash and the lab rules are checked. A refusal is shown before the student is asked to confirm.
- A file with the same lab ID and version as an installed lab but different content is refused. Give each changed lab a new version.
- Installing a version that is already installed and active changes nothing and says so. A version that is installed but not active (for example after **Restore course default**) is made active again. A version you retired is refused; **Restore** it in Manage Labs instead.
- Installing is refused while the local lab catalog cannot be loaded.
- A course default file (from **Save as course default file…**) is refused: it is for the next application build, not for students. Share the revision's **Export** instead.
- The older Op Amp Lab file from the retired v1.0.2 installer is refused, because the Op Amp Lab is built in. **Import lab** in Manage Labs can still add it deliberately to a catalog that does not hold it; where the migration retired it, use **Restore** instead (see [Op Amp Lab](#op-amp-lab)).

Each install is recorded in `lab_write_audit.log` as `student_install`, with the version it replaced. To undo it on that Windows account: when it replaced the shipped course default, select the lab and use **Restore course default** (or **Retire** the installed version in Manage Labs). When it replaced a local or installed revision, select **Make active** on that revision in Manage Labs; **Restore course default** would select the shipped default instead.

## Make a course default

A course default is a lab compiled into the application, so every computer gets it with the next build. In Instructor mode, select **Save as course default file…** on a local revision in **Manage Labs**. The Save dialog suggests the lab's factory file name, for example `general_analog_development.profile.json`. A new lab gets a name made from its display name.

The file gets the same normalization as `scripts/update_factory_labs.ps1 promote`:

- source `built_in` and status `locked`;
- the next factory version: one MINOR above the lab's published versions and the revision's own version, with PATCH 0. For example General Analog 2.0.2 gives 2.1.0, and a new lab ID saved as 1.0.0 and edited to 1.0.2 gives 1.1.0, so the factory version never has the version of one of your local revisions;
- a fresh canonical hash, checked against the rules for a lab saved now.

The file name must end in `.profile.json`. Saving the file does not change the lab catalog, and it is recorded in `lab_write_audit.log` as `course_default_export`. A revision with the same content as the shipped course default is refused, because there is nothing new to save.

Lab Manager then lists the next steps. A maintainer does these in the repository:

1. Copy the file into `profiles/`, replacing the lab's current file.
2. Run `scripts/update_factory_labs.ps1 check` and follow what it reports. `fix <file>` records the new version in `profiles/factory-labs.lock.json` and keeps the published version the file replaced in `profiles/legacy/`, taken from the copy committed in Git.
3. For a new lab, add the `factory_lab_file!("<file>"),` line at the end of `FACTORY_LAB_FILES` in `src-tauri/src/profiles.rs`.
4. Rebuild the application, and commit the lab file, the lock, and the legacy file together.

[How to change a built-in lab](LAB_CONFIGURATION.md#how-to-change-a-built-in-lab) has the details.

Do not hand the course default file to students. Every file saved from one build of a lab gets the same version, whatever its content, so two drafts could share a version, and the version that is finally built in could differ again; **Install lab file…** refuses it. Until the new build is installed, push the change with the revision's **Export** (see [Push a lab to students](#push-a-lab-to-students)).

## Calibration and plots

Lab definitions state which conversion capabilities and default units are offered. Student-specific calibration coefficients are local presets and are not written into the immutable lab. Plot groups affect display only; every acquired channel is still recorded.

The lab field `display.voltage_reference_v` is informational: conversions use the **ADC reference voltage (V)** field in Calibration & Units (default 5 V). See [Calibration and Units](CALIBRATION_AND_UNITS.md).

## Firmware recovery

Use the **Board** card to refresh, verify, or restore the WVU reference firmware. Restore is available whenever a supported UNO R4 WiFi and the bundled Arduino tools are available, even when the firmware is outdated or silent.

**After updating to this version, restore every board once.** This version uses WVU firmware build `0x00010007` (it adds the LED-sequenced pulse-ox mode), and the application accepts only its own build. A board restored by version 1.0.x shows **Older WVU firmware** until it is restored. **Update the application on every computer that connects to the course boards before you restore any board**: lab computers, your own and your TAs' laptops, and students' own installations. Have students install this version before the boards are restored. Version 1.0.x meeting a restored board reports **Firmware update required** and points to **Restore WVU Firmware**; do not restore from that computer, because it would put the older firmware back and undo the update for the next class. Update that computer instead. From this version on, an application older than a board's firmware says **Newer WVU firmware** and asks for the application to be updated.

For a class recovery:

1. Connect one board at a time. Restore is refused while more than one UNO R4 WiFi is connected.
2. Close Arduino IDE and Arduino Cloud Agent.
3. When the restore window asks for it, press the board's RESET button twice quickly. Every board already running the native-USB WVU firmware needs this; the window gives you 90 s.
4. Do not unplug the board while it says "Writing firmware".

The Board card keeps the last firmware operation and its log path; **Copy diagnostics** collects the details. See [Troubleshooting](TROUBLESHOOTING.md#no-wvu-firmware-firmware-update-required).

## Op Amp Lab

**Op Amp Lab** 1.0.0 is a factory course lab with its own lab ID, `wvu.bmeg420l.op_amp.course.capture.v1`. It records Vmid on A0 (`vmid_counts`) and Vout on A1 (`vout_counts`) at 100 frames/s with 14-bit ADC, one plot per signal, and counts or volts display. It is bench-only and needs no acknowledgement at Start. Edit, Duplicate, Export, and Restore course default work as for the other factory labs.

The standalone v1.0.2 installer (`install_op_amp_lab_profile.bat` and `.ps1`) is retired; do not give it to students. It stored **Op Amp Lab 2.0.1** as a revision of **General Analog — Development** (same lab ID), so on those computers it replaced General Analog in the Lab list. That copy's CSV columns were `a0_counts`, `a0_V`, `01_counts`, and `01_V`; the built-in lab writes `vmid_counts`, `vmid_V`, `vout_counts`, and `vout_V`, so update analysis templates that use the old names. Recordings already made are unchanged.

**One-time migration.** On the first start of this version, a catalog whose active General Analog revision is that copy (version 2.0.1, as installed by the script or saved as your own instructor revision) is changed once:

1. The catalog is copied to `lab_state.pre-migration-<time>.json` in the `profiles` folder. If the copy fails, nothing changes and the next start tries again.
2. Op Amp Lab 2.0.1 is retired (it stays in the catalog and in Manage Labs) and General Analog's course default is active again.
3. The change is recorded in `lab_state.json` (`applied_migrations`), in the application log (`LAB_CATALOG_MIGRATED`), and in `lab_write_audit.log` (`op_amp_builtin_v1`).
4. The **Lab** card shows a notice with the backup path, in Student and Instructor mode, until the application closes.

If the catalog cannot be written at that moment, this launch still shows the migrated lab list, says the change was not saved, and the next start tries again with the same backup. The migration never runs while the catalog is locked by another window or read-only, and it runs only once per catalog.

To keep the old copy anyway, select **Restore** on Op Amp Lab 2.0.1 in Manage Labs. (**Import lab** adds it only to a catalog that does not hold it, for example after **Reset local customizations**; on a migrated catalog it reports the copy as already installed.) The migration is recorded with the change, so your choice is kept at later starts. **Reset local customizations** removes the copy and the record.

Teaching use only — not a medical device.
