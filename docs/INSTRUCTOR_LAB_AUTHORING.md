# Instructor Lab Authoring

## Purpose and boundary

**Manage Labs** is an Instructor-only workflow inside Acquisition. It is a local workflow guard,
not account-based or cryptographic authorization. It configures BMEG 420L course capture; it does
not create a generic MCU sequencer, perform physiological interpretation, or authorize clinical use.

Student mode sees active locked lab revisions only. Instructor mode can list historical revisions,
edit, duplicate, import/export, retire/restore, and restore a shipped course default. The explicit
Instructor acknowledgement and the backend mode check are both required for authoring commands.

## Versioning and recording snapshots

**Edit selected lab** creates a draft with the next patch version. **Save changes as new version**
validates the draft, computes its SHA-256 integrity hash, locks it, and makes it active for future
sessions. It never changes an older lab or a completed recording. Recordings retain the selected
lab/profile snapshot, firmware identity, pin map, rate, ADC resolution, output map, calibration
snapshot, and markers.

Retiring removes a revision from the active Student list without deleting it or any recording
snapshot. Restoring a course default creates a new active instructor revision from the shipped
package; it is not a destructive rollback.

## Simultaneous analog labs

An Instructor can configure one through six unique inputs from `A0`–`A5`. Every enabled channel
needs a unique machine-safe channel ID and CSV field name, plus a human label, default visibility,
supported conversion capability, and default plot assignment. A channel may allow Counts/Volts
only, the documented MPXV fixed pressure formula, or a student-owned generic linear calibration.
Generic manual points retain their user-entered output quantity/unit text in the calibration
snapshot and CSV derived-column label; the raw counts remain unchanged.

The controlled firmware advertises its capabilities during the normal handshake. Offline save is
allowed, but capture checks advertised mode, channel limit, ADC resolution, rate, and output mask
before CONFIGURE. It never silently lowers a rate or remaps a resource.

| Behavior | Meaning |
| --- | --- |
| Always LOW | Output remains safe/disabled during the lab. |
| HIGH while recording | Active only after START; LOW on Stop, disconnect, fault, watchdog, and idle. |
| Acquisition-sequenced | Pulse-ox RED/IR only; controlled by the fixed phase order. |

The reference firmware forces D4/D5/D6 LOW on startup, idle, Stop, error, and watchdog fault.
A simultaneous lab cannot request D5/D6 HIGH while recording, preventing accidental RED/IR use.

## Pulse oximetry — fixed four-state mode

The Pulse Oximetry template selects the only supported order:

```text
RED ON → DARK 1 → IR ON → DARK 2
```

The order is not editable. An Instructor may choose distinct TX/RX analog pins (`A0`–`A5`),
distinct RED/IR output pins (`D4`–`D6`), 12- or 14-bit ADC, and a 250–5000 µs dwell. The displayed
cycle rate, `1 / (4 × dwell)`, is nominal; raw timestamps carry measured timing evidence.

Raw fields remain authoritative:

```text
cycle_index,t_us,red_TX,dark1_TX,ir_TX,dark2_TX,red_RX,dark1_RX,ir_RX,dark2_RX
```

Preview plots never replace those fields or calculate SpO2, R, perfusion index, or heart rate.
RED and IR are never HIGH simultaneously.

## Firmware association and import/export

A lab may name **WVU Reference Firmware** or an optional local relative `.ino` reference with an
optional source hash. Saving a lab never compiles or uploads it. The Firmware workspace remains
responsible for explicit Save, Save As, Compile, Upload, and Restore Reference Firmware actions.

Lab export is portable JSON and excludes recordings, local calibration presets, absolute machine
paths, compiled artifacts, and firmware binaries. Import validates schema, resource safety,
supported parameters, version conflicts, and integrity; it never silently overwrites a revision.

## Current templates

| Template | Default mapping | Rate / ADC | Default plots |
| --- | --- | --- | --- |
| ECG | A0 ECG | 1000 Hz / 12-bit | ECG |
| EMG + Force | A0 raw, A1 rectified, A2 envelope, A3 pressure | 1000 Hz / 12-bit | one per signal |
| Blood Pressure + PPG | A0 PPG, A1 MPXV, A2 XGZP, D4 green while recording | 200 Hz / 12-bit | one per signal |
| Pulse Oximetry | TX A0, RX A1, RED D5, IR D6 | 1000 µs dwell / 14-bit | TX previews / RX previews |
| General / Blank simultaneous | Instructor-defined 1–6 inputs | advertised supported settings | one per signal |
