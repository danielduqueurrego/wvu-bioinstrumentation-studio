# Instructor Guide

## Instructor mode and Manage Labs

1. In the **Lab** card, tick **I understand Instructor mode can change acquisition settings.**
2. Select **Instructor mode**. If you select it before ticking the box, the app stays in Student mode. The badge in the card changes to **Instructor mode**.
3. Select **Manage Labs** in the **Instructor Lab Editor** section that appears above **Selected lab**.

Student mode sees active labs and normal acquisition only. Instructor mode is a safety step against accidental changes, not a password lock: anyone using the same Windows account can select it. Every mode change and catalog write is recorded in the `profiles` folder's audit logs. Manage Labs and the mode control are unavailable while a recording runs.

## Edit safely

Choose **Edit** on a lab (or **Edit selected lab**, or **Duplicate selected lab** / **Blank simultaneous lab** with a new lab ID) to open an in-memory draft. Nothing is written until you save. You can adjust:

- Labels, A0–A5 mappings, rates, and ADC resolution
- Allowed conversions and digital outputs
- Default plot groups

For pulse oximetry, the mode and RED/DARK 1/IR/DARK 2 order remain fixed. TX/RX pins, RED/IR pins, dwell, labels, and plot defaults are editable within firmware limits. The rules for channel names, rates, and outputs are in [Lab Configuration](LAB_CONFIGURATION.md).

Select **Save changes as new version** once to create the next revision and make it active. **Discard draft** drops the draft without creating a version. Closing Manage Labs with an unsaved draft asks for confirmation first. Existing recordings retain their embedded lab snapshot.

## Course defaults and versions

The Manage Labs table lists every revision with its source and status (Active, Historical, Retired) and these actions:

- **Make active** activates a local revision.
- **Use factory default** activates the shipped factory revision.
- **Restore** brings back a retired revision.
- **Retire** retires a local revision and asks for confirmation when that revision is active. Retiring the active local revision of a factory lab makes the course default active again; retiring the active revision of a lab with no factory version hides that lab from students until another revision is made active.
- **Export** writes a `.lab.json` package.
- **Import lab** adds a package. Re-importing a lab that is already installed is reported as such.
- **Restore course default** returns a lab to its factory configuration. It is never automatic.
- **Reset local customizations** removes every local revision after saving a copy of the catalog (`lab_state.reset-<time>.json`) and asks for confirmation first. Export any lab you want to keep before using it.

Factory labs are always available. A local instructor revision overrides the factory definition only after an explicit save or activation.

When a lab changes, the lab list in the main window refreshes. A Start always records the revision the page shows: if the active revision changed in the meantime, Start is refused until the page has reloaded it.

## Calibration and plots

Lab definitions state which conversion capabilities and default units are offered. Student-specific calibration coefficients are local presets and are not written into the immutable lab. Plot groups affect display only; every acquired channel is still recorded.

The lab field `display.voltage_reference_v` is informational: conversions use the **ADC reference voltage (V)** field in Calibration & Units (default 5 V). See [Calibration and Units](CALIBRATION_AND_UNITS.md).

## Firmware recovery

Use the **Board** card to refresh, verify, or restore the WVU reference firmware. Restore is available whenever a supported UNO R4 WiFi and the bundled Arduino tools are available, even when the firmware is outdated or silent.

For a class recovery:

1. Connect one board at a time. Restore is refused while more than one UNO R4 WiFi is connected.
2. Close Arduino IDE and Arduino Cloud Agent.
3. When the restore window asks for it, press the board's RESET button twice quickly. Every board already running the native-USB WVU firmware needs this; the window gives you 90 s.
4. Do not unplug the board while it says "Writing firmware".

The Board card keeps the last firmware operation and its log path; **Copy diagnostics** collects the details. See [Troubleshooting](TROUBLESHOOTING.md#no-wvu-firmware-firmware-update-required).

## Op Amp Lab installer

The standalone Op Amp Lab installer (`scripts/install_op_amp_lab_profile.bat` with its `.ps1`) installs **Op Amp Lab 2.0.1** for the current Windows user. It is a revision of the factory **General Analog — Development** lab (same lab ID), so after installation it takes General Analog's place in that user's Lab list. **Use factory default** in Manage Labs switches back. Other labs are unchanged.

Teaching use only — not a medical device.
