# Instructor Guide

## Instructor mode and Manage Labs

Select Instructor mode, acknowledge the local authoring notice, then use **Manage Labs** next to the lab selector. Student mode sees active labs and normal acquisition only. Instructor mode is a safety step against accidental changes, not a password lock: anyone using the same Windows account can select it, and every mode change and catalog write is recorded in the `profiles` folder's audit logs.

## Edit safely

Choose **Edit** to create a draft. Adjust labels, A0–A5 mappings, rates, ADC resolution, allowed conversions, digital outputs, and default plot groups. For pulse oximetry, the mode and RED/DARK 1/IR/DARK 2 order remain fixed; TX/RX pins, RED/IR pins, dwell, labels, and plot defaults are editable within firmware limits.

Select **Save changes** once to create the next revision and activate it. Cancel discards the draft. Existing recordings retain their embedded lab snapshot.

## Course defaults and versions

Factory labs are always available. A local instructor revision overrides the factory definition only after an explicit save. Use Restore course default to deliberately return to the factory configuration; it is not automatic. Retiring the active local revision of a factory lab makes the course default active again. **Reset local customizations** removes every local revision after saving a copy of the catalog (`lab_state.reset-<time>.json`); export any lab you want to keep first.

## Calibration and plots

Lab definitions state which conversion capabilities and default units are offered. Student-specific calibration coefficients are local presets and are not written into the immutable lab. Plot groups affect display only; every acquired channel is still recorded.

## Firmware recovery

Use the top Board controls to refresh, verify, or restore the WVU reference firmware. Restore is available whenever a supported UNO R4 WiFi and the bundled Arduino tools are available, even when the firmware is outdated or silent.
