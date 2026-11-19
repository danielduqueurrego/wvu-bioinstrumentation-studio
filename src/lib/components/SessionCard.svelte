<script lang="ts">
  import { labRateShortText } from '$lib/lab-authoring';
  import type { LabProfile } from '$lib/labs';

  /**
   * The Session setup card: the Output folder (relative to the Project
   * folder) and an optional recording note, beside the lab's protected
   * pins, ADC resolution, and rate (read-only; the lab sets them).
   */
  type Props = {
    outputDirectory: string;
    note: string;
    profile?: LabProfile;
    channels: Array<{ pin: string }>;
    /** "Sample rate", "Cycle rate" (legacy 4-state pulse-ox), or "Record rate" (LED-sequenced). */
    rateLabel: string;
    rateText: string;
    effectiveOutputFolder: string;
    outputFolderError?: string;
    locked: boolean;
  };

  let {
    outputDirectory = $bindable(),
    note = $bindable(),
    profile,
    channels,
    rateLabel,
    rateText,
    effectiveOutputFolder,
    outputFolderError,
    locked
  }: Props = $props();

  const pins = $derived(!profile
    ? '—'
    : profile.acquisition.acquisition_mode === 'pulseox_4state'
      ? `TX ${profile.acquisition.analog_inputs?.tx ?? 'A0'}; RX ${profile.acquisition.analog_inputs?.rx ?? 'A1'}`
      : channels.map((channel) => channel.pin).join(', '));
  // The field holds the rate and unit only, so it fits its column; the full
  // text ("… (one per LED state)") is its tooltip and the card's summary.
  const rateValue = $derived(profile ? labRateShortText(profile.acquisition) : rateText);
</script>

<div class="studio-form">
  <div class="control-grid">
    <label>Output folder <input id="output-folder-input" bind:value={outputDirectory} placeholder="Participant01\Trial03" disabled={locked} aria-invalid={Boolean(outputFolderError)} aria-describedby={outputFolderError ? 'output-help output-folder-error' : 'output-help'} /></label>
    <label>Analog pins <input value={pins} readonly title="Protected by the selected profile" /></label>
    <label>ADC resolution <input value={profile ? `${profile.acquisition.adc_resolution_bits} bit` : '—'} readonly title="Protected by the selected profile" /></label>
    <label>{rateLabel} <input value={rateValue || '—'} readonly title={rateText && rateText !== rateValue ? `${rateText}. Protected by the selected profile` : 'Protected by the selected profile'} /></label>
    <label>Recording note <input bind:value={note} maxlength="4096" placeholder="Optional note for this recording" disabled={locked} /></label>
  </div>
  <p id="output-help" class="help">Effective destination: <strong>{effectiveOutputFolder || 'Choose a Project folder'}</strong>. Output folder is relative to Project folder; nested trial folders are allowed. Files use timestamped names and existing files are never overwritten.</p>
  {#if outputFolderError}<p id="output-folder-error" class="error">{outputFolderError}</p>{/if}
</div>
