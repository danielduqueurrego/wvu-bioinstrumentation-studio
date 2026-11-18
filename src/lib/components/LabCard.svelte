<script lang="ts">
  import LabManager from './LabManager.svelte';
  import OperatingModeControl from './OperatingModeControl.svelte';
  import type { LabProfile } from '$lib/labs';
  import type { OperatingMode } from '$lib/operating-mode';

  /**
   * The Lab card: Student/Instructor mode, Lab Manager (Instructor mode),
   * the lab list, the lab's channels and rate, and the bench-only notice a
   * student confirms for ECG and EMG. The page owns every value and applies
   * a lab change itself (`onSelectLab`), so it can reload that lab's
   * calibrations in the right order.
   */
  type Props = {
    operatingMode: OperatingMode;
    instructorAcknowledgement: boolean;
    benchNoticeAcknowledged: boolean;
    selectedProfileId: string;
    profiles: LabProfile[];
    profile?: LabProfile;
    channels: Array<{ pin: string; label: string }>;
    pulseox: boolean;
    rateText: string;
    catalogNotice?: string;
    /** A recording runs or a Start is being sent. */
    locked: boolean;
    modeChangeInFlight: boolean;
    onSelectLab: (profileId: string) => void;
    onModeConfirmed: (mode: OperatingMode) => void;
    onInstructorBlocked: () => void;
    onLabSaved: (profile: LabProfile) => void;
    onCatalogChanged: () => Promise<void> | void;
    onStatus: (message: string) => void;
  };

  let {
    operatingMode = $bindable(),
    instructorAcknowledgement = $bindable(),
    benchNoticeAcknowledged = $bindable(),
    selectedProfileId = $bindable(),
    profiles,
    profile,
    channels,
    pulseox,
    rateText,
    catalogNotice,
    locked,
    modeChangeInFlight,
    onSelectLab,
    onModeConfirmed,
    onInstructorBlocked,
    onLabSaved,
    onCatalogChanged,
    onStatus
  }: Props = $props();

  const instructor = $derived(operatingMode === 'instructor_authoring');
  // Mirrors the backend's legacy-category gate (`requires_bench_acknowledgement`).
  const benchOnly = $derived(Boolean(profile?.safety.bench_only && ['ecg', 'emg'].includes(profile.category)));
</script>

<div class="studio-form">
  <div class="panel-heading"><p class="help">Choose the assigned course lab. Its required channels and recording settings are applied automatically.</p><span class:locked={!instructor} class="mode-badge">{instructor ? 'Instructor mode' : 'Student mode'}</span></div>
  <OperatingModeControl bind:operatingMode bind:instructorAcknowledgement disabled={locked || modeChangeInFlight} {onModeConfirmed} {onInstructorBlocked} />
  {#if catalogNotice && instructor}<p class="warning">{catalogNotice}</p>{/if}
</div>
{#if instructor}
  <LabManager selectedProfile={profile} disabled={locked} onSaved={onLabSaved} {onCatalogChanged} {onStatus} />
{/if}
<div class="studio-form">
  <label>Selected lab
    <select id="lab-select" bind:value={selectedProfileId} onchange={(event) => onSelectLab(event.currentTarget.value)} disabled={locked}>
      {#each profiles as option}<option value={option.profile_id}>{option.display_name} — {option.profile_version}</option>{/each}
    </select>
  </label>
  {#if profile}
    <div class="profile-details">
      <span><strong>Channels</strong>{channels.map((channel) => channel.label).join('; ')}</span>
      <span><strong>{pulseox ? 'Cycle rate' : 'Sample rate'}</strong>{rateText}</span>
      <span><strong>ADC</strong>{profile.acquisition.adc_resolution_bits} bit</span>
      <span><strong>Units</strong>ADC counts and Arduino input volts</span>
    </div>
    <details class="advanced-details" open={instructor}>
      <summary>Lab details</summary>
      <div class="profile-details">
        <span><strong>Pin mapping</strong>{channels.map((channel) => `${channel.pin} = ${channel.label}`).join('; ')}</span>
        {#if profile.acquisition.acquisition_mode === 'pulseox_4state'}<span><strong>Pulse-ox sequence</strong>RED → DARK → IR → DARK; {profile.acquisition.state_dwell_us} µs per state</span>{/if}
        {#if instructor}<span><strong>Instructor reference</strong>{profile.profile_id} / {profile.profile_version}</span>{/if}
      </div>
    </details>
    {#if instructor && profile.safety.notices.length}
      <details class="advanced-details"><summary>Safety and lab notes</summary>{#each profile.safety.notices as notice}<p class="help profile-notice">{notice}</p>{/each}</details>
    {/if}
    {#if benchOnly}
      <label class="acknowledgement"><input id="bench-acknowledgement" type="checkbox" bind:checked={benchNoticeAcknowledged} disabled={locked} /> I acknowledge this {profile.category.toUpperCase()} profile is bench-validation only. No human-connected recording is authorized.</label>
    {/if}
  {:else}
    <p class="error">No course lab is available. Restart the application or contact your instructor.</p>
  {/if}
</div>

<style>
  .panel-heading { display: flex; min-width: 0; flex-wrap: wrap; justify-content: space-between; gap: .75rem; align-items: start; }
  .panel-heading .help { max-width: 75ch; margin: 0; }
  .mode-badge { border: 1px solid #855a00; border-radius: 999px; padding: .35rem .6rem; background: #fff4dd; color: #684600; font-weight: 700; white-space: nowrap; }
  .mode-badge.locked { border-color: var(--ok); background: var(--ok-bg); color: #174f27; }
  .profile-details { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 16rem), 1fr)); gap: .55rem; margin-top: .8rem; }
  .profile-details span { min-width: 0; padding: .6rem .7rem; border: 1px solid var(--border); border-radius: .3rem; background: #f9fbfc; overflow-wrap: anywhere; }
  .profile-details strong { display: block; color: #42515d; font-size: .78rem; text-transform: uppercase; letter-spacing: .03em; }
  .profile-notice { margin: .6rem 0 0; }
  .acknowledgement { display: flex; gap: .55rem; align-items: flex-start; margin-top: .75rem; padding: .7rem; background: #fff4dd; border: 1px solid var(--warn); font-weight: 700; }
  .acknowledgement input { width: auto; min-height: auto; margin-top: .2rem; }
</style>
