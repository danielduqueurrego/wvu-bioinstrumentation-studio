<script lang="ts">
  import { untrack } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import Modal from './Modal.svelte';
  import { xgzpFitRequestPayload, type CalibrationPreset } from '$lib/calibration';
  import {
    calibrationOutput,
    defaultCalibrationLabel,
    fitVoltageNotice,
    parseCalibrationPoints,
    type CalibrationFitResult
  } from '$lib/calibration-inputs';
  import type { LabProfile } from '$lib/labs';

  /**
   * The Calibrate dialog: a linear fit from a finished Blood Pressure + PPG
   * recording (XGZP against its synchronized MPXV) or from points the student
   * types, then Save. A fit may be saved only while its inputs are
   * unchanged. Saving stores a local preset; raw BMEG samples never change.
   * The page applies the saved preset (`onSaved`).
   */
  type Props = {
    open: boolean;
    profile?: LabProfile;
    channelId: string;
    channelLabel: string;
    /** The last finished recording (`SessionSummary`), for the XGZP fit. */
    recordingPath: string;
    recordingSeconds: number;
    adcReferenceError: string;
    mpxvSupplyError: string;
    appliedAdcReferenceV: number;
    appliedMpxvSupplyV: number;
    locked: boolean;
    /** Awaited after the preset is saved, before the dialog closes. */
    onSaved: (calibration: CalibrationPreset) => Promise<void> | void;
    onStatus: (message: string) => void;
  };

  let {
    open = $bindable(),
    profile,
    channelId,
    channelLabel,
    recordingPath,
    recordingSeconds,
    adcReferenceError,
    mpxvSupplyError,
    appliedAdcReferenceV,
    appliedMpxvSupplyV,
    locked,
    onSaved,
    onStatus
  }: Props = $props();

  let method = $state<'xgzp_recording' | 'manual_points'>('xgzp_recording');
  let startSeconds = $state(0);
  let endSeconds = $state(10);
  // No example values: a calibration is saved only from points the student
  // entered (the example is a placeholder), with the units they entered.
  let label = $state('');
  let manualPoints = $state('');
  let manualQuantity = $state('');
  let manualUnits = $state('');
  // Lab and channel the dialog was last opened for; typed points are kept
  // only when it is reopened for the same channel.
  let dialogChannelKey = '';
  let fit = $state<CalibrationFitResult | undefined>();
  // The inputs the shown fit was calculated from; a changed input needs a new fit.
  let fitInputs = $state('');
  let error = $state('');

  const bpXgzp = $derived(channelId === 'xgzp' && profile?.category === 'course_blood_pressure');
  const fitCurrent = $derived(Boolean(fit) && fitInputs === inputsKey());
  const notice = $derived(fitVoltageNotice(fit));
  const saveOutput = $derived(calibrationOutput(method, manualQuantity, manualUnits));

  // Each time the dialog opens: a fresh fit, the recording's interval, and a
  // label naming this channel.
  let wasOpen = false;
  $effect.pre(() => {
    const opening = open && !wasOpen;
    wasOpen = open;
    if (opening) untrack(prepare);
  });

  function prepare() {
    fit = undefined;
    error = '';
    if (recordingSeconds > 0) {
      startSeconds = 0;
      endSeconds = Math.max(0.1, recordingSeconds);
    }
    if (!bpXgzp) method = 'manual_points';
    const key = `${profile?.profile_id ?? ''}/${channelId}`;
    if (key !== dialogChannelKey) {
      dialogChannelKey = key;
      manualPoints = '';
      manualQuantity = bpXgzp ? 'pressure' : '';
      manualUnits = bpXgzp ? 'mmHg' : '';
    }
    label = defaultCalibrationLabel(channelLabel || 'Channel', new Date());
  }

  /** What a fit is calculated from; any change needs a new fit before Save. */
  function inputsKey(): string {
    const profileId = profile?.profile_id ?? '';
    return JSON.stringify(method === 'manual_points'
      ? [method, profileId, channelId, manualPoints]
      : [method, profileId, channelId, recordingPath, Number(startSeconds), Number(endSeconds), appliedAdcReferenceV, appliedMpxvSupplyV]);
  }

  function close() {
    open = false;
    fit = undefined;
    error = '';
  }

  async function calculate() {
    error = '';
    fit = undefined;
    try {
      const inputs = inputsKey();
      let result: CalibrationFitResult;
      if (method === 'xgzp_recording') {
        if (!recordingPath || profile?.category !== 'course_blood_pressure' || channelId !== 'xgzp') {
          throw new Error('Finish a Blood Pressure + PPG recording before fitting XGZP against its synchronized MPXV channel.');
        }
        // The fit uses the values in the fields; refuse an invalid one rather
        // than silently fitting with the last valid value.
        if (adcReferenceError || mpxvSupplyError) throw new Error(adcReferenceError || mpxvSupplyError);
        result = await invoke<CalibrationFitResult>('fit_xgzp_calibration', {
          request: xgzpFitRequestPayload(recordingPath, Number(startSeconds), Number(endSeconds), appliedAdcReferenceV, appliedMpxvSupplyV)
        });
      } else {
        const parsed = parseCalibrationPoints(manualPoints);
        if (parsed.error !== undefined) throw new Error(parsed.error);
        result = await invoke<CalibrationFitResult>('fit_manual_linear_calibration', { points: parsed.points });
      }
      fit = result;
      fitInputs = inputs;
      onStatus(`Linear calibration fit: ${result.paired_samples} paired samples; R² ${result.r_squared.toFixed(4)}. Review it before saving.`);
    } catch (reason) {
      fit = undefined;
      error = `Calibration fit error: ${reason instanceof Error ? reason.message : String(reason)}`;
      onStatus(error);
    }
  }

  async function save() {
    if (!profile || !fit || !channelId) return;
    if (fitInputs !== inputsKey()) {
      error = 'The inputs changed after this fit was calculated. Select Calculate linear fit again before saving.';
      return;
    }
    const output = calibrationOutput(method, manualQuantity, manualUnits);
    if (output.error !== undefined) {
      error = output.error;
      return;
    }
    const trimmed = label.trim();
    if (!trimmed) {
      error = 'Enter a calibration label.';
      return;
    }
    error = '';
    // A fit from a recording records which Vref and Vs produced it.
    const provenance: Record<string, number> = fit.adc_reference_v !== undefined && fit.mpxv_sensor_supply_v !== undefined
      ? { fit_adc_reference_v: fit.adc_reference_v, fit_mpxv_sensor_supply_v: fit.mpxv_sensor_supply_v }
      : {};
    const calibration: CalibrationPreset = {
      schema_version: 1,
      calibration_id: `${profile.profile_id}.${channelId}.${Date.now()}`.replace(/[^a-zA-Z0-9._-]/g, '_').toLowerCase(),
      profile_id: profile.profile_id,
      channel_id: channelId,
      calibration_type: 'linear',
      input_quantity: 'volts',
      output_quantity: output.quantity,
      output_units: output.units,
      parameters: { slope: fit.slope, offset: fit.offset, ...provenance },
      created_at: new Date().toISOString(),
      label: trimmed
    };
    try {
      await invoke<CalibrationPreset>('save_calibration', { calibration });
      await onSaved(calibration);
      open = false;
      onStatus(`Saved local calibration “${calibration.label}” (${calibration.output_quantity}, ${calibration.output_units}). Raw BMEG samples remain unchanged; the calibration will be snapshotted only in future recordings.`);
    } catch (reason) {
      onStatus(`Could not save calibration: ${String(reason)}`);
    }
  }
</script>

<Modal {open} labelledBy="calibration-dialog-title" onClose={close}>
  <div class="studio-form">
    <div class="dialog-heading"><h2 id="calibration-dialog-title">Calibrate {channelLabel || 'channel'}</h2><button type="button" onclick={close}>Close</button></div>
    <p class="warning">Course calibration only. This fits engineering pressure units from your selected data; it does not make a blood-pressure determination or clinical conclusion.</p>
    <fieldset disabled={locked}>
      <legend>Calibration method</legend>
      <div class="choice-row">{#if bpXgzp}<label class="choice"><input type="radio" bind:group={method} value="xgzp_recording" /> Use completed synchronized BP recording</label>{/if}<label class="choice"><input type="radio" bind:group={method} value="manual_points" /> Enter manual linear points</label></div>
    </fieldset>
    {#if method === 'xgzp_recording'}
      <p class="help">The completed recording provides A1 MPXV as the reference and A2 XGZP as the fitted input. Select a stable interval in seconds.</p>
      <div class="control-grid"><label>Start (seconds)<input type="number" min="0" step="0.1" bind:value={startSeconds} /></label><label>End (seconds)<input type="number" min="0.1" step="0.1" bind:value={endSeconds} /></label></div>
    {:else}
      <label>Manual points: channel volts, reference engineering value (one pair per line)<textarea rows="5" bind:value={manualPoints} spellcheck="false" placeholder={'0.80, 20\n1.20, 60\n1.60, 100'}></textarea></label>
      <label>Output quantity<input bind:value={manualQuantity} placeholder="for example pressure" /></label>
      <label>Output units<input bind:value={manualUnits} placeholder="for example mmHg" /></label>
    {/if}
    <div class="action-row"><button type="button" class="gold" onclick={calculate}>Calculate linear fit</button></div>
    {#if error}<p class="error">{error}</p>{/if}
    {#if fit}
      <section class="metric-grid" aria-label="Calibration fit"><span><strong>Slope</strong>{fit.slope.toFixed(6)}</span><span><strong>Offset</strong>{fit.offset.toFixed(6)}</span><span><strong>R²</strong>{fit.r_squared.toFixed(6)} (informational)</span><span><strong>Paired samples</strong>{fit.paired_samples}</span></section>
      {#if notice}<p class="help">{notice.used}</p>{#if notice.warning}<p class="warning">{notice.warning}</p>{/if}{/if}
      {#if fitCurrent}
        <p class="help">{saveOutput.error === undefined ? `Saves as ${saveOutput.quantity} in ${saveOutput.units}.` : saveOutput.error}</p>
        <label>Calibration label<input bind:value={label} maxlength="80" /></label>
        <div class="action-row"><button type="button" class="gold" onclick={save} disabled={saveOutput.error !== undefined || !label.trim()}>Save calibration</button></div>
      {:else}
        <p class="warning">The inputs changed after this fit was calculated. Select Calculate linear fit again before saving.</p>
      {/if}
    {/if}
  </div>
</Modal>

<style>
  .dialog-heading { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: .7rem; }
  .dialog-heading h2 { margin: 0; }
</style>
