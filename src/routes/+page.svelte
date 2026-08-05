<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import LivePlot from '$lib/components/LivePlot.svelte';
  import logoUrl from '../../assets/branding/WVU-CBE Logo.svg';

  type Point = { sequence: number; timestamp_us: number; counts: number };
  type Board = { port: string; name: string; fqbn: string; serial_number?: string };
  type Integrity = {
    received_packets: number; crc_failures: number; invalid_frames: number; unsupported_versions: number;
    missing_packet_sequences: number; duplicate_packets: number; out_of_order_packets: number;
    missing_sample_sequences: number; duplicate_sample_sequences: number; out_of_order_sample_sequences: number;
    firmware_overflows: number; host_channel_overflows: number; reconnects: number; disconnect_events: number;
  };
  type Summary = {
    state: string; samples: number; packets: number; measured_rate_hz: number;
    board_elapsed_seconds: number; host_elapsed_seconds: number; bmeg_path: string; csv_path: string;
    metadata_path: string; recording_status: string; integrity: Integrity; error?: string;
  };
  type SessionStatus = {
    state: string; board: string; port: string; protocol_version: string; simulator: boolean;
    samples: number; packets: number; measured_rate_hz: number; integrity: Integrity;
    last_error?: string; last_summary?: Summary;
  };

  const emptyIntegrity: Integrity = {
    received_packets: 0, crc_failures: 0, invalid_frames: 0, unsupported_versions: 0,
    missing_packet_sequences: 0, duplicate_packets: 0, out_of_order_packets: 0,
    missing_sample_sequences: 0, duplicate_sample_sequences: 0, out_of_order_sample_sequences: 0,
    firmware_overflows: 0, host_channel_overflows: 0, reconnects: 0, disconnect_events: 0
  };
  let view = 'Home';
  let samples: Point[] = [];
  let boards: Board[] = [];
  let selectedPort = '';
  let source: 'simulator' | 'hardware' = 'simulator';
  let outputDirectory = 'recordings';
  let durationSeconds = 5;
  let note = 'Simulator waveform; no human signal.';
  let volts = false;
  let statusMessage = 'Ready. Simulator uses the same Rust session, parser, recording, and export path.';
  let session: SessionStatus = {
    state: 'Disconnected', board: '', port: '', protocol_version: '0.1', simulator: false,
    samples: 0, packets: 0, measured_rate_hz: 0, integrity: emptyIntegrity
  };
  let polling = false;

  $: if (source === 'hardware') {
    durationSeconds = Math.max(durationSeconds, 60);
    note = 'A0 raw floating/uncalibrated engineering communication test; no human signal.';
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

  async function startRecording() {
    statusMessage = 'Connecting through the Rust production session controller…';
    try {
      session = source === 'simulator'
        ? await invoke<SessionStatus>('start_simulator_recording', { outputDirectory, seconds: durationSeconds })
        : await invoke<SessionStatus>('start_hardware_recording', { port: selectedPort, outputDirectory, seconds: durationSeconds });
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

  async function exportCsv() {
    try {
      const csvPath = await invoke<string>('export_session_csv');
      statusMessage = `CSV was streamed from the finalized BMEG recording: ${csvPath}`;
    } catch (error) {
      statusMessage = `Export error: ${String(error)}`;
    }
  }

  onMount(() => {
    void refreshBoards();
    void pollSession();
    const timer = window.setInterval(() => void pollSession(), 40); // 25 Hz, never one event per ADC sample.
    return () => window.clearInterval(timer);
  });
</script>

<svelte:head><title>WVU Bioinstrumentation Studio</title></svelte:head>

<main>
  <header>
    <img src={logoUrl} alt="Approved WVU College of Business and Economics logo" />
    <div><h1>WVU Bioinstrumentation Studio</h1><p>Firmware, Acquisition, and Calibration for BMEG 420L</p></div>
  </header>
  <aside>
    <nav aria-label="Primary">
      {#each ['Home', 'Firmware', 'Acquisition', 'Diagnostics'] as item}
        <button class:active={view === item} onclick={() => view = item}>{item}</button>
      {/each}
    </nav>
  </aside>
  <section class="content">
    {#if view === 'Home'}
      <h2>Home</h2>
      <p class="notice">Teaching and engineering equipment only — not a medical device. Phase 1 permits Arduino-alone, simulator, or safe bench-signal work only.</p>
      <button onclick={refreshBoards}>Refresh supported UNO R4 WiFi boards</button>
      <button class="gold" onclick={() => { source = 'simulator'; view = 'Acquisition'; }}>Open simulator acquisition</button>
    {:else if view === 'Firmware'}
      <h2>Firmware</h2>
      <p>Phase 1 uses the approved safe UNO R4 WiFi template and Arduino CLI adapter. D4, D5, and D6 are initialized LOW and never driven HIGH.</p>
    {:else if view === 'Acquisition'}
      <h2>Acquisition</h2>
      <p class="notice">Raw A0 only. A floating A0 test is uncalibrated; no physiological interpretation or hidden filtering is performed.</p>
      <div class="controls">
        <label>Source <select bind:value={source}><option value="simulator">Simulator</option><option value="hardware">Hardware</option></select></label>
        <button onclick={refreshBoards} disabled={session.state !== 'Disconnected'}>Refresh devices</button>
        {#if source === 'hardware'}
          <label>UNO R4 WiFi port <select bind:value={selectedPort} disabled={session.state !== 'Disconnected'}>{#each boards as board}<option value={board.port}>{board.name} — {board.port} ({board.fqbn})</option>{/each}</select></label>
        {/if}
        <label>Output folder <input bind:value={outputDirectory} disabled={session.state !== 'Disconnected'} /></label>
        <label>Duration (s) <input type="number" min="1" max="600" bind:value={durationSeconds} disabled={session.state !== 'Disconnected'} /></label>
        <label>Test note <input bind:value={note} readonly /></label>
      </div>
      <div class="controls">
        <button class="gold" onclick={startRecording} disabled={session.state !== 'Disconnected' || (source === 'hardware' && !selectedPort)}>Connect, configure, and start recording</button>
        <button onclick={stopRecording} disabled={!['Connecting', 'Connected', 'Configured', 'Acquiring', 'Stopping'].includes(session.state)}>Stop recording</button>
        <button onclick={disconnect} disabled={session.state === 'Disconnected'}>Disconnect</button>
        <button onclick={exportCsv} disabled={!session.last_summary}>Show CSV export</button>
      </div>
      <div class="metrics">
        <span>State: {session.state}</span><span>Device: {session.board || 'not connected'} {session.port ? `(${session.port})` : ''}</span><span>Protocol: {session.protocol_version}</span>
        <span>Analog: A0</span><span>ADC: 12 bit</span><span>Requested: 1000 samples/s</span><span>Display: 25 Hz batch</span>
        <span>Elapsed board: {session.last_summary?.board_elapsed_seconds?.toFixed(3) ?? '—'} s</span><span>Samples: {session.samples}</span><span>Measured: {session.measured_rate_hz.toFixed(3)} Hz</span>
        <span>Valid packets: {session.integrity.received_packets}</span><span>CRC failures: {session.integrity.crc_failures}</span><span>Missing packets: {session.integrity.missing_packet_sequences}</span><span>Missing samples: {session.integrity.missing_sample_sequences}</span>
        <span>Duplicate / out-of-order packets: {session.integrity.duplicate_packets} / {session.integrity.out_of_order_packets}</span><span>Firmware / host overflows: {session.integrity.firmware_overflows} / {session.integrity.host_channel_overflows}</span>
        <span>Reconnects / disconnects: {session.integrity.reconnects} / {session.integrity.disconnect_events}</span>
      </div>
      <label><input type="checkbox" bind:checked={volts} /> Display volts (counts × 5.0 / 4095.0)</label>
      <LivePlot {samples} {volts} />
      {#if session.last_summary}<p class="paths">{session.last_summary.bmeg_path}<br />{session.last_summary.metadata_path}<br />{session.last_summary.csv_path}</p>{/if}
    {:else}
      <h2>Diagnostics</h2>
      <p>Current state: <strong>{session.state}</strong></p>
      <p>Last error: {session.last_error ?? 'none'}</p>
      <p>Serial ownership, packet validation, bounded display data, recording, and export run in Rust; the frontend only polls snapshots.</p>
    {/if}
    <p class="status" aria-live="polite">{statusMessage}</p>
  </section>
</main>

<style>
  :global(body) { margin: 0; font-family: Segoe UI, Arial, sans-serif; background: #F7F7F7; color: #17222e; }
  main { display: grid; grid-template-columns: 210px 1fr; min-height: 100vh; }
  header { grid-column: 1 / -1; background: #002855; color: #F7F7F7; display: flex; align-items: center; gap: 18px; padding: 12px 28px; }
  header img { width: 150px; max-height: 72px; object-fit: contain; } h1 { font-size: 1.4rem; margin: 0; } header p { margin: .2rem 0 0; }
  aside { background: #e8edf1; padding: 20px 12px; } nav { display: grid; gap: 7px; }
  button { border: 1px solid #002855; border-radius: 4px; background: #fff; color: #002855; padding: .6rem .8rem; font-weight: 600; text-align: left; cursor: pointer; }
  button:disabled { cursor: not-allowed; opacity: .55; } button.active, button:not(:disabled):hover { background: #002855; color: #fff; } button.gold { background: #EEAA00; color: #17222e; }
  .content { padding: 28px; max-width: 1080px; } .notice { border-left: 5px solid #EEAA00; background: #fff8e8; padding: 12px; }
  .controls { display: flex; align-items: end; gap: 10px; flex-wrap: wrap; margin: 12px 0; } label { display: grid; gap: 4px; } input, select { min-height: 28px; }
  .metrics { display: flex; gap: 8px; flex-wrap: wrap; margin: 14px 0; } .metrics span, .status, .paths { background: #fff; border: 1px solid #d7dde2; padding: 7px 10px; border-radius: 4px; }
  .status { margin-top: 18px; } .paths { overflow-wrap: anywhere; line-height: 1.5; }
  @media (max-width: 760px) { main { grid-template-columns: 1fr; } header { grid-column: auto; } aside { padding: 10px; } nav { grid-template-columns: repeat(4, 1fr); } .content { padding: 18px; } }
</style>
