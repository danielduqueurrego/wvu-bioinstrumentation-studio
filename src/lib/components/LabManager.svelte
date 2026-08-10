<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { open, save } from '@tauri-apps/plugin-dialog';
  import { labConfigurationIssues, pulseOxDraft } from '$lib/lab-authoring';
  import { beginEditSession, beginNewLabSession, labSourceLabel, saveArguments } from '$lib/lab-catalog';
  import type { LabChannel as Channel, LabDigitalOutput as DigitalOutput, LabListEntry as LabEntry, LabPlotGroup as PlotGroup, LabProfile } from '$lib/labs';

  export let selectedProfile: LabProfile | undefined;
  export let onSaved: (profile: LabProfile) => void;
  export let onStatus: (message: string) => void;
  export let openFirmware: () => void;

  const analogPins = ['A0', 'A1', 'A2', 'A3', 'A4', 'A5'];
  const digitalPins = ['D4', 'D5', 'D6'];
  const rates = [100, 200, 250, 500, 1000];
  let visible = false;
  let labs: LabEntry[] = [];
  let draft: LabProfile | undefined;
  let draftBaseVersion: string | undefined;
  let draftSaveRequestId = '';
  let saving = false;
  let duplicateId = '';
  let error = '';
  let loading = false;

  $: simultaneous = draft?.acquisition.acquisition_mode === 'simultaneous';
  $: pulseox = draft?.acquisition.acquisition_mode === 'pulseox_4state';
  $: draftChannels = draft?.acquisition.channels ?? [];
  $: outputSummary = draft?.acquisition.digital_outputs?.map((output) => `${output.pin} ${output.label} — ${output.behavior.replaceAll('_', ' ')}`).join('; ') || 'None';
  $: estimatedPulseRate = draft?.acquisition.state_dwell_us ? (1_000_000 / (4 * Number(draft.acquisition.state_dwell_us))).toFixed(1) : '—';

  async function refresh() {
    loading = true;
    error = '';
    try {
      labs = await invoke<LabEntry[]>('list_instructor_labs');
    } catch (reason) {
      error = `Could not load lab history: ${String(reason)}`;
    } finally {
      loading = false;
    }
  }

  async function openManager() {
    visible = true;
    discardDraft();
    await refresh();
  }

  function newRequestId() {
    return globalThis.crypto?.randomUUID?.() ?? `save-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  }

  function discardDraft() {
    draft = undefined;
    draftBaseVersion = undefined;
    draftSaveRequestId = '';
    saving = false;
  }

  function closeManager() {
    if (draft && !window.confirm('Discard unsaved lab changes? No lab version has been created.')) return;
    visible = false;
    discardDraft();
  }

  async function editLab(profileId = selectedProfile?.profile_id) {
    if (!profileId) return;
    error = '';
    try {
      draft = await invoke<LabProfile>('begin_lab_edit', { profileId });
      const session = beginEditSession(draft, newRequestId());
      draftBaseVersion = session.baseVersion;
      draftSaveRequestId = session.requestId;
      ensureDraftDefaults();
    } catch (reason) { error = String(reason); }
  }

  async function duplicateLab() {
    if (!selectedProfile || !duplicateId.trim()) {
      error = 'Enter a unique lowercase lab ID before duplicating.';
      return;
    }
    try {
      draft = await invoke<LabProfile>('duplicate_lab', { profileId: selectedProfile.profile_id, labId: duplicateId.trim() });
      const session = beginNewLabSession(draft, newRequestId());
      draftBaseVersion = session.baseVersion;
      draftSaveRequestId = session.requestId;
      ensureDraftDefaults();
      duplicateId = '';
    } catch (reason) { error = String(reason); }
  }

  async function createBlankLab() {
    if (!duplicateId.trim()) {
      error = 'Enter a unique lowercase lab ID before creating a blank lab.';
      return;
    }
    try {
      draft = await invoke<LabProfile>('create_blank_simultaneous_lab', { labId: duplicateId.trim() });
      const session = beginNewLabSession(draft, newRequestId());
      draftBaseVersion = session.baseVersion;
      draftSaveRequestId = session.requestId;
      ensureDraftDefaults();
      duplicateId = '';
    } catch (reason) { error = String(reason); }
  }

  function ensureDraftDefaults() {
    if (!draft) return;
    const channels = draft.acquisition.channels?.length
      ? draft.acquisition.channels
      : [{ pin: draft.acquisition.analog_pin, id: 'channel_1', label: 'Channel 1', csv_name: 'channel_1_counts', units: 'ADC counts', allowed_conversions: ['counts_volts'], default_display_unit: 'counts', default_visible: true }];
    draft = {
      ...draft,
      acquisition: { ...draft.acquisition, channels, digital_outputs: draft.acquisition.digital_outputs ?? [] },
      plot_defaults: draft.plot_defaults ?? { groups: channels.map((channel) => ({ channel_ids: [channel.id] })) },
      associated_sketch: draft.associated_sketch ?? { name: 'WVU Reference Firmware', is_wvu_reference: true }
    };
  }

  function updateChannels(channels: Channel[]) {
    if (!draft) return;
    draft = { ...draft, acquisition: { ...draft.acquisition, analog_pin: channels[0]?.pin ?? 'A0', channels }, plot_defaults: { groups: normalizeGroups(channels, draft.plot_defaults?.groups ?? []) } };
  }

  function addChannel() {
    if (!draft || draftChannels.length >= 6) return;
    const pin = analogPins.find((candidate) => !draftChannels.some((channel) => channel.pin === candidate)) ?? 'A0';
    const index = draftChannels.length + 1;
    updateChannels([...draftChannels, { pin, id: `channel_${index}`, label: `Channel ${index}`, csv_name: `channel_${index}_counts`, units: 'ADC counts', allowed_conversions: ['counts_volts'], default_display_unit: 'counts', default_visible: true }]);
  }

  function removeChannel(index: number) {
    if (draftChannels.length <= 1) return;
    updateChannels(draftChannels.filter((_, candidate) => candidate !== index));
  }

  function updateChannel(index: number, field: keyof Channel, value: string | boolean | string[]) {
    updateChannels(draftChannels.map((channel, candidate) => candidate === index ? { ...channel, [field]: value } : channel));
  }

  function normalizeGroups(channels: Channel[], groups: PlotGroup[]): PlotGroup[] {
    const ids = new Set(channels.map((channel) => channel.id));
    const seen = new Set<string>();
    const result = groups.map((group) => ({ channel_ids: group.channel_ids.filter((id) => ids.has(id) && !seen.has(id) && Boolean(seen.add(id))) })).filter((group) => group.channel_ids.length);
    for (const channel of channels) if (!seen.has(channel.id)) result.push({ channel_ids: [channel.id] });
    return result;
  }

  function setPlotNumber(channelId: string, value: string) {
    if (!draft) return;
    const requested = Math.max(1, Math.min(draftChannels.length, Number(value)));
    const groups = normalizeGroups(draftChannels, draft.plot_defaults?.groups ?? []).map((group) => ({ ...group, channel_ids: group.channel_ids.filter((id) => id !== channelId) }));
    while (groups.length < requested) groups.push({ channel_ids: [] });
    groups[requested - 1].channel_ids.push(channelId);
    draft = { ...draft, plot_defaults: { groups: groups.filter((group) => group.channel_ids.length) } };
  }

  function configurePulseox() {
    if (!draft) return;
    draft = pulseOxDraft(draft);
  }

  function configureSimultaneous() {
    if (!draft) return;
    const channels = draftChannels.length ? draftChannels : [{ pin: 'A0', id: 'channel_1', label: 'Channel 1', csv_name: 'channel_1_counts', units: 'ADC counts', allowed_conversions: ['counts_volts'], default_display_unit: 'counts', default_visible: true }];
    draft = { ...draft, acquisition: { ...draft.acquisition, acquisition_mode: 'simultaneous', analog_pin: channels[0].pin, channels, analog_inputs: undefined, state_dwell_us: undefined, digital_outputs: [] }, plot_defaults: { groups: channels.map((channel) => ({ channel_ids: [channel.id] })) } };
  }

  function setPulseInput(kind: 'tx' | 'rx', pin: string) {
    if (!draft) return;
    draft = { ...draft, acquisition: { ...draft.acquisition, analog_inputs: { ...(draft.acquisition.analog_inputs ?? { tx: 'A0', rx: 'A1' }), [kind]: pin } } };
  }

  function setPulseOutput(kind: 'red' | 'ir', pin: string) {
    if (!draft) return;
    const outputs = (draft.acquisition.digital_outputs ?? []).map((output) => output.label.toLowerCase().startsWith(kind) ? { ...output, pin } : output);
    draft = { ...draft, acquisition: { ...draft.acquisition, digital_outputs: outputs, led_outputs: { ...(draft.acquisition.led_outputs ?? {}), [kind]: pin } } };
  }

  function setPulseDwell(value: string) {
    if (!draft) return;
    const state_dwell_us = Number(value);
    draft = {
      ...draft,
      acquisition: {
        ...draft.acquisition,
        state_dwell_us,
        // This is a nominal provenance estimate. Raw timestamps remain the
        // authoritative record of pulse-ox timing.
        sample_rate_hz: Math.max(1, Math.round(1_000_000 / (4 * state_dwell_us)))
      }
    };
  }

  function setSimultaneousOutput(pin: string, enabled: boolean) {
    if (!draft) return;
    const current = draft.acquisition.digital_outputs ?? [];
    const outputs = enabled
      ? [...current.filter((output) => output.pin !== pin), { pin, label: pin === 'D4' ? 'Green LED' : `${pin} output`, behavior: pin === 'D4' ? 'high_while_recording' as const : 'always_low' as const }]
      : current.filter((output) => output.pin !== pin);
    draft = { ...draft, acquisition: { ...draft.acquisition, digital_outputs: outputs } };
  }

  function setSketchKind(value: string) {
    if (!draft) return;
    const isReference = value === 'reference';
    draft = {
      ...draft,
      associated_sketch: {
        ...(draft.associated_sketch ?? { name: 'WVU Reference Firmware', is_wvu_reference: true }),
        is_wvu_reference: isReference,
        name: isReference
          ? 'WVU Reference Firmware'
          : (draft.associated_sketch?.name === 'WVU Reference Firmware' ? 'Custom Arduino sketch' : draft.associated_sketch?.name ?? 'Custom Arduino sketch')
      }
    };
  }

  async function saveDraft() {
    if (!draft || saving) return;
    error = '';
    const issues = labConfigurationIssues(draft);
    if (issues.length) {
      error = issues.join(' ');
      return;
    }
    saving = true;
    try {
      const saved = await invoke<LabProfile>('save_lab_draft', saveArguments({ draft, baseVersion: draftBaseVersion, requestId: draftSaveRequestId }));
      discardDraft();
      await refresh();
      onSaved(saved);
      onStatus(`Saved ${saved.display_name} ${saved.profile_version} as the active locked lab revision.`);
    } catch (reason) { error = `Lab configuration is not valid: ${String(reason)}`; }
    finally { saving = false; }
  }

  async function retire(entry: LabEntry) {
    try {
      await invoke('retire_profile', { profileId: entry.profile.profile_id, profileVersion: entry.profile.profile_version });
      await refresh();
      onStatus(`Retired ${entry.profile.display_name} ${entry.profile.profile_version}. Existing recording snapshots remain unchanged.`);
    } catch (reason) { error = String(reason); }
  }

  async function restore(entry: LabEntry) {
    try {
      const restored = await invoke<LabProfile>('restore_retired_lab', { profileId: entry.profile.profile_id, profileVersion: entry.profile.profile_version });
      await refresh();
      onSaved(restored);
    } catch (reason) { error = String(reason); }
  }

  async function restoreDefault() {
    if (!selectedProfile) return;
    try {
      const restored = await invoke<LabProfile>('restore_course_default_lab', { profileId: selectedProfile.profile_id });
      await refresh();
      onSaved(restored);
      onStatus(`The shipped course default ${restored.profile_version} is active. No new version was created.`);
    } catch (reason) { error = String(reason); }
  }

  async function exportLab(entry: LabEntry) {
    const destination = await save({ defaultPath: `${entry.profile.profile_id}_${entry.profile.profile_version}.lab.json`, filters: [{ name: 'BMEG lab configuration', extensions: ['json'] }] });
    if (!destination) return;
    try {
      await invoke('export_profile_package', { profileId: entry.profile.profile_id, profileVersion: entry.profile.profile_version, destination });
      onStatus(`Exported ${entry.profile.display_name} to ${destination}.`);
    } catch (reason) { error = String(reason); }
  }

  async function importLab() {
    const source = await open({ multiple: false, filters: [{ name: 'BMEG lab configuration', extensions: ['json'] }] });
    if (typeof source !== 'string') return;
    try {
      const imported = await invoke<LabProfile>('import_profile_package', { source });
      await refresh();
      onSaved(imported);
      onStatus(`Imported locked lab ${imported.display_name} ${imported.profile_version}.`);
    } catch (reason) { error = String(reason); }
  }

  async function activate(entry: LabEntry) {
    try {
      const active = await invoke<LabProfile>('set_active_lab_version', { profileId: entry.profile.profile_id, profileVersion: entry.profile.profile_version });
      await refresh();
      onSaved(active);
    } catch (reason) { error = String(reason); }
  }

  async function resetLocalCustomizations() {
    if (!window.confirm('Reset all local instructor/imported lab customizations? Shipped course defaults and existing recording files are preserved.')) return;
    try {
      await invoke('reset_local_lab_customizations');
      await refresh();
      onStatus('Reset local lab customizations. Shipped factory course labs are active. Existing recordings retain their embedded snapshots.');
    } catch (reason) { error = String(reason); }
  }
</script>

<section class="authoring" aria-label="Instructor Lab Editor">
  <div class="manager-heading"><div><h3>Instructor Lab Editor</h3><p class="help">Editing creates a new locked revision. Earlier lab snapshots and recordings are never changed.</p></div><button type="button" onclick={openManager}>Manage Labs</button></div>
</section>

{#if visible}
  <div class="backdrop">
    <dialog open class="lab-dialog" aria-modal="true" aria-labelledby="lab-manager-title">
      <div class="manager-heading"><div><h2 id="lab-manager-title">Manage Labs</h2><p class="help">Instructor mode is a local workflow guard. A saved lab must be supported by the connected WVU firmware before recording.</p></div><button type="button" onclick={closeManager}>Close</button></div>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      {#if !draft}
        <div class="actions"><button type="button" class="gold" onclick={() => editLab()}>Edit selected lab</button><label>New/duplicate lab ID<input bind:value={duplicateId} placeholder="team2.custom.emg" /></label><button type="button" onclick={duplicateLab}>Duplicate selected lab</button><button type="button" onclick={createBlankLab}>Blank simultaneous lab</button><button type="button" onclick={importLab}>Import lab</button><button type="button" onclick={restoreDefault} disabled={!selectedProfile}>Restore course default</button><button type="button" class="danger" onclick={resetLocalCustomizations}>Reset local customizations</button></div>
        <section class="history" aria-label="Lab version history">
          <h3>Lab versions</h3>
          {#if loading}<p>Loading local lab history…</p>{:else}
            <div class="history-table"><div class="history-head"><span>Name</span><span>Version</span><span>Source / status</span><span>Configuration</span><span>Actions</span></div>{#each labs as entry}<div class="history-row"><span title={entry.profile.profile_id}><strong>{entry.profile.display_name}</strong><small>{entry.profile.profile_id}</small></span><span>{entry.profile.profile_version}</span><span>{labSourceLabel(entry.profile.source)} · {entry.retired ? 'Retired' : entry.active ? 'Active' : 'Historical'}</span><span>{entry.profile.acquisition.acquisition_mode === 'pulseox_4state' ? 'Pulse oximetry — 4-state' : `${entry.profile.acquisition.channels?.length ?? 1} analog channel(s)`}; {entry.profile.acquisition.sample_rate_hz} Hz; {entry.profile.acquisition.adc_resolution_bits} bit</span><span class="row-actions"><button type="button" onclick={() => editLab(entry.profile.profile_id)} disabled={entry.retired}>Edit</button><button type="button" onclick={() => exportLab(entry)}>Export</button>{#if entry.retired}<button type="button" onclick={() => restore(entry)}>Restore</button>{:else if !entry.active}<button type="button" onclick={() => activate(entry)}>{entry.profile.source === 'built_in' ? 'Use factory default' : 'Make active'}</button>{/if}{#if entry.profile.source !== 'built_in' && !entry.retired}<button type="button" onclick={() => retire(entry)}>Retire</button>{/if}</span></div>{/each}</div>
          {/if}
        </section>
      {:else}
        <p class="warning">This is an in-memory draft based on <strong>{draft.profile_id} {draftBaseVersion ?? 'new lab'}</strong>. Only <strong>Save changes</strong> creates one new immutable active version. Firmware upload is never automatic.</p>
        <section class="editor-grid"><label>Lab name<input bind:value={draft.display_name} /></label><label>Lab ID<input value={draft.profile_id} readonly /></label><label>Description<textarea bind:value={draft.description} rows="3"></textarea></label><label>Acquisition mode<select value={draft.acquisition.acquisition_mode} onchange={(event) => event.currentTarget.value === 'pulseox_4state' ? configurePulseox() : configureSimultaneous()}><option value="simultaneous">Simultaneous Analog</option><option value="pulseox_4state">Pulse Oximetry — 4-State</option></select></label>{#if simultaneous}<label>Frame rate<select bind:value={draft.acquisition.sample_rate_hz}>{#each rates as rate}<option value={rate}>{rate} Hz</option>{/each}</select></label>{:else}<span class="estimate"><strong>Cycle rate (from dwell)</strong>{estimatedPulseRate} cycles/s nominal</span>{/if}<label>ADC resolution<select bind:value={draft.acquisition.adc_resolution_bits}><option value={12}>12 bit</option><option value={14}>14 bit</option></select></label></section>
        {#if simultaneous}
          <section class="subpanel"><div class="manager-heading"><h3>Analog channel editor</h3><button type="button" onclick={addChannel} disabled={draftChannels.length >= 6}>Add channel</button></div><p class="help">Each enabled channel needs a unique pin, ID, and CSV field. WVU firmware supports A0–A5 and 1–6 synchronized channels.</p><div class="channel-grid header"><span>Pin</span><span>Channel ID</span><span>Label</span><span>CSV field</span><span>Calibration capability</span><span>Visible</span><span>Plot</span><span>Remove</span></div>{#each draftChannels as channel, index}<div class="channel-grid"><select value={channel.pin} onchange={(event) => updateChannel(index, 'pin', event.currentTarget.value)}>{#each analogPins as pin}<option value={pin}>{pin}</option>{/each}</select><input value={channel.id} oninput={(event) => updateChannel(index, 'id', event.currentTarget.value)} /><input value={channel.label} oninput={(event) => updateChannel(index, 'label', event.currentTarget.value)} /><input value={channel.csv_name} oninput={(event) => updateChannel(index, 'csv_name', event.currentTarget.value)} /><select value={channel.allowed_conversions?.includes('mpxv_pressure') ? 'mpxv_pressure' : channel.allowed_conversions?.includes('linear_calibration') ? 'linear_calibration' : 'counts_volts'} onchange={(event) => updateChannel(index, 'allowed_conversions', [event.currentTarget.value])}><option value="counts_volts">Counts / Volts only</option><option value="mpxv_pressure">MPXV pressure</option><option value="linear_calibration">Generic linear calibration</option></select><input type="checkbox" checked={channel.default_visible !== false} onchange={(event) => updateChannel(index, 'default_visible', event.currentTarget.checked)} /><select value={String(Math.max(0, (draft.plot_defaults?.groups ?? []).findIndex((group) => group.channel_ids.includes(channel.id))) + 1)} onchange={(event) => setPlotNumber(channel.id, event.currentTarget.value)}>{#each draftChannels as _, groupIndex}<option value={String(groupIndex + 1)}>Plot {groupIndex + 1}</option>{/each}</select><button type="button" onclick={() => removeChannel(index)} disabled={draftChannels.length <= 1}>Remove</button></div>{/each}</section>
          <section class="subpanel"><h3>Digital outputs</h3><p class="help">D4 can be HIGH while recording. D5/D6 stay LOW in simultaneous labs so RED and IR are never accidentally enabled.</p><div class="output-grid">{#each digitalPins as pin}<label class="choice"><input type="checkbox" checked={Boolean(draft.acquisition.digital_outputs?.some((output) => output.pin === pin))} onchange={(event) => setSimultaneousOutput(pin, event.currentTarget.checked)} /> {pin} {pin === 'D4' ? 'Green LED — HIGH while recording' : 'controlled output — Always LOW'}</label>{/each}</div></section>
        {:else if pulseox}
          <section class="subpanel"><h3>Pulse oximetry — fixed 4-state phase</h3><p class="warning">Phase order is fixed: <strong>RED ON → DARK 1 → IR ON → DARK 2</strong>. RED and IR are independently remappable but never HIGH simultaneously.</p><div class="editor-grid"><label>TX analog pin<select value={draft.acquisition.analog_inputs?.tx ?? 'A0'} onchange={(event) => setPulseInput('tx', event.currentTarget.value)}>{#each analogPins as pin}<option value={pin}>{pin}</option>{/each}</select></label><label>RX analog pin<select value={draft.acquisition.analog_inputs?.rx ?? 'A1'} onchange={(event) => setPulseInput('rx', event.currentTarget.value)}>{#each analogPins as pin}<option value={pin}>{pin}</option>{/each}</select></label><label>RED output<select value={draft.acquisition.digital_outputs?.find((output) => output.label === 'Red LED')?.pin ?? 'D5'} onchange={(event) => setPulseOutput('red', event.currentTarget.value)}>{#each digitalPins as pin}<option value={pin}>{pin}</option>{/each}</select></label><label>IR output<select value={draft.acquisition.digital_outputs?.find((output) => output.label === 'IR LED')?.pin ?? 'D6'} onchange={(event) => setPulseOutput('ir', event.currentTarget.value)}>{#each digitalPins as pin}<option value={pin}>{pin}</option>{/each}</select></label><label>State dwell (µs)<input type="number" min="250" max="5000" value={draft.acquisition.state_dwell_us} oninput={(event) => setPulseDwell(event.currentTarget.value)} /></label><span class="estimate"><strong>Estimated cycle rate</strong>{estimatedPulseRate} cycles/s (nominal; measured timing is recorded separately)</span></div><p class="help">Raw fields are automatically preserved: cycle_index, t_us, red_TX, dark1_TX, ir_TX, dark2_TX, red_RX, dark1_RX, ir_RX, dark2_RX. Preview plots remain display-only.</p></section>
        {/if}
        <section class="subpanel"><h3>Firmware association and save-time summary</h3><div class="editor-grid"><label>Firmware<select value={draft.associated_sketch?.is_wvu_reference ? 'reference' : 'custom'} onchange={(event) => setSketchKind(event.currentTarget.value)}><option value="reference">WVU Reference Firmware</option><option value="custom">Custom sketch</option></select></label><label>Sketch name<input bind:value={draft.associated_sketch!.name} /></label><label>Relative sketch path (optional)<input bind:value={draft.associated_sketch!.relative_path} placeholder="labs/team2.ino" /></label></div><p class="help">Mode: {draft.acquisition.acquisition_mode === 'pulseox_4state' ? 'Pulse Oximetry — 4-State' : 'Simultaneous Analog'} · Channels: {pulseox ? 'TX + RX' : draftChannels.length} · Rate: {draft.acquisition.sample_rate_hz} Hz · ADC: {draft.acquisition.adc_resolution_bits} bit · Outputs: {outputSummary}</p><p class="help">Firmware compatibility: the connected firmware advertises its limits during the normal handshake. The shipped WVU firmware supports up to six analog channels, 12/14-bit ADC, 100–1000 Hz simultaneous frames, and the fixed four-state pulse-ox workflow. Save is allowed offline; Start blocks if the connected board cannot advertise or configure the requested resources.</p><div class="actions"><button type="button" onclick={openFirmware}>Open associated sketch in Firmware</button><button class="gold" type="button" onclick={saveDraft} disabled={saving}>{saving ? 'Saving…' : 'Save changes as new version'}</button><button type="button" onclick={discardDraft} disabled={saving}>Discard draft</button></div></section>
      {/if}
    </dialog>
  </div>
{/if}

<style>
  .authoring { margin-top: .8rem; padding-top: .8rem; border-top: 1px solid #d7dde2; }
  .manager-heading, .actions { display: flex; flex-wrap: wrap; gap: .7rem; align-items: end; justify-content: space-between; }
  .manager-heading h2, .manager-heading h3 { margin: 0; } .manager-heading p { margin: .35rem 0 0; max-width: 74ch; }
  .backdrop { position: fixed; inset: 0; z-index: 1100; display: grid; place-items: center; padding: 1rem; background: rgb(12 27 42 / 52%); }
  .lab-dialog { width: min(100%, 96rem); max-height: min(92vh, 70rem); overflow: auto; border: 1px solid #8aa0b6; border-radius: .4rem; padding: clamp(1rem, 2.5vw, 1.5rem); background: #fff; }
  .help { color: #4b5965; } .warning { border-left: 4px solid #EEAA00; background: #fff8e8; padding: .7rem; } .error { color: #8b1515; font-weight: 700; }
  label { min-width: 0; display: grid; gap: .3rem; font-weight: 650; } input, select, textarea { min-width: 0; width: 100%; min-height: 2.35rem; border: 1px solid #8493a0; border-radius: .25rem; padding: .35rem .5rem; font: inherit; } textarea { min-height: 5rem; resize: vertical; }
  button { min-height: 2.4rem; border: 1px solid #002855; border-radius: .3rem; background: #fff; color: #002855; padding: .45rem .7rem; font: inherit; font-weight: 650; cursor: pointer; } button.gold { background: #EEAA00; color: #17222e; } button.danger { border-color: #8b1515; color: #8b1515; } button:focus-visible, input:focus-visible, select:focus-visible, textarea:focus-visible { outline: 3px solid #EEAA00; outline-offset: 2px; }
  .history, .subpanel { min-width: 0; margin-top: 1rem; padding: .8rem; border: 1px solid #d7dde2; border-radius: .3rem; background: #f9fbfc; } .history h3, .subpanel h3 { margin-top: 0; }
  .history-table { min-width: 0; overflow: auto; } .history-head, .history-row { min-width: 56rem; display: grid; grid-template-columns: minmax(14rem, 1.4fr) 6rem 6rem minmax(15rem, 1.3fr) minmax(16rem, 1.3fr); gap: .6rem; padding: .6rem; border-bottom: 1px solid #d7dde2; align-items: center; } .history-head { font-weight: 800; background: #eaf0f4; } .history-row small { display: block; overflow-wrap: anywhere; } .row-actions { display: flex; flex-wrap: wrap; gap: .35rem; } .row-actions button { min-height: 2rem; }
  .editor-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 15rem), 1fr)); gap: .75rem; align-items: end; margin-top: .8rem; }
  .channel-grid { display: grid; grid-template-columns: 5.5rem minmax(8rem, 1fr) minmax(10rem, 1.2fr) minmax(10rem, 1.2fr) minmax(12rem, 1.2fr) 4.5rem 5rem auto; gap: .45rem; align-items: center; margin-top: .45rem; min-width: 63rem; } .channel-grid.header { font-weight: 800; font-size: .84rem; } .channel-grid input[type='checkbox'] { width: auto; min-height: auto; justify-self: center; } .subpanel { overflow-x: auto; }
  .output-grid { display: flex; flex-wrap: wrap; gap: .8rem; } .choice { display: flex; align-items: center; gap: .45rem; } .choice input { width: auto; min-height: auto; } .estimate { display: grid; gap: .3rem; padding: .55rem .7rem; border: 1px solid #d7dde2; border-radius: .25rem; background: #fff; }
  @media (max-width: 700px) { .lab-dialog { padding: 1rem; } .manager-heading, .actions { align-items: stretch; } .manager-heading > button, .actions > button { flex: 1 1 12rem; } }
</style>
