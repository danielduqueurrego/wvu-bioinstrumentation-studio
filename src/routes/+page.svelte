<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { open, save } from '@tauri-apps/plugin-dialog';
  import LivePlot from '$lib/components/LivePlot.svelte';
  import OperatingModeControl from '$lib/components/OperatingModeControl.svelte';
  import type FirmwareWorkspaceComponent from '$lib/components/FirmwareWorkspace.svelte';
  import { connectionActions } from '$lib/connection-actions';
  import { durationRequest, isTimedDurationValid, type RecordingDurationRequest } from '$lib/duration';
  import type { OperatingMode } from '$lib/operating-mode';
  import logoUrl from '../../assets/branding/WVU-CBE Logo.svg';

  type Point = { sequence: number; timestamp_us: number; counts: number };
  type Board = { port: string; name: string; fqbn: string; serial_number?: string };
  type Duration = RecordingDurationRequest;
  type Integrity = {
    received_packets: number; crc_failures: number; invalid_frames: number; unsupported_versions: number;
    missing_packet_sequences: number; duplicate_packets: number; out_of_order_packets: number;
    missing_sample_sequences: number; duplicate_sample_sequences: number; out_of_order_sample_sequences: number;
    firmware_overflows: number; host_channel_overflows: number; reconnects: number; disconnect_events: number;
  };
  type Summary = {
    state: string; samples: number; packets: number; measured_rate_hz: number;
    board_elapsed_seconds: number; host_elapsed_seconds: number; bmeg_path: string; csv_path: string;
    metadata_path: string; recording_status: string; duration: Duration; stop_reason: string;
    completion_status: string; initial_free_disk_bytes?: number; final_free_disk_bytes?: number;
    integrity: Integrity; error?: string; profile?: ProfileSnapshot;
  };
  type SessionStatus = {
    state: string; board: string; port: string; protocol_version: string; simulator: boolean;
    samples: number; packets: number; measured_rate_hz: number; integrity: Integrity;
    duration?: Duration; elapsed_seconds: number; remaining_seconds?: number;
    available_disk_bytes?: number; storage_warning?: string; stop_reason?: string;
    connection_diagnostics?: ConnectionDiagnostics;
    last_error?: string; last_summary?: Summary;
  };
  type ConnectionDiagnostics = {
    selected_port: string; board: string; fqbn: string; port_opened: boolean;
    bytes_received: number; valid_frames: number; crc_failures: number; skipped_noise_bytes: number;
    hello_received: boolean; capabilities_received: boolean; pong_received: boolean;
    protocol_version?: string; firmware_build?: number; firmware_board_id?: number; raw_byte_classification: string; ping_attempts: number; handshake_elapsed_ms: number;
    reset_attempted: boolean; original_port?: string; final_port?: string;
    disappearance_observed: boolean; reappearance_observed: boolean; bootloader_observed: boolean;
    failure_category?: string; recommended_action: string;
  };
  type ResetRetryResult = {
    original_port: string; final_port?: string; handshake_succeeded: boolean;
    diagnostics: ConnectionDiagnostics;
  };
  type HandshakeRetryResult = { handshake_succeeded: boolean; diagnostics: ConnectionDiagnostics };
  type FirmwareWorkflowStatus = { compatibility: string };
  type AcquisitionProfile = {
    schema_version: number; profile_id: string; profile_version: string; display_name: string;
    category: string; status: 'locked' | 'draft' | 'retired'; source: 'built_in' | 'instructor';
    description: string; target_board: string; fqbn: string;
    required_firmware: { protocol_major: number; protocol_minor_min: number; build: string; device: string };
    acquisition: { analog_pin: string; adc_resolution_bits: number; sample_rate_hz: number; allowed_duration_modes: string[]; timed_presets_seconds: number[]; minimum_custom_duration_seconds: number };
    display: { primary_quantity: string; channel_label: string; raw_units_label: string; voltage_units_label: string; voltage_reference_v: number; plot_min_v: number; plot_max_v: number };
    safety: { bench_only: boolean; human_connection_authorized: boolean; not_medical_device: boolean; notices: string[] };
    export: { signal_name: string; include_profile_snapshot: boolean };
    integrity: { canonical_hash_algorithm: string; canonical_hash: string };
  };
  type ProfileSnapshot = { bench_notice_acknowledged: boolean; profile: AcquisitionProfile };

  const emptyIntegrity: Integrity = {
    received_packets: 0, crc_failures: 0, invalid_frames: 0, unsupported_versions: 0,
    missing_packet_sequences: 0, duplicate_packets: 0, out_of_order_packets: 0,
    missing_sample_sequences: 0, duplicate_sample_sequences: 0, out_of_order_sample_sequences: 0,
    firmware_overflows: 0, host_channel_overflows: 0, reconnects: 0, disconnect_events: 0
  };
  const activeStates = ['Connecting', 'Connected', 'Configured', 'Acquiring', 'Stopping'];

  let view = 'Home';
  let samples: Point[] = [];
  let boards: Board[] = [];
  let selectedPort = '';
  let source: 'simulator' | 'hardware' = 'simulator';
  let outputDirectory = 'recordings';
  let durationMode: 'timed' | 'until_stopped' = 'timed';
  let durationPreset = '60';
  let customSeconds = 60;
  let duration: Duration = { mode: 'timed', seconds: 60 };
  let note = 'Simulator waveform; no human signal.';
  let volts = false;
  let statusMessage = 'Ready. Simulator uses the same Rust session, parser, recording, and export path.';
  let session: SessionStatus = {
    state: 'Disconnected', board: '', port: '', protocol_version: '0.1', simulator: false,
    samples: 0, packets: 0, measured_rate_hz: 0, integrity: emptyIntegrity,
    elapsed_seconds: 0
  };
  let polling = false;
  let FirmwareWorkspace: typeof FirmwareWorkspaceComponent | undefined;
  let firmwareCompatibility = 'unknown';
  let acquisitionProfiles: AcquisitionProfile[] = [];
  let selectedProfileId = 'wvu.bmeg420l.general.a0.development.v1';
  let operatingMode: OperatingMode = 'student';
  let modeChangeInFlight = false;
  let benchNoticeAcknowledged = false;
  let instructorAcknowledgement = false;
  let draftId = 'wvu.bmeg420l.instructor.example.v1';
  let draftDescription = '';
  let finalDraftVersion = '1.0.1';
  let authoringDraft: AcquisitionProfile | undefined;

  $: if (source === 'hardware') {
    note = 'A0 raw floating/uncalibrated engineering communication test; no human signal.';
  } else {
    note = 'Simulator waveform; no human signal.';
  }
  $: timedSeconds = durationPreset === 'custom' ? Number(customSeconds) : Number(durationPreset);
  $: activeProfile = acquisitionProfiles.find((profile) => profile.profile_id === selectedProfileId);
  $: instructorModeActive = operatingMode === 'instructor_authoring';
  $: profileTimedPresets = activeProfile?.acquisition.timed_presets_seconds ?? [10, 30, 60, 300, 600];
  $: timedDurationValid = isTimedDurationValid(timedSeconds);
  $: duration = durationRequest(durationMode, timedSeconds);
  $: canStart = session.state === 'Disconnected'
    && (source === 'simulator' || (Boolean(selectedPort) && firmwareCompatibility === 'wvu_protocol_compatible'))
    && Boolean(activeProfile)
    && (durationMode === 'until_stopped' || timedDurationValid)
    && (!activeProfile?.category || !['ecg', 'emg'].includes(activeProfile.category) || benchNoticeAcknowledged);
  $: isActive = activeStates.includes(session.state);
  $: recoveryActions = connectionActions({
    source,
    active: isActive,
    selectedPort,
    failureCategory: session.connection_diagnostics?.failure_category
  });
  $: canReset = recoveryActions.canReset;
  $: canRetryHandshake = recoveryActions.canRetryHandshake;
  $: estimatedMegabytesPerMinute = 0.82;

  function formatDuration(seconds: number | undefined) {
    if (seconds === undefined || !Number.isFinite(seconds)) return '—';
    const whole = Math.max(0, Math.floor(seconds));
    const hours = Math.floor(whole / 3600);
    const minutes = Math.floor((whole % 3600) / 60);
    const remainder = whole % 60;
    return hours > 0
      ? `${hours}:${String(minutes).padStart(2, '0')}:${String(remainder).padStart(2, '0')}`
      : `${minutes}:${String(remainder).padStart(2, '0')}`;
  }

  function formatStorage(bytes: number | undefined) {
    if (bytes === undefined) return 'not yet checked';
    return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GiB free`;
  }

  function estimateText() {
    if (durationMode === 'until_stopped') return `Approx. ${estimatedMegabytesPerMinute.toFixed(2)} MiB/minute; free space is checked every 15 seconds.`;
    return `Approx. ${(timedSeconds / 60 * estimatedMegabytesPerMinute).toFixed(2)} MiB raw BMEG + CSV for this timed run.`;
  }

  async function refreshBoards() {
    try {
      boards = await invoke<Board[]>('list_boards');
      selectedPort = boards.some((board) => board.port === selectedPort) ? selectedPort : (boards[0]?.port ?? '');
      statusMessage = boards.length
        ? `${boards[0].name} detected on ${boards[0].port}.`
        : 'No supported UNO R4 WiFi detected. Simulator remains available.';
    } catch (error) {
      statusMessage = `Discovery error: ${String(error)}`;
    }
  }

  async function pollSession() {
    if (polling) return;
    polling = true;
    try {
      session = await invoke<SessionStatus>('get_session_status');
      samples = await invoke<Point[]>('get_recent_display_data');
      if (session.last_error) statusMessage = session.last_error;
      if (session.last_summary) {
        statusMessage = `${session.last_summary.recording_status}: ${session.last_summary.samples} validated samples at ${session.last_summary.measured_rate_hz.toFixed(3)} Hz.`;
      }
    } catch (error) {
      statusMessage = `Session status error: ${String(error)}`;
    } finally {
      polling = false;
    }
  }

  async function refreshFirmwareCompatibility() {
    try {
      firmwareCompatibility = (await invoke<FirmwareWorkflowStatus>('get_firmware_workflow_status')).compatibility;
    } catch {
      firmwareCompatibility = 'unknown';
    }
  }

  async function refreshProfiles() {
    try {
      acquisitionProfiles = await invoke<AcquisitionProfile[]>('list_acquisition_profiles');
      if (!acquisitionProfiles.some((profile) => profile.profile_id === selectedProfileId)) {
        selectedProfileId = acquisitionProfiles[0]?.profile_id ?? '';
      }
      const confirmedMode = await invoke<OperatingMode>('get_profile_mode');
      if (!modeChangeInFlight) operatingMode = confirmedMode;
    } catch (error) {
      statusMessage = `Profile error: ${String(error)}`;
    }
  }

  function selectProfile() {
    benchNoticeAcknowledged = false;
    durationPreset = String(profileTimedPresets.includes(60) ? 60 : profileTimedPresets[0] ?? 10);
    statusMessage = activeProfile
      ? `${activeProfile.display_name} selected. ${activeProfile.status === 'locked' ? 'Protected settings are locked by the approved profile.' : 'Draft profile selected.'}`
      : 'Select a valid locked acquisition profile.';
  }

  async function commitProfileMode(mode: OperatingMode) {
    modeChangeInFlight = true;
    try {
      operatingMode = await invoke<OperatingMode>('set_profile_mode', {
        mode,
        acknowledgement: mode === 'instructor_authoring' && instructorAcknowledgement
      });
      statusMessage = operatingMode === 'instructor_authoring'
        ? 'Instructor authoring mode is enabled locally. It is a workflow guard, not authentication.'
        : 'Student mode is enabled. Locked profile settings cannot be edited.';
    } catch (error) {
      // The backend remains authoritative if a local command failure occurs.
      try {
        operatingMode = await invoke<OperatingMode>('get_profile_mode');
      } catch {
        operatingMode = 'student';
      }
      statusMessage = `Mode change error: ${String(error)}`;
    } finally {
      modeChangeInFlight = false;
    }
  }

  function onModeConfirmed(mode: OperatingMode) {
    if (modeChangeInFlight || session.state !== 'Disconnected') return;
    void commitProfileMode(mode);
  }

  function onInstructorBlocked() {
    statusMessage = 'Confirm that instructor mode can change acquisition settings, then select Instructor authoring.';
  }

  async function duplicateDraft() {
    try {
      authoringDraft = await invoke<AcquisitionProfile>('duplicate_profile_to_draft', { profileId: selectedProfileId, draftId });
      draftDescription = authoringDraft.description;
      statusMessage = `Draft ${authoringDraft.profile_id} created. Only its descriptive field is editable in this Phase 3A interface.`;
    } catch (error) { statusMessage = `Draft error: ${String(error)}`; }
  }

  async function finalizeDraft() {
    if (!authoringDraft) return;
    try {
      await invoke<AcquisitionProfile>('update_profile_draft_description', { profileId: authoringDraft.profile_id, profileVersion: authoringDraft.profile_version, description: draftDescription });
      const finalized = await invoke<AcquisitionProfile>('finalize_profile_draft', { profileId: authoringDraft.profile_id, profileVersion: authoringDraft.profile_version, finalVersion: finalDraftVersion });
      authoringDraft = undefined;
      await refreshProfiles();
      selectedProfileId = finalized.profile_id;
      statusMessage = `Finalized ${finalized.display_name} ${finalized.profile_version}; its SHA-256 integrity hash is now locked.`;
    } catch (error) { statusMessage = `Finalize error: ${String(error)}`; }
  }

  async function importProfilePackage() {
    try {
      const selected = await open({ multiple: false, filters: [{ name: 'Acquisition profile', extensions: ['json'] }] });
      if (typeof selected !== 'string') return;
      const imported = await invoke<AcquisitionProfile>('import_profile_package', { source: selected });
      await refreshProfiles();
      selectedProfileId = imported.profile_id;
      statusMessage = `Imported and validated locked profile ${imported.profile_id} ${imported.profile_version}.`;
    } catch (error) { statusMessage = `Profile import error: ${String(error)}`; }
  }

  async function exportProfilePackage() {
    if (!activeProfile) return;
    try {
      const destination = await save({ defaultPath: `${activeProfile.profile_id}_${activeProfile.profile_version}.profile.json`, filters: [{ name: 'Acquisition profile', extensions: ['json'] }] });
      if (!destination) return;
      await invoke('export_profile_package', { profileId: activeProfile.profile_id, profileVersion: activeProfile.profile_version, destination });
      statusMessage = `Exported the validated locked profile package to ${destination}.`;
    } catch (error) { statusMessage = `Profile export error: ${String(error)}`; }
  }

  async function retireSelectedProfile() {
    if (!activeProfile || activeProfile.source !== 'instructor') return;
    try {
      await invoke('retire_profile', {
        profileId: activeProfile.profile_id,
        profileVersion: activeProfile.profile_version
      });
      const retiredName = `${activeProfile.display_name} ${activeProfile.profile_version}`;
      await refreshProfiles();
      selectedProfileId = acquisitionProfiles[0]?.profile_id ?? '';
      statusMessage = `Retired ${retiredName} from the active selection list. Existing recording provenance remains unchanged.`;
    } catch (error) { statusMessage = `Retire error: ${String(error)}`; }
  }

  async function startRecording() {
    if (!canStart) {
      statusMessage = 'Choose a valid timed duration of at least 10 seconds, or select Until stopped.';
      return;
    }
    statusMessage = 'Connecting through the Rust production session controller…';
    try {
      session = source === 'simulator'
        ? await invoke<SessionStatus>('start_profile_simulator_recording', { outputDirectory, duration, profileId: selectedProfileId, benchNoticeAcknowledged })
        : await invoke<SessionStatus>('start_profile_hardware_recording', { port: selectedPort, outputDirectory, duration, profileId: selectedProfileId, benchNoticeAcknowledged });
      view = 'Acquisition';
    } catch (error) {
      statusMessage = `Start error: ${String(error)}`;
    }
  }

  async function stopRecording() {
    try {
      session = await invoke<SessionStatus>('stop_recording');
      statusMessage = 'Stop requested. The Rust worker is flushing validated data and metadata.';
    } catch (error) {
      statusMessage = `Stop error: ${String(error)}`;
    }
  }

  async function disconnect() {
    try {
      session = await invoke<SessionStatus>('disconnect_session');
      statusMessage = 'Session disconnected. A future recording requires an explicit new start.';
    } catch (error) {
      statusMessage = `Disconnect error: ${String(error)}`;
    }
  }

  async function resetBoardAndRetry() {
    if (!canReset) return;
    statusMessage = 'Resetting the selected UNO R4 WiFi at 1200 bps, then waiting for it to re-enumerate…';
    try {
      const result = await invoke<ResetRetryResult>('reset_board_and_retry', { port: selectedPort });
      if (result.final_port) selectedPort = result.final_port;
      await refreshBoards();
      await pollSession();
      statusMessage = result.handshake_succeeded
        ? `Reset and retry succeeded on ${result.final_port ?? result.original_port}. Select Start recording for a new session.`
        : `${result.diagnostics.failure_category ?? 'reset_failed'}: ${result.diagnostics.recommended_action}`;
    } catch (error) {
      statusMessage = `Reset and retry error: ${String(error)}`;
    }
  }

  async function retryHandshake() {
    if (!canRetryHandshake) return;
    statusMessage = 'Retrying the normal protocol handshake without resetting or uploading the board…';
    try {
      const result = await invoke<HandshakeRetryResult>('retry_hardware_handshake', { port: selectedPort });
      await pollSession();
      statusMessage = result.handshake_succeeded
        ? `Handshake succeeded on ${selectedPort}. Select Start recording for a new session.`
        : `${result.diagnostics.failure_category ?? 'handshake_failed'}: ${result.diagnostics.recommended_action}`;
    } catch (error) {
      statusMessage = `Handshake retry error: ${String(error)}`;
    }
  }

  async function exportCsv() {
    try {
      const csvPath = await invoke<string>('export_session_csv');
      statusMessage = `CSV was streamed from the finalized BMEG recording: ${csvPath}`;
    } catch (error) {
      statusMessage = `Export error: ${String(error)}`;
    }
  }

  async function selectView(nextView: string) {
    if (nextView === 'Firmware' && !FirmwareWorkspace) {
      const module = await import('$lib/components/FirmwareWorkspace.svelte');
      FirmwareWorkspace = module.default;
    }
    view = nextView;
  }

  onMount(() => {
    void refreshBoards();
    void pollSession();
    void refreshFirmwareCompatibility();
    void refreshProfiles();
    const timer = window.setInterval(() => void pollSession(), 40); // 25 Hz; no per-sample events.
    const firmwareTimer = window.setInterval(() => void refreshFirmwareCompatibility(), 750);
    return () => {
      window.clearInterval(timer);
      window.clearInterval(firmwareTimer);
    };
  });
</script>

<svelte:head><title>WVU Bioinstrumentation Studio</title></svelte:head>

<div class="app-shell">
  <header class="app-header">
    <img src={logoUrl} alt="Approved WVU College of Business and Economics logo" />
    <div>
      <h1>WVU Bioinstrumentation Studio</h1>
      <p>Firmware, Acquisition, and Calibration for BMEG 420L</p>
    </div>
  </header>

  <div class="workspace">
    <aside class="navigation">
      <nav aria-label="Primary">
        {#each ['Home', 'Firmware', 'Acquisition', 'Diagnostics'] as item}
          <button class:active={view === item} aria-current={view === item ? 'page' : undefined} onclick={() => void selectView(item)}>{item}</button>
        {/each}
      </nav>
    </aside>

    <main class="content" class:wide-content={view === 'Firmware' || view === 'Acquisition'}>
      {#if view === 'Home'}
        <h2>Home</h2>
        <p class="notice">Teaching and engineering equipment only — not a medical device. Phase 1 permits Arduino-alone, simulator, or safe bench-signal work only.</p>
        <div class="action-row">
          <button onclick={refreshBoards}>Refresh supported UNO R4 WiFi boards</button>
          <button class="gold" onclick={() => { source = 'simulator'; view = 'Acquisition'; }}>Open simulator acquisition</button>
        </div>
      {:else if view === 'Firmware'}
        {#if FirmwareWorkspace}<FirmwareWorkspace />{/if}
      {:else if view === 'Acquisition'}
        <h2>Acquisition</h2>
        <p class="notice">Raw A0 only. A floating A0 test is uncalibrated; no physiological interpretation or hidden filtering is performed.</p>
        {#if source === 'hardware' && firmwareCompatibility !== 'wvu_protocol_compatible'}
          <p class="warning" role="status">Hardware recording is disabled until the selected UNO R4 WiFi proves the controlled WVU firmware identity. Open <button class="inline-action" onclick={() => void selectView('Firmware')}>Firmware</button> and use Verify WVU firmware or Restore WVU reference firmware.</p>
        {/if}

        <section class="panel profile-panel" aria-labelledby="profile-title">
          <div class="panel-heading"><div><h3 id="profile-title">Acquisition profile</h3><p class="help">Profiles bind protected acquisition settings, safety notices, firmware requirements, and export provenance.</p></div><span class:locked={!instructorModeActive} class="mode-badge">{instructorModeActive ? 'Instructor authoring' : 'Student mode'}</span></div>
          <OperatingModeControl bind:operatingMode bind:instructorAcknowledgement disabled={session.state !== 'Disconnected' || modeChangeInFlight} {onModeConfirmed} {onInstructorBlocked} />
          {#if instructorModeActive}
            <section class="authoring" aria-label="Instructor draft profile workflow">
              <p class="warning">Instructor mode is a local workflow guard, not strong authentication. Finalizing creates a new locked version; it does not alter built-in profiles.</p>
              <div class="control-grid"><label>New draft profile ID <input bind:value={draftId} /></label><div class="field-action"><span>Draft from selected locked profile</span><button onclick={duplicateDraft}>Duplicate to draft</button></div></div>
              {#if authoringDraft}<div class="control-grid"><label>Draft description <input bind:value={draftDescription} /></label><label>Final version <input bind:value={finalDraftVersion} placeholder="1.0.1" /></label><div class="field-action"><span>Finalize immutable package</span><button class="gold" onclick={finalizeDraft}>Validate and finalize</button></div></div>{/if}
              <div class="button-pair"><button onclick={importProfilePackage}>Import locked profile package</button><button onclick={exportProfilePackage} disabled={!activeProfile}>Export selected profile</button>{#if activeProfile?.source === 'instructor'}<button onclick={retireSelectedProfile}>Retire selected instructor profile</button>{/if}</div>
            </section>
          {/if}
          <label>Approved profile
            <select bind:value={selectedProfileId} onchange={selectProfile} disabled={session.state !== 'Disconnected'}>
              {#each acquisitionProfiles as profile}<option value={profile.profile_id}>{profile.display_name} — {profile.profile_version}</option>{/each}
            </select>
          </label>
          {#if activeProfile}
            <div class="profile-details">
              <span><strong>Profile</strong>{activeProfile.profile_id} / {activeProfile.profile_version}</span><span><strong>Category / source</strong>{activeProfile.category} / {activeProfile.source}</span><span><strong>Lock status</strong>{activeProfile.status}; protected pin, ADC, rate, firmware requirement, safety, and units cannot be changed in Student mode.</span>
              <span><strong>Channel / units</strong>{activeProfile.display.channel_label}; {activeProfile.display.raw_units_label} and Arduino input {activeProfile.display.voltage_units_label} only</span><span><strong>Protected acquisition</strong>{activeProfile.acquisition.analog_pin}, {activeProfile.acquisition.adc_resolution_bits} bit, {activeProfile.acquisition.sample_rate_hz} samples/s</span><span><strong>Firmware requirement</strong>Protocol {activeProfile.required_firmware.protocol_major}.{activeProfile.required_firmware.protocol_minor_min}+; build {activeProfile.required_firmware.build}; device {activeProfile.required_firmware.device}</span>
              <span class="profile-hash"><strong>Integrity</strong>{activeProfile.integrity.canonical_hash_algorithm} {activeProfile.integrity.canonical_hash}</span>
            </div>
            {#each activeProfile.safety.notices as notice}<p class="warning profile-notice">{notice}</p>{/each}
            {#if ['ecg', 'emg'].includes(activeProfile.category)}
              <label class="acknowledgement"><input type="checkbox" bind:checked={benchNoticeAcknowledged} disabled={session.state !== 'Disconnected'} /> I acknowledge this {activeProfile.category.toUpperCase()} profile is bench-validation only. No human-connected recording is authorized.</label>
            {/if}
          {:else}
            <p class="error">No valid locked profile is available. Refresh the application or contact the instructor.</p>
          {/if}
        </section>

        <section class="panel" aria-labelledby="setup-title">
          <h3 id="setup-title">Session setup</h3>
          <div class="control-grid">
            <label>Source
              <select bind:value={source} disabled={session.state !== 'Disconnected'}>
                <option value="simulator">Simulator</option>
                <option value="hardware">Hardware</option>
              </select>
            </label>
            <div class="field-action"><span>Detected devices</span><button onclick={refreshBoards} disabled={session.state !== 'Disconnected'}>Refresh devices</button></div>
            {#if source === 'hardware'}
              <label>UNO R4 WiFi port
                <select bind:value={selectedPort} disabled={session.state !== 'Disconnected'}>
                  {#each boards as board}<option value={board.port}>{board.name} — {board.port} ({board.fqbn})</option>{/each}
                </select>
              </label>
            {/if}
            <label>Output folder <input bind:value={outputDirectory} disabled={session.state !== 'Disconnected'} aria-describedby="output-help" /></label>
            <label>Analog pin <input value={activeProfile?.acquisition.analog_pin ?? '—'} readonly title="Protected by the selected profile" /></label>
            <label>ADC resolution <input value={activeProfile ? `${activeProfile.acquisition.adc_resolution_bits} bit` : '—'} readonly title="Protected by the selected profile" /></label>
            <label>Sample rate <input value={activeProfile ? `${activeProfile.acquisition.sample_rate_hz} samples/s` : '—'} readonly title="Protected by the selected profile" /></label>
            <label>Test note <input bind:value={note} readonly /></label>
          </div>
          <p id="output-help" class="help">Files use a controlled timestamped Phase 1 name. Existing files are never overwritten.</p>
        </section>

        <section class="panel" aria-labelledby="duration-title">
          <h3 id="duration-title">Recording duration</h3>
          <fieldset disabled={session.state !== 'Disconnected'}>
            <legend>Duration mode</legend>
            <div class="choice-row">
              <label class="choice"><input type="radio" bind:group={durationMode} value="timed" /> Timed</label>
              <label class="choice"><input type="radio" bind:group={durationMode} value="until_stopped" /> Until stopped</label>
            </div>
          </fieldset>
          {#if durationMode === 'timed'}
            <div class="duration-controls">
              <label>Timed preset
                <select bind:value={durationPreset} disabled={session.state !== 'Disconnected'}>
                  {#each profileTimedPresets as preset}<option value={String(preset)}>{preset >= 60 ? `${preset / 60} minute${preset === 60 ? '' : 's'}` : `${preset} seconds`}</option>{/each}<option value="custom">Custom</option>
                </select>
              </label>
              {#if durationPreset === 'custom'}
                <label>Custom duration (seconds)
                  <input type="number" min="10" step="1" bind:value={customSeconds} disabled={session.state !== 'Disconnected'} aria-invalid={!timedDurationValid} />
                </label>
              {/if}
              {#if !timedDurationValid}<p class="error">Timed recordings must be whole seconds and at least 10 seconds.</p>{/if}
            </div>
          {:else}
            <p class="help">Recording continues without an automatic time limit. Press <strong>Stop recording</strong> to finish it.</p>
          {/if}
          <p class="help">{estimateText()} Warning below 1 GiB free; controlled stop below 250 MiB free.</p>
        </section>

        <div class="action-row recording-actions">
          <button class="gold" onclick={startRecording} disabled={!canStart}>Connect, configure, and start recording</button>
          <button class="stop" onclick={stopRecording} disabled={!isActive || session.state === 'Stopping'}>Stop recording</button>
          <button onclick={disconnect} disabled={session.state === 'Disconnected'}>Disconnect</button>
          {#if canRetryHandshake}<button onclick={retryHandshake}>Retry handshake</button>{/if}
          {#if canReset}<button onclick={resetBoardAndRetry}>Reset board and retry</button>{/if}
          <button onclick={exportCsv} disabled={!session.last_summary}>Show CSV export</button>
        </div>

        <section class="metric-grid" aria-label="Acquisition metrics">
          <span><strong>State</strong>{session.state}</span><span><strong>Device</strong>{session.board || 'not connected'} {session.port ? `(${session.port})` : ''}</span><span><strong>Protocol</strong>{session.protocol_version}</span>
          <span><strong>Duration</strong>{session.duration?.mode === 'until_stopped' ? 'Until stopped' : session.duration?.seconds ? `${session.duration.seconds} s timed` : durationMode === 'until_stopped' ? 'Until stopped' : `${timedSeconds} s timed`}</span>
          <span><strong>Elapsed host</strong>{formatDuration(session.elapsed_seconds)}</span>
          {#if session.remaining_seconds !== undefined}<span><strong>Remaining</strong>{formatDuration(session.remaining_seconds)}</span>{/if}
          <span><strong>Storage</strong>{formatStorage(session.available_disk_bytes)}</span><span><strong>Samples</strong>{session.samples}</span><span><strong>Measured</strong>{session.measured_rate_hz.toFixed(3)} Hz</span>
          <span><strong>Valid packets</strong>{session.integrity.received_packets}</span><span><strong>CRC failures</strong>{session.integrity.crc_failures}</span><span><strong>Missing packets</strong>{session.integrity.missing_packet_sequences}</span><span><strong>Missing samples</strong>{session.integrity.missing_sample_sequences}</span>
          <span><strong>Duplicate / out-of-order packets</strong>{session.integrity.duplicate_packets} / {session.integrity.out_of_order_packets}</span><span><strong>Firmware / host overflows</strong>{session.integrity.firmware_overflows} / {session.integrity.host_channel_overflows}</span><span><strong>Reconnects / disconnects</strong>{session.integrity.reconnects} / {session.integrity.disconnect_events}</span>
        </section>
        {#if session.storage_warning}<p class="warning" role="status">{session.storage_warning}</p>{/if}
        {#if session.last_error}<p class="error" role="alert">Last error: {session.last_error}</p>{/if}
        {#if session.connection_diagnostics}
          <section class="panel diagnostics" aria-label="Connection diagnostics">
            <h3>Connection diagnostics</h3>
            <div class="metric-grid">
              <span><strong>Original / final port</strong>{session.connection_diagnostics.original_port ?? session.connection_diagnostics.selected_port} / {session.connection_diagnostics.final_port ?? session.connection_diagnostics.selected_port}</span>
              <span><strong>Port opened / handshake</strong>{session.connection_diagnostics.port_opened ? 'yes' : 'no'} / {session.connection_diagnostics.handshake_elapsed_ms} ms</span>
              <span><strong>Bytes / valid frames / CRC</strong>{session.connection_diagnostics.bytes_received} / {session.connection_diagnostics.valid_frames} / {session.connection_diagnostics.crc_failures}</span>
              <span><strong>HELLO / capabilities / PONG</strong>{session.connection_diagnostics.hello_received ? 'yes' : 'no'} / {session.connection_diagnostics.capabilities_received ? 'yes' : 'no'} / {session.connection_diagnostics.pong_received ? 'yes' : 'no'}</span>
              <span><strong>Firmware build / board ID</strong>{session.connection_diagnostics.firmware_build ?? '—'} / {session.connection_diagnostics.firmware_board_id ?? '—'}</span>
              <span><strong>Received bytes</strong>{session.connection_diagnostics.raw_byte_classification}</span>
              <span><strong>Failure category</strong>{session.connection_diagnostics.failure_category ?? 'none'}</span>
              {#if session.connection_diagnostics.reset_attempted}<span><strong>Reset discovery</strong>bootloader: {session.connection_diagnostics.bootloader_observed ? 'yes' : 'no'}; disappeared: {session.connection_diagnostics.disappearance_observed ? 'yes' : 'no'}; returned: {session.connection_diagnostics.reappearance_observed ? 'yes' : 'no'}</span>{/if}
            </div>
            <p class="help">{session.connection_diagnostics.recommended_action}</p>
          </section>
        {/if}

        <section class="panel plot-panel">
          <div class="plot-heading"><h3>Bounded live raw plot</h3><label class="toggle"><input type="checkbox" bind:checked={volts} /> Display volts (counts × 5.0 / 4095.0)</label></div>
          <LivePlot {samples} {volts} />
        </section>
        {#if session.last_summary}
          <section class="panel paths">
            <h3>Finalized files</h3>
            {#if session.last_summary.profile}<p><strong>Profile provenance:</strong> {session.last_summary.profile.profile.profile_id} {session.last_summary.profile.profile.profile_version}</p>{/if}
            <p title={session.last_summary.bmeg_path}>{session.last_summary.bmeg_path}</p>
            <p title={session.last_summary.metadata_path}>{session.last_summary.metadata_path}</p>
            <p title={session.last_summary.csv_path}>{session.last_summary.csv_path}</p>
            <p>Finalization: {session.last_summary.completion_status}; reason: {session.last_summary.stop_reason}.</p>
          </section>
        {/if}
      {:else}
        <h2>Diagnostics</h2>
        <section class="panel diagnostic-grid">
          <p>Current state: <strong>{session.state}</strong></p>
          <p>Last error: {session.last_error ?? 'none'}</p>
          <p>Stop reason: {session.stop_reason ?? 'none'}</p>
          <p>Available storage: {formatStorage(session.available_disk_bytes)}</p>
        </section>
        <p>Serial ownership, packet validation, bounded display data, recording, and export run in Rust; the frontend only polls snapshots.</p>
      {/if}
      <p class="status" aria-live="polite">{statusMessage}</p>
    </main>
  </div>
</div>

<style>
  :global(*) { box-sizing: border-box; }
  :global(body) { margin: 0; min-width: 0; overflow-x: hidden; font-family: "Segoe UI", Arial, sans-serif; background: #F7F7F7; color: #17222e; }
  :global(button), :global(input), :global(select) { font: inherit; }
  .app-shell { min-height: 100vh; min-width: 0; }
  .app-header { min-width: 0; display: flex; flex-wrap: wrap; align-items: center; gap: clamp(.65rem, 2vw, 1.2rem); padding: clamp(.75rem, 2vw, 1.2rem) clamp(1rem, 3vw, 2rem); background: #002855; color: #F7F7F7; }
  .app-header img { width: clamp(110px, 16vw, 150px); max-width: 100%; max-height: 72px; object-fit: contain; flex: 0 1 auto; }
  h1 { margin: 0; font-size: clamp(1.15rem, 2.2vw, 1.45rem); overflow-wrap: anywhere; } .app-header p { margin: .2rem 0 0; overflow-wrap: anywhere; }
  .workspace { display: grid; grid-template-columns: minmax(10.5rem, 13rem) minmax(0, 1fr); min-height: calc(100vh - 96px); }
  .navigation { background: #e8edf1; padding: clamp(.7rem, 2vw, 1.25rem) .75rem; } nav { display: grid; gap: .45rem; }
  button { min-height: 2.5rem; border: 1px solid #002855; border-radius: .3rem; background: #fff; color: #002855; padding: .55rem .75rem; font-weight: 650; text-align: left; cursor: pointer; overflow-wrap: anywhere; }
  button:focus-visible, input:focus-visible, select:focus-visible { outline: 3px solid #EEAA00; outline-offset: 2px; }
  button:disabled { cursor: not-allowed; opacity: .55; } button.active, button:not(:disabled):hover { background: #002855; color: #fff; } button.gold { background: #EEAA00; color: #17222e; } button.stop { background: #9d2424; border-color: #761a1a; color: #fff; } button.inline-action { display: inline; min-height: auto; padding: 0; border: 0; background: transparent; color: #002855; text-decoration: underline; }
  /* Keep prose-oriented pages comfortably readable, but let the data-dense Firmware and
     Acquisition workspaces use the available desktop width.  `width: 100%` also avoids
     percentage sizing being resolved against an intrinsic grid size on some WebView builds. */
  .content { min-width: 0; width: 100%; max-width: 72rem; justify-self: start; padding: clamp(1rem, 3vw, 1.75rem); overflow-wrap: anywhere; }
  .content.wide-content { max-width: none; justify-self: stretch; }
  h2 { margin-top: 0; } h3 { margin: 0 0 .8rem; } .notice, .warning { border-left: 5px solid #EEAA00; background: #fff8e8; padding: .75rem; } .warning { border-color: #9b6700; }
  .panel { min-width: 0; margin: 1rem 0; padding: clamp(.8rem, 2vw, 1rem); background: #fff; border: 1px solid #d7dde2; border-radius: .35rem; }
  .panel-heading { display: flex; min-width: 0; flex-wrap: wrap; justify-content: space-between; gap: .75rem; align-items: start; } .panel-heading .help { max-width: 75ch; }
  .mode-badge { border: 1px solid #855a00; border-radius: 999px; padding: .35rem .6rem; background: #fff4dd; color: #684600; font-weight: 700; white-space: nowrap; } .mode-badge.locked { border-color: #176c33; background: #eaf6ee; color: #174f27; }
  .profile-panel > label { margin-top: .8rem; } .profile-details { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 16rem), 1fr)); gap: .55rem; margin-top: .8rem; } .profile-details span { min-width: 0; padding: .6rem .7rem; border: 1px solid #d7dde2; border-radius: .3rem; background: #f9fbfc; overflow-wrap: anywhere; } .profile-details strong { display: block; color: #42515d; font-size: .78rem; text-transform: uppercase; letter-spacing: .03em; } .profile-hash { font-family: Consolas, "Cascadia Code", monospace; font-size: .82rem; } .profile-notice { margin: .6rem 0 0; }
  .acknowledgement { display: flex; gap: .55rem; align-items: flex-start; margin-top: .75rem; padding: .7rem; background: #fff4dd; border: 1px solid #9b6700; font-weight: 700; } .acknowledgement input { width: auto; min-height: auto; margin-top: .2rem; } .authoring { margin-top: .8rem; padding-top: .8rem; border-top: 1px solid #d7dde2; }
  .button-pair { display: flex; flex-wrap: wrap; gap: .6rem; margin-top: .8rem; } .button-pair button { flex: 0 1 18rem; }
  .control-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 15rem), 1fr)); gap: .8rem; align-items: end; }
  label, .field-action { min-width: 0; display: grid; gap: .3rem; font-weight: 600; } .field-action > span { font-size: .9rem; }
  input, select { min-width: 0; width: 100%; min-height: 2.4rem; border: 1px solid #8493a0; border-radius: .25rem; background: #fff; padding: .35rem .5rem; }
  input[readonly] { background: #eef1f3; color: #374450; } .help { margin: .7rem 0 0; color: #4b5965; } .error { color: #8b1515; font-weight: 650; }
  fieldset { min-width: 0; border: 0; padding: 0; margin: 0; } legend { font-weight: 650; margin-bottom: .3rem; } .choice-row, .duration-controls, .action-row, .plot-heading { display: flex; min-width: 0; flex-wrap: wrap; gap: .7rem; align-items: end; } .choice { display: flex; align-items: center; gap: .4rem; } .choice input, .toggle input { width: auto; min-height: auto; }
  .duration-controls { margin-top: .75rem; } .action-row { margin: 1rem 0; } .recording-actions button { flex: 0 1 20rem; } .metric-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 12rem), 1fr)); gap: .55rem; margin: 1rem 0; } .metric-grid span { min-width: 0; padding: .6rem .7rem; border: 1px solid #d7dde2; border-radius: .3rem; background: #fff; overflow-wrap: anywhere; } .metric-grid strong { display: block; color: #42515d; font-size: .78rem; text-transform: uppercase; letter-spacing: .03em; }
  .plot-panel { min-width: 0; } .plot-heading { justify-content: space-between; align-items: center; } .toggle { display: flex; align-items: center; gap: .45rem; font-size: .9rem; }
  .paths p, .status { overflow-wrap: anywhere; word-break: break-word; } .paths p { margin: .35rem 0; } .status { margin: 1rem 0 0; padding: .7rem; border: 1px solid #d7dde2; background: #fff; border-radius: .3rem; } .diagnostic-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 16rem), 1fr)); gap: .75rem; }
  @media (max-width: 900px) { .workspace { grid-template-columns: 1fr; } .navigation { padding: .65rem; } nav { grid-template-columns: repeat(4, minmax(0, 1fr)); } nav button { text-align: center; padding-inline: .35rem; } }
  @media (max-width: 650px) { .app-header { align-items: flex-start; } .app-header img { max-width: 130px; } nav { grid-template-columns: repeat(2, minmax(0, 1fr)); } .content { padding: 1rem; } .recording-actions button { flex-basis: 100%; } .plot-heading { align-items: flex-start; } }
</style>
