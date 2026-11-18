<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { confirm } from '@tauri-apps/plugin-dialog';
  import {
    calibrationById,
    displayUnitLabel,
    mergeCalibrationPresets,
    type CalibrationPreset,
    type DisplayUnit,
    type RecordingCalibration
  } from '$lib/calibration';
  import { applyCalibrationList } from '$lib/calibration-selection';

  /**
   * The Calibration & Units card: Vref and the MPXV supply, the saved linear
   * calibrations of one channel (list, select, delete), and each plotted
   * channel's display unit. Raw counts stay authoritative; these settings
   * change only the live display and the derived CSV columns, and are frozen
   * into each new recording's metadata.
   *
   * The card owns the saved list. The page owns the selection it records
   * (`activeCalibration`, bound) and tells the card when the lab changes
   * (`loadLab`) or a calibration was saved (`applySaved`).
   */
  type Props = {
    adcReferenceV: number | null;
    mpxvSensorSupplyV: number | null;
    adcReferenceError: string;
    mpxvSupplyError: string;
    /** The last valid values, which the display and the recording keep using. */
    appliedAdcReferenceV: number;
    appliedMpxvSupplyV: number;
    hasMpxvChannel: boolean;
    /** The selected lab. */
    profileId: string;
    calibratableChannels: Array<{ id: string; label: string }>;
    channelId: string;
    selectedCalibrationId: string;
    activeCalibration?: CalibrationPreset;
    plotChannels: Array<{ id: string; label: string }>;
    channelUnits: Record<string, DisplayUnit>;
    channelUnitOptions: Record<string, DisplayUnit[]>;
    recordingCalibration: RecordingCalibration;
    category?: string;
    pulseox: boolean;
    locked: boolean;
    onCalibrate: () => void;
    onStatus: (message: string) => void;
  };

  let {
    adcReferenceV = $bindable(),
    mpxvSensorSupplyV = $bindable(),
    adcReferenceError,
    mpxvSupplyError,
    appliedAdcReferenceV,
    appliedMpxvSupplyV,
    hasMpxvChannel,
    profileId,
    calibratableChannels,
    channelId = $bindable(),
    selectedCalibrationId = $bindable(),
    activeCalibration = $bindable(),
    plotChannels,
    channelUnits = $bindable(),
    channelUnitOptions,
    recordingCalibration,
    category,
    pulseox,
    locked,
    onCalibrate,
    onStatus
  }: Props = $props();

  let savedCalibrations = $state<CalibrationPreset[]>([]);
  // Only the newest `list_calibrations` response is applied (see `refresh`).
  let requestCounter = 0;

  const xgzp = $derived(channelId === 'xgzp');
  const unitFor = (preset: CalibrationPreset): DisplayUnit => preset.output_units.trim().toLowerCase() === 'mmhg' ? 'mmhg' : 'calibrated';

  function clearSelection() {
    savedCalibrations = [];
    selectedCalibrationId = '';
    activeCalibration = undefined;
  }

  /**
   * Loads the saved calibrations of one lab and channel. The IDs are passed
   * explicitly, and only the newest response that still matches the current
   * selection is applied, so a slow answer for the previous lab cannot fill
   * the list.
   */
  async function refresh(forProfileId: string, forChannelId: string) {
    const requestId = ++requestCounter;
    if (!forProfileId || !forChannelId) {
      clearSelection();
      return;
    }
    try {
      const stored = await invoke<CalibrationPreset[]>('list_calibrations', { profileId: forProfileId, channelId: forChannelId });
      const next = applyCalibrationList(
        { requestId, selection: { profileId: forProfileId, channelId: forChannelId }, stored },
        {
          latestRequestId: requestCounter,
          selection: { profileId, channelId },
          state: { presets: savedCalibrations, selectedId: selectedCalibrationId, active: activeCalibration }
        }
      );
      if (!next) return;
      savedCalibrations = next.presets;
      selectedCalibrationId = next.selectedId;
      activeCalibration = next.active;
    } catch (error) {
      if (requestId === requestCounter) onStatus(`Could not load local calibrations: ${String(error)}`);
    }
  }

  /** A new lab is selected: its calibrations, never the previous lab's. */
  export function loadLab(forProfileId: string, forChannelId: string) {
    clearSelection();
    void refresh(forProfileId, forChannelId);
  }

  /** A calibration was saved in the dialog: applied at once, without waiting for the list. */
  export async function applySaved(calibration: CalibrationPreset) {
    selectedCalibrationId = calibration.calibration_id;
    activeCalibration = calibration;
    savedCalibrations = mergeCalibrationPresets(savedCalibrations, [calibration]);
    channelUnits = { ...channelUnits, [calibration.channel_id]: unitFor(calibration) };
    await refresh(calibration.profile_id, calibration.channel_id);
  }

  async function selectChannel(next: string) {
    channelId = next;
    clearSelection();
    await refresh(profileId, next);
  }

  async function selectCalibration(calibrationId: string) {
    selectedCalibrationId = calibrationId;
    activeCalibration = calibrationById(savedCalibrations, calibrationId);
    if (activeCalibration) {
      // Selecting a calibration explicitly opts into its engineering-unit
      // display and freezes the same preset into the next recording.
      channelUnits = { ...channelUnits, [channelId]: unitFor(activeCalibration) };
    } else if (['mmhg', 'calibrated'].includes(channelUnits[channelId] ?? '')) {
      // "None": the channel's calibrated unit is no longer available.
      channelUnits = { ...channelUnits, [channelId]: 'volts' };
    }
    await refresh(profileId, channelId);
  }

  async function deleteSelected() {
    const preset = activeCalibration;
    if (!preset) return;
    // Deleting cannot be undone; recreating an XGZP fit can mean repeating a recording.
    const approved = await confirm(
      `Delete the saved calibration “${preset.label}” (${preset.output_units})? This cannot be undone. Recordings already made keep their own copy.`,
      { title: 'Delete calibration', kind: 'warning', okLabel: 'Delete calibration', cancelLabel: 'Keep it' }
    );
    if (!approved || activeCalibration?.calibration_id !== preset.calibration_id) return;
    try {
      await invoke('delete_calibration', { calibrationId: preset.calibration_id });
      selectedCalibrationId = '';
      activeCalibration = undefined;
      savedCalibrations = savedCalibrations.filter((calibration) => calibration.calibration_id !== preset.calibration_id);
      if (['mmhg', 'calibrated'].includes(channelUnits[preset.channel_id] ?? '')) channelUnits = { ...channelUnits, [preset.channel_id]: 'volts' };
      await refresh(preset.profile_id, preset.channel_id);
      onStatus(`Deleted the local calibration “${preset.label}”. Existing recording snapshots are unchanged.`);
    } catch (error) {
      onStatus(`Could not delete calibration: ${String(error)}`);
    }
  }

  /** Display-only: a unit the channel offers. */
  function setUnit(forChannelId: string, unit: string) {
    if (!(channelUnitOptions[forChannelId] ?? ['counts', 'volts']).includes(unit as DisplayUnit)) {
      onStatus(`${forChannelId} does not have that calibrated unit available.`);
      return;
    }
    channelUnits = { ...channelUnits, [forChannelId]: unit as DisplayUnit };
  }
</script>

<div class="studio-form">
  <p class="help">Raw ADC counts remain authoritative in BMEG. These settings control only live display and derived CSV columns, and are frozen into each new recording’s metadata.</p>
  <div class="control-grid">
    <label>ADC reference voltage (V)<input id="adc-reference-input" type="number" min="0.1" max="10" step="0.001" bind:value={adcReferenceV} disabled={locked} aria-invalid={Boolean(adcReferenceError)} aria-describedby={adcReferenceError ? 'adc-reference-error' : undefined} /></label>
    {#if hasMpxvChannel}
      <label>MPXV sensor supply (Vs, V)<input id="mpxv-supply-input" type="number" min="0.1" max="10" step="0.001" bind:value={mpxvSensorSupplyV} disabled={locked} aria-invalid={Boolean(mpxvSupplyError)} aria-describedby={mpxvSupplyError ? 'mpxv-supply-error' : undefined} /></label>
    {/if}
    {#if calibratableChannels.length}
      <label>Linear calibration channel
        <select value={channelId} onchange={(event) => void selectChannel(event.currentTarget.value)} disabled={locked}>
          {#each calibratableChannels as channel}<option value={channel.id}>{channel.label}</option>{/each}
        </select>
      </label>
      <label>{xgzp ? 'XGZP calibration' : 'Channel calibration'}
        <select value={selectedCalibrationId} onchange={(event) => void selectCalibration(event.currentTarget.value)} disabled={locked}>
          <option value="">None — counts/volts only</option>
          {#each savedCalibrations as calibration}<option value={calibration.calibration_id}>{calibration.label} ({calibration.output_units})</option>{/each}
        </select>
      </label>
    {/if}
  </div>
  {#if calibratableChannels.length}
    <p class="help">Active {xgzp ? 'XGZP' : 'channel'} calibration: {activeCalibration?.label ?? 'none — counts/volts only'}.</p>
    <!-- Delete is destructive and asks first; it sits apart from Calibrate, styled as a danger action. -->
    <div class="calibration-actions">
      <button type="button" onclick={onCalibrate} disabled={locked}>Calibrate {xgzp ? 'XGZP' : 'channel'}</button>
      <button type="button" class="danger" onclick={() => void deleteSelected()} disabled={locked || !activeCalibration}>Delete calibration</button>
    </div>
  {/if}
  {#if adcReferenceError}<p id="adc-reference-error" class="error">{adcReferenceError} The display and the next recording keep {appliedAdcReferenceV} V until it is corrected.</p>{/if}
  {#if hasMpxvChannel && mpxvSupplyError}<p id="mpxv-supply-error" class="error">{mpxvSupplyError} The display and the next recording keep {appliedMpxvSupplyV} V until it is corrected.</p>{/if}
  <div class="unit-grid" role="group" aria-label="Per-channel display units">
    {#each plotChannels as channel}
      <label>{channel.label}
        <select value={channelUnits[channel.id] ?? 'counts'} onchange={(event) => setUnit(channel.id, event.currentTarget.value)}>
          {#each channelUnitOptions[channel.id] ?? ['counts', 'volts'] as unit}<option value={unit}>{displayUnitLabel(unit, recordingCalibration, channel.id)}</option>{/each}
        </select>
      </label>
    {/each}
  </div>
  {#if hasMpxvChannel}<p class="help">MPXV uses P<sub>kPa</sub> = (Vout / Vs − 0.04) / 0.009 and P<sub>mmHg</sub> = 7.5006 × P<sub>kPa</sub>. A channel configured for generic linear calibration offers mmHg only after you save its local fit.</p>{/if}
  {#if category === 'course_emg_force'}<p class="help">The A3 conversion is labeled <strong>Pressure (kPa)</strong>; it does not infer muscular force.</p>{/if}
  {#if pulseox}<p class="help">Pulse-ox units apply directly to the eight raw LED-state values. No background subtraction or optical processing is applied.</p>{/if}
</div>

<style>
  .calibration-actions { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: .6rem 2rem; margin-top: .7rem; padding-top: .7rem; border-top: 1px dashed var(--border); }
  .unit-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 13rem), 1fr)); gap: .65rem; margin-top: .75rem; }
</style>
