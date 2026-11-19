<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  // Not the browser's confirm: plugin-dialog's is async and must be awaited.
  import { confirm, open } from '@tauri-apps/plugin-dialog';
  import LabManager from './LabManager.svelte';
  import OperatingModeControl from './OperatingModeControl.svelte';
  import { installConfirmation, installOutcomeText, type LabFileSummary } from '$lib/lab-catalog';
  import type { LabProfile } from '$lib/labs';
  import type { OperatingMode } from '$lib/operating-mode';

  /**
   * The Lab card: Student/Instructor mode, Lab Manager (Instructor mode),
   * the lab list, the lab's channels and rate, and the bench-only notice a
   * student confirms for ECG and EMG. The page owns every value and applies
   * a lab change itself (`onSelectLab`), so it can reload that lab's
   * calibrations in the right order. **Install lab file…** (both modes)
   * installs a lab file from the instructor and selects it (`onLabSaved`).
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
    /** A startup catalog migration's notice, shown in both modes. */
    migrationNotice?: string;
    /** A recording runs or a Start is being sent. */
    locked: boolean;
    modeChangeInFlight: boolean;
    onSelectLab: (profileId: string) => void;
    onModeConfirmed: (mode: OperatingMode) => void;
    onInstructorBlocked: () => void;
    /** A revision became active (Lab Manager, Install lab file): reload the list and select it. */
    onLabSaved: (profile: LabProfile) => Promise<void> | void;
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
    migrationNotice,
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
  let installing = $state(false);
  /** The last Install lab file result; the message center clears its own copy after a while. */
  let installMessage = $state('');
  let installFailed = $state(false);

  function reportInstall(message: string, failed = false) {
    installMessage = message;
    installFailed = failed;
    onStatus(message);
  }

  /**
   * Install lab file: choose a file, show what it holds, ask (a warning when
   * it replaces a lab), then install it. Rust applies the same rules again
   * and refuses while a recording runs.
   */
  async function installLabFile() {
    if (locked || installing) return;
    installing = true;
    installMessage = '';
    try {
      const source = await open({ multiple: false, filters: [{ name: 'BMEG lab file', extensions: ['json'] }] });
      if (typeof source !== 'string') return;
      const summary = await invoke<LabFileSummary>('inspect_lab_file', { source });
      if (summary.refusal) {
        // A version that is already installed and active is simply selected.
        if (summary.already_installed && profiles.some((option) => option.profile_id === summary.profile_id && option.profile_version === summary.profile_version)) {
          onSelectLab(summary.profile_id);
        }
        reportInstall(summary.refusal, !summary.already_installed);
        return;
      }
      const question = installConfirmation(summary);
      let approved = false;
      try {
        approved = await confirm(question.message, { title: 'Install lab file', kind: question.kind, okLabel: question.okLabel, cancelLabel: 'Cancel' });
      } catch (reason) {
        reportInstall(`Could not ask for confirmation, so nothing was installed: ${String(reason)}`, true);
        return;
      }
      if (!approved) return;
      // A recording may have started while the question was open.
      if (locked) {
        reportInstall('A recording is running, so the lab file was not installed. Install it after the recording has finished.', true);
        return;
      }
      const installed = await invoke<LabProfile>('install_lab_file', { source });
      await onLabSaved(installed);
      reportInstall(installOutcomeText(installed, summary.replaces));
    } catch (reason) {
      reportInstall(`The lab file was not installed: ${String(reason)}`, true);
    } finally {
      installing = false;
    }
  }
  // Mirrors the backend's legacy-category gate (`requires_bench_acknowledgement`).
  const benchOnly = $derived(Boolean(profile?.safety.bench_only && ['ecg', 'emg'].includes(profile.category)));
</script>

<div class="studio-form">
  <div class="panel-heading"><p class="help">Choose the assigned course lab. Its required channels and recording settings are applied automatically.</p><span class:locked={!instructor} class="mode-badge">{instructor ? 'Instructor mode' : 'Student mode'}</span></div>
  <OperatingModeControl bind:operatingMode bind:instructorAcknowledgement disabled={locked || modeChangeInFlight} {onModeConfirmed} {onInstructorBlocked} />
  {#if catalogNotice && instructor}<p class="warning">{catalogNotice}</p>{/if}
  {#if migrationNotice}<p class="migration-notice" role="note">{migrationNotice}</p>{/if}
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
  <div class="install-row">
    <button type="button" id="install-lab-file" onclick={installLabFile} disabled={locked || installing}>{installing ? 'Installing…' : 'Install lab file…'}</button>
    <span class="help">{locked ? 'Lab files can be installed when no recording is running.' : 'Adds a lab file from your instructor to this computer.'}</span>
  </div>
  {#if installMessage}<p class="install-result" class:failed={installFailed}>{installMessage}</p>{/if}
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
  .migration-notice { max-width: 75ch; margin: .75rem 0 0; padding: .6rem .75rem; border-left: 4px solid var(--wvu-navy, #002855); background: var(--surface-muted, #f2f5f8); overflow-wrap: anywhere; }
  .install-row { display: flex; flex-wrap: wrap; gap: .4rem .75rem; align-items: center; margin-top: .6rem; }
  .install-row .help { margin: 0; }
  .install-row button { min-height: 2.4rem; border: 1px solid #002855; border-radius: .3rem; background: #fff; color: #002855; padding: .45rem .7rem; font: inherit; font-weight: 650; cursor: pointer; }
  .install-row button:disabled { cursor: not-allowed; opacity: .55; }
  .install-row button:focus-visible { outline: 3px solid var(--focus-ring, #002855); outline-offset: 2px; box-shadow: 0 0 0 2px #fff; }
  .install-result { max-width: 75ch; margin: .6rem 0 0; padding: .55rem .75rem; border-left: 4px solid var(--ok, #2e7d32); background: var(--ok-bg, #eef7ef); overflow-wrap: anywhere; }
  .install-result.failed { border-left-color: var(--warn, #b26a00); background: #fff4dd; }
  .acknowledgement { display: flex; gap: .55rem; align-items: flex-start; margin-top: .75rem; padding: .7rem; background: #fff4dd; border: 1px solid var(--warn); font-weight: 700; }
  .acknowledgement input { width: auto; min-height: auto; margin-top: .2rem; }
</style>
