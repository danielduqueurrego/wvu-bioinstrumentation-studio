<script lang="ts">
  import { formatClock } from '$lib/recording-status';
  import type { FirmwareCompatibility } from '$lib/board-controls';
  import type {
    FirmwareEnvironment,
    FirmwareWorkflowStatus,
    PlotErrorRecord,
    SessionStatus,
    StartFailureRecord
  } from '$lib/session-types';

  /**
   * The raw diagnostics behind the plain-language messages: backend
   * categories, fault origin, transport details, and the last start or plot
   * failure. For instructors and troubleshooting; read-only.
   */
  type Props = {
    session: SessionStatus;
    firmwareCompatibility: FirmwareCompatibility;
    firmwareWorkflow: FirmwareWorkflowStatus;
    firmwareEnvironment: FirmwareEnvironment;
    arduinoToolsState: 'preparing' | 'ready' | 'failed';
    arduinoToolsDetail: string;
    boardScanLastCompleted: string;
    boardScanError: string;
    lastStartFailure?: StartFailureRecord;
    lastPlotError?: PlotErrorRecord;
  };

  let {
    session,
    firmwareCompatibility,
    firmwareWorkflow,
    firmwareEnvironment,
    arduinoToolsState,
    arduinoToolsDetail,
    boardScanLastCompleted,
    boardScanError,
    lastStartFailure,
    lastPlotError
  }: Props = $props();

  const yesNoUnknown = (value: boolean | undefined | null) => value === true ? 'yes' : value === false ? 'no' : 'unknown';
  const diagnostics = $derived(session.connection_diagnostics);
</script>

<details class="advanced-details">
  <summary>Advanced details</summary>
  <div class="diagnostic-grid">
    <p>Session state: {session.state}</p>
    {#if session.fault_origin}<p>Fault origin: {session.fault_origin}</p>{/if}
    {#if diagnostics?.failure_category}<p>Failure category: {diagnostics.failure_category}</p><p>Backend recommendation: {diagnostics.recommended_action}</p>{/if}
    <p>Protocol: {session.protocol_version}</p>
    <p>Firmware status: {firmwareCompatibility}</p>
    {#if firmwareWorkflow.verified_port}<p>Verified port: {firmwareWorkflow.verified_port}</p>{/if}
    {#if firmwareWorkflow.last_failure}<p>Last firmware failure: {firmwareWorkflow.last_failure.category} at {firmwareWorkflow.last_failure.stage}</p>{/if}
    {#if diagnostics?.firmware_build}<p>Firmware build: {diagnostics.firmware_build}</p>{/if}
    {#if diagnostics?.firmware_board_id}<p>Board ID: {diagnostics.firmware_board_id}</p>{/if}
    <p>Arduino tools: {firmwareEnvironment.cli_version ?? (arduinoToolsState === 'preparing' ? 'preparing' : 'not available')}</p>
    {#if firmwareEnvironment.problem}<p>Arduino tools problem: {firmwareEnvironment.problem}</p>{/if}
    {#if arduinoToolsDetail}<p>Arduino tools detail: {arduinoToolsDetail}</p>{/if}
    <p>Last board refresh: {boardScanLastCompleted || 'not yet completed'}</p>
    {#if boardScanError}<p>Board refresh error: {boardScanError}</p>{/if}
    <p>Received packets: {session.integrity.received_packets}</p>
    <p>CRC failures: {session.integrity.crc_failures}</p>
    <p>Elapsed (host clock): {formatClock(session.elapsed_seconds)}</p>
    {#if diagnostics?.terminal_error_stage}
      <p>Capture failure stage: {diagnostics.terminal_error_stage}</p>
      <p>Capture failure type: {diagnostics.terminal_error_classification}</p>
      <p>I/O error kind: {diagnostics.terminal_error_kind}</p>
      <p>Windows error: {diagnostics.terminal_error_raw_os_error ?? 'none'}</p>
      <p>Capture error detail: {diagnostics.terminal_error_detail}</p>
      <p>Capture failure elapsed: {diagnostics.terminal_error_elapsed_ms} ms</p>
      <p>Port still listed: {yesNoUnknown(diagnostics.selected_port_present_after_error)}</p>
      <p>Same VID/PID listed: {yesNoUnknown(diagnostics.same_vid_pid_present_after_error)}</p>
      <p>Same USB serial listed: {yesNoUnknown(diagnostics.same_serial_present_after_error)}</p>
      <p>UNO R4 listed: {yesNoUnknown(diagnostics.uno_r4_present_after_error)}</p>
      {#if diagnostics.port_enumeration_error}<p>Port enumeration error: {diagnostics.port_enumeration_error}</p>{/if}
      <p>Last valid packet: {diagnostics.last_valid_packet_utc ?? 'not recorded'}</p>
      <p>Last valid sample: {diagnostics.last_valid_sample_utc ?? 'not recorded'}</p>
      <p>Last successful PING: {diagnostics.last_successful_ping_utc ?? 'not recorded'}</p>
      <p>Last PONG/status: {diagnostics.last_pong_or_status_utc ?? 'not recorded'}</p>
    {/if}
    {#if lastStartFailure}
      <p>Last recording start: {lastStartFailure.timestamp}</p>
      <p>Stage: {lastStartFailure.stage}</p>
      <p>Code: {lastStartFailure.code}</p>
      <p>Board: {lastStartFailure.port}</p>
      <p>Lab: {lastStartFailure.lab}</p>
      <p>Detail: {lastStartFailure.technicalDetail}</p>
    {/if}
    {#if lastPlotError}
      <p>Last live-plot error: {lastPlotError.timestamp}</p>
      <p>Plot stage: {lastPlotError.stage}</p>
      <p>Plot detail: {lastPlotError.detail}</p>
    {/if}
    {#if session.last_summary}<p>Last recording stop reason: {session.last_summary.stop_reason} ({session.last_summary.recording_status})</p>{/if}
    {#if session.last_error}<p>Recent connection error: {session.last_error}</p>{/if}
  </div>
</details>

<style>
  .advanced-details {
    min-width: 0;
    margin: 1rem 0;
    padding: clamp(.8rem, 2vw, 1rem);
    background: var(--surface, #fff);
    border: 1px solid var(--border, #d7dde2);
    border-radius: .35rem;
  }
  summary { cursor: pointer; color: var(--wvu-navy, #002855); font-weight: 700; }
  .advanced-details[open] > summary { margin-bottom: .75rem; }
  .diagnostic-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 16rem), 1fr)); gap: .75rem; }
  .diagnostic-grid p { margin: 0; overflow-wrap: anywhere; }
</style>
