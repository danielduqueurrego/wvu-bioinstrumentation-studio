<script lang="ts">
  import { storageEstimateText, storageThresholdText, type RecordingStorageEstimate } from '$lib/storage';

  /**
   * The Recording duration card: Timed (a preset or a custom number of
   * seconds) or Until stopped, limited to the modes the lab allows, and the
   * storage the planned recording needs. The page owns the values and
   * enforces the same rules before Start; Rust enforces them again.
   */
  type Props = {
    durationMode: 'timed' | 'until_stopped';
    durationPreset: string;
    customSeconds: number;
    allowedModes: Array<'timed' | 'until_stopped'>;
    presets: number[];
    minimumCustomSeconds: number;
    timedDurationValid: boolean;
    storageEstimate?: RecordingStorageEstimate;
    storageShortfall?: string;
    locked: boolean;
  };

  let {
    durationMode = $bindable(),
    durationPreset = $bindable(),
    customSeconds = $bindable(),
    allowedModes,
    presets,
    minimumCustomSeconds,
    timedDurationValid,
    storageEstimate,
    storageShortfall,
    locked
  }: Props = $props();

  const presetLabel = (seconds: number) => seconds >= 60 ? `${seconds / 60} minute${seconds === 60 ? '' : 's'}` : `${seconds} seconds`;
</script>

<div class="studio-form">
  <fieldset disabled={locked}>
    <legend>Duration mode</legend>
    <div class="choice-row">
      {#if allowedModes.includes('timed')}<label class="choice"><input id="duration-mode-timed" type="radio" bind:group={durationMode} value="timed" /> Timed</label>{/if}
      {#if allowedModes.includes('until_stopped')}<label class="choice"><input id="duration-mode-until-stopped" type="radio" bind:group={durationMode} value="until_stopped" /> Until stopped</label>{/if}
    </div>
    {#if allowedModes.length === 1}<p class="help">This lab allows only {allowedModes[0] === 'timed' ? 'timed' : 'Until-stopped'} recordings.</p>{/if}
  </fieldset>
  {#if durationMode === 'timed'}
    <div class="duration-controls">
      <label>Timed preset
        <select id="duration-preset-select" bind:value={durationPreset} disabled={locked}>
          {#each presets as preset}<option value={String(preset)}>{presetLabel(preset)}</option>{/each}<option value="custom">Custom</option>
        </select>
      </label>
      {#if durationPreset === 'custom'}
        <label>Custom duration (seconds)
          <input id="duration-custom-input" type="number" min={minimumCustomSeconds} step="1" bind:value={customSeconds} disabled={locked} aria-invalid={!timedDurationValid} aria-describedby={timedDurationValid ? undefined : 'duration-error'} />
        </label>
      {/if}
      {#if !timedDurationValid}<p id="duration-error" class="error">Timed recordings must be whole seconds and at least {minimumCustomSeconds} seconds.</p>{/if}
    </div>
  {:else}
    <p class="help">Recording continues without an automatic time limit. Press <strong>Stop recording</strong> to finish it.</p>
  {/if}
  <p class="help">{storageEstimateText(storageEstimate, durationMode)} {storageThresholdText(storageEstimate)}</p>
  {#if storageShortfall}<p class="warning">{storageShortfall}</p>{/if}
</div>
