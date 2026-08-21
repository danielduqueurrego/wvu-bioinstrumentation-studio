<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { confirm, open } from '@tauri-apps/plugin-dialog';
  import LivePlot from '$lib/components/LivePlot.svelte';
  import LabManager from '$lib/components/LabManager.svelte';
  import type { LabProfile } from '$lib/labs';
  import OperatingModeControl from '$lib/components/OperatingModeControl.svelte';
  import { connectionActions } from '$lib/connection-actions';
  import { durationRequest, isTimedDurationValid, type RecordingDurationRequest } from '$lib/duration';
  import {
    displayUnitLabel,
    calibrationById,
    fixedMpxvCalibration,
    initialChannelUnits,
    mergeCalibrationPresets,
    supportedDisplayUnits,
    unitsForGroup,
    xgzpFitRequestPayload,
    type CalibrationPreset,
    type DisplayUnit,
    type RecordingCalibration
  } from '$lib/calibration';
  import type { OperatingMode } from '$lib/operating-mode';
  import {
    assignChannelToPlot,
    defaultPlotGroups,
    initialTraceVisibility,
    normalizePlotGroups,
    onePlotPerSignal,
    overlayAll,
    setPlotGroupCount,
    setTraceVisibility,
    visiblePlotGroups,
    visibleChannelIds,
    type PlotGroup,
    type VisibleChannelMap
  } from '$lib/multichannel';
  import { reconcileBoardCache } from '$lib/board-cache';
  import { boardControls } from '$lib/board-controls';
  import { hardwareStartInvokePayload, recordingStartFailure, recordingStartReadiness, type RecordingStartFailure, type RecordingStartReadiness } from '$lib/recording-start';
  import { effectiveRecordingFolder, relativeOutputFolderError } from '$lib/project-folder';
  import {
    DEFAULT_PLOT_TIME_WINDOW_SECONDS,
    MAX_RENDERED_DISPLAY_POINTS,
    normalizePlotTimeWindow,
    previewPlotTimeWindow
  } from '$lib/live-display';
  import logoUrl from '../../assets/branding/WVU-CBE Logo.svg';

  type Point = { sequence: number; timestamp_us: number; values: number[]; status_flags: number };
  type Board = {
    port: string; name: string; fqbn: string; serial_number?: string;
    usb_vid?: number; usb_pid?: number;
  };
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
    integrity: Integrity; error?: string; profile?: ProfileSnapshot; calibration?: RecordingCalibration;
    active_digital_output_mask?: number; final_digital_output_mask?: number;
  };
  type SessionStatus = {
    state: string; board: string; port: string; protocol_version: string; simulator: boolean;
    samples: number; packets: number; measured_rate_hz: number; integrity: Integrity;
    duration?: Duration; elapsed_seconds: number; remaining_seconds?: number;
    available_disk_bytes?: number; storage_warning?: string; stop_reason?: string;
    connection_diagnostics?: ConnectionDiagnostics;
    last_error?: string; last_summary?: Summary; calibration?: RecordingCalibration;
    digital_output_mask?: number;
    display_origin_timestamp_us?: number;
  };
  type ConnectionDiagnostics = {
    selected_port: string; board: string; fqbn: string; port_opened: boolean;
    bytes_received: number; valid_frames: number; crc_failures: number; skipped_noise_bytes: number;
    hello_received: boolean; capabilities_received: boolean; pong_received: boolean;
    protocol_version?: string; firmware_build?: number; firmware_board_id?: number; raw_byte_classification: string; ping_attempts: number; handshake_elapsed_ms: number;
    reset_attempted: boolean; original_port?: string; final_port?: string;
    disappearance_observed: boolean; reappearance_observed: boolean; bootloader_observed: boolean;
    failure_category?: string; recommended_action: string;
    terminal_error_classification?: string; terminal_error_stage?: string;
    terminal_error_kind?: string; terminal_error_raw_os_error?: number;
    terminal_error_detail?: string; terminal_error_elapsed_ms?: number;
    last_valid_packet_utc?: string; last_valid_sample_utc?: string;
    last_successful_ping_utc?: string; last_pong_or_status_utc?: string;
    selected_port_present_after_error?: boolean; same_vid_pid_present_after_error?: boolean;
    same_serial_present_after_error?: boolean; uno_r4_present_after_error?: boolean;
    port_enumeration_error?: string;
  };
  type ResetRetryResult = {
    original_port: string; final_port?: string; handshake_succeeded: boolean;
    diagnostics: ConnectionDiagnostics;
  };
  type HandshakeRetryResult = { handshake_succeeded: boolean; diagnostics: ConnectionDiagnostics };
  type FirmwareJob = { kind: string; stage: string; active: boolean; message: string };
  type FirmwareWorkflowStatus = { compatibility: string; job?: FirmwareJob };
  type FirmwareVerification = { declared_kind: string; compatible: boolean; protocol_version?: string; identity?: { protocol_version: string; firmware_build: number; device_id: number }; bytes_received?: number; valid_frames?: number; crc_failures?: number; explanation: string };
  type FirmwareEnvironment = { cli_path?: string; cli_version?: string; uno_r4_core_version?: string; expected_fqbn: string; boards: Board[]; ready: boolean; problem?: string };
  type ArduinoRuntimeStatus = { ready: boolean; cli_version: string; core_version: string; message: string };
  type ActiveOperation = { title: string; stage: string; cancelable: boolean };
  type AcquisitionProfile = LabProfile;
  type ProfileSnapshot = { bench_notice_acknowledged: boolean; profile: AcquisitionProfile };

  const emptyIntegrity: Integrity = {
    received_packets: 0, crc_failures: 0, invalid_frames: 0, unsupported_versions: 0,
    missing_packet_sequences: 0, duplicate_packets: 0, out_of_order_packets: 0,
    missing_sample_sequences: 0, duplicate_sample_sequences: 0, out_of_order_sample_sequences: 0,
    firmware_overflows: 0, host_channel_overflows: 0, reconnects: 0, disconnect_events: 0
  };
  const activeStates = ['Connecting', 'Connected', 'Configured', 'Acquiring', 'Stopping'];
  const yesNoUnknown = (value: boolean | undefined | null) => value === true ? 'yes' : value === false ? 'no' : 'unknown';

  let samples: Point[] = [];
  let displayRevision = 0;
  // One display-only time extent is shared by every rendered plot group. It
  // changes only the bounded display query, never the active acquisition.
  let plotTimeWindowSeconds = DEFAULT_PLOT_TIME_WINDOW_SECONDS;
  let plotTimeWindowInput = String(DEFAULT_PLOT_TIME_WINDOW_SECONDS);
  let boards: Board[] = [];
  let selectedPort = '';
  let selectedBoardUsesNativeUsb = false;
  let boardScanStatus: 'idle' | 'scanning' | 'complete' | 'error' = 'idle';
  let boardScanLastCompleted = '';
  let boardScanError = '';
  let boardScanInFlight = false;
  let activeOperation: ActiveOperation | undefined;
  let firmwareEnvironment: FirmwareEnvironment = { expected_fqbn: 'arduino:renesas_uno:unor4wifi', boards: [], ready: false };
  let source: 'simulator' | 'hardware' = 'simulator';
  let projectFolder = '';
  let outputDirectory = '';
  let durationMode: 'timed' | 'until_stopped' = 'timed';
  let durationPreset = '60';
  let customSeconds = 60;
  let duration: Duration = { mode: 'timed', seconds: 60 };
  let note = 'Simulator waveform; no human signal.';
  let adcReferenceV = 5;
  let mpxvSensorSupplyV = 5;
  let channelUnits: Record<string, DisplayUnit> = {};
  let channelUnitOptions: Record<string, DisplayUnit[]> = {};
  let currentRecordingCalibration: RecordingCalibration = {
    adc_reference_v: 5,
    mpxv_sensor_supply_v: 5,
    channel_units: {},
    active_calibrations: []
  };
  let savedCalibrations: CalibrationPreset[] = [];
  let calibrationChannelId = '';
  let selectedXgzpCalibrationId = '';
  // This is the calibration actually applied to live display/future recording.
  // It is deliberately an object, not only an ID-derived reactive value, so a
  // successful save can take effect immediately even while a list refresh runs.
  let activeXgzpCalibration: CalibrationPreset | undefined;
  let calibrationDialogOpen = false;
  let calibrationMethod: 'xgzp_recording' | 'manual_points' = 'xgzp_recording';
  let calibrationStartSeconds = 0;
  let calibrationEndSeconds = 10;
  let calibrationLabel = 'XGZP calibration';
  let manualCalibrationPoints = '0.80, 20\n1.20, 60\n1.60, 100';
  let manualCalibrationQuantity = 'pressure';
  let manualCalibrationUnits = 'mmHg';
  let calibrationFit: { slope: number; offset: number; r_squared: number; paired_samples: number } | undefined;
  let calibrationError = '';
  let statusMessage = 'Ready.';
  let startFeedback = '';
  let lastStartFailure: (RecordingStartFailure & { timestamp: string; port: string; lab: string }) | undefined;
  let lastPlotError: { timestamp: string; stage: string; detail: string } | undefined;
  let startInFlight = false;
  let displayedSummaryPath = '';
  let startReadiness: RecordingStartReadiness = { canStart: false };
  let arduinoToolsReady = false;
  let session: SessionStatus = {
    state: 'Disconnected', board: '', port: '', protocol_version: '0.1', simulator: false,
    samples: 0, packets: 0, measured_rate_hz: 0, integrity: emptyIntegrity,
    elapsed_seconds: 0
  };
  let polling = false;
  let firmwareCompatibility = 'unknown';
  let firmwareWorkflow: FirmwareWorkflowStatus = { compatibility: 'unknown' };
  let acquisitionProfiles: AcquisitionProfile[] = [];
  let selectedProfileId = 'wvu.bmeg420l.general.analog.development.v2';
  let operatingMode: OperatingMode = 'student';
  let modeChangeInFlight = false;
  let benchNoticeAcknowledged = false;
  let instructorAcknowledgement = false;
  let traceVisibility: VisibleChannelMap = {};
  let traceProfileKey = '';
  let plotGroups: PlotGroup[] = [];

  $: if (source === 'hardware') {
    note = 'Arduino data source — follow the assigned lab instructions.';
  } else {
    note = 'Simulator — no Arduino data are being recorded.';
  }
  $: timedSeconds = durationPreset === 'custom' ? Number(customSeconds) : Number(durationPreset);
  $: activeProfile = acquisitionProfiles.find((profile) => profile.profile_id === selectedProfileId);
  $: activeChannels = activeProfile?.acquisition.channels?.length
    ? activeProfile.acquisition.channels
    : activeProfile ? [{ pin: activeProfile.acquisition.analog_pin, id: 'raw', label: activeProfile.display.channel_label, csv_name: activeProfile.export.signal_name, units: 'ADC counts' }] : [];
  $: pulseoxProfile = activeProfile?.acquisition.acquisition_mode === 'pulseox_4state';
  $: plotChannels = pulseoxProfile
    ? [
      { id: 'red_tx', label: 'TX Red', csv_name: 'red_TX', allowed_conversions: ['counts_volts'] },
      { id: 'dark1_tx', label: 'TX Dark 1', csv_name: 'dark1_TX', allowed_conversions: ['counts_volts'] },
      { id: 'ir_tx', label: 'TX IR', csv_name: 'ir_TX', allowed_conversions: ['counts_volts'] },
      { id: 'dark2_tx', label: 'TX Dark 2', csv_name: 'dark2_TX', allowed_conversions: ['counts_volts'] },
      { id: 'red_rx', label: 'RX Red', csv_name: 'red_RX', allowed_conversions: ['counts_volts'] },
      { id: 'dark1_rx', label: 'RX Dark 1', csv_name: 'dark1_RX', allowed_conversions: ['counts_volts'] },
      { id: 'ir_rx', label: 'RX IR', csv_name: 'ir_RX', allowed_conversions: ['counts_volts'] },
      { id: 'dark2_rx', label: 'RX Dark 2', csv_name: 'dark2_RX', allowed_conversions: ['counts_volts'] }
    ]
    : activeChannels;
  $: linearCalibrationChannels = activeChannels.filter((channel) =>
    channel.allowed_conversions?.includes('linear_calibration') || channel.id === 'xgzp'
  );
  $: hasMpxvChannel = activeChannels.some((channel) =>
    channel.allowed_conversions?.includes('mpxv_pressure')
      || channel.id === 'mpxv'
      || (activeProfile?.category === 'course_emg_force' && channel.id === 'pressure')
  );
  $: if (!linearCalibrationChannels.some((channel) => channel.id === calibrationChannelId)) {
    calibrationChannelId = linearCalibrationChannels[0]?.id ?? '';
  }
  $: plotProfileKey = `${activeProfile?.profile_id ?? ''}:${pulseoxProfile ? 'pulseox-raw' : 'analog'}:${plotChannels.map((channel) => channel.id).join('|')}`;
  // Visibility is reset only when the selected profile's display fields change.  It is
  // never reconciled during a live checkbox toggle, which keeps checkbox DOM state and
  // uPlot series derived from one authoritative map.
  $: if (plotProfileKey !== traceProfileKey) {
    traceProfileKey = plotProfileKey;
    traceVisibility = initialTraceVisibility(plotChannels);
    plotGroups = pulseoxProfile
      ? defaultPlotGroups(activeProfile?.category, plotChannels)
      : activeProfile?.plot_defaults?.groups?.length
      ? normalizePlotGroups(plotChannels, activeProfile.plot_defaults.groups.map((group, index) => ({ id: `profile-${index + 1}`, channelIds: group.channel_ids })))
      : defaultPlotGroups(activeProfile?.category, plotChannels);
    channelUnits = initialChannelUnits(plotChannels.map((channel) => channel.id));
    savedCalibrations = [];
    selectedXgzpCalibrationId = '';
    activeXgzpCalibration = undefined;
  }
  $: visibleTraceIds = visibleChannelIds(plotChannels, traceVisibility);
  $: renderedPlotGroups = visiblePlotGroups(plotChannels, plotGroups, traceVisibility);
  // Keep this as a reactive value used directly by the template.  A helper
  // called only from markup can hide its dependencies from Svelte's legacy
  // compiler and leave <option> elements stale after a calibration changes.
  $: channelUnitOptions = Object.fromEntries(plotChannels.map((channel) => [
    channel.id,
    supportedDisplayUnits(
      activeProfile?.category,
      channel.id,
      Boolean(activeXgzpCalibration && selectedXgzpCalibrationId && activeXgzpCalibration.channel_id === channel.id),
      channel.allowed_conversions ?? [],
      activeXgzpCalibration?.channel_id === channel.id ? activeXgzpCalibration.output_units : 'mmHg'
    )
  ]));
  // This is intentionally a direct reactive expression rather than a markup
  // helper call. It gives the live uPlot components the same fresh calibration
  // snapshot that a subsequent Start command will freeze into metadata.
  $: currentRecordingCalibration = buildRecordingCalibration(
    activeProfile,
    activeChannels,
    activeXgzpCalibration,
    Number(adcReferenceV),
    Number(mpxvSensorSupplyV),
    channelUnits
  );
  $: instructorModeActive = operatingMode === 'instructor_authoring';
  $: profileTimedPresets = activeProfile?.acquisition.timed_presets_seconds ?? [10, 30, 60, 300, 600];
  $: timedDurationValid = isTimedDurationValid(timedSeconds);
  $: duration = durationRequest(durationMode, timedSeconds);
  $: outputFolderError = relativeOutputFolderError(outputDirectory);
  $: isActive = activeStates.includes(session.state);
  // A failed verification deliberately leaves the session Faulted so its
  // diagnostics are available. Faulted does not own a serial handle and must
  // therefore still permit board selection, retry, and firmware restoration.
  $: boardOperationBusy = boardScanInFlight
    || Boolean(activeOperation)
    || firmwareWorkflow.job?.active === true;
  $: boardControlState = boardControls({
    selectedBoard: Boolean(selectedPort),
    recordingActive: isActive,
    boardOperationBusy,
    arduinoToolsReady,
    firmwareStatus: firmwareCompatibility
  });
  $: startReadiness = recordingStartReadiness({
    source,
    selectedBoard: Boolean(selectedPort),
    firmwareReady: firmwareCompatibility === 'wvu_protocol_compatible',
    sessionState: session.state,
    boardOperationBusy,
    startInFlight,
    activeProfile: Boolean(activeProfile),
    projectFolder: Boolean(projectFolder),
    outputFolderError,
    durationValid: durationMode === 'until_stopped' || timedDurationValid,
    acknowledgementSatisfied: !activeProfile?.safety.bench_only
      || !['ecg', 'emg'].includes(activeProfile.category)
      || benchNoticeAcknowledged
  });
  $: canStart = startReadiness.canStart;
  $: recoveryActions = connectionActions({
    source,
    active: isActive,
    selectedPort,
    failureCategory: session.connection_diagnostics?.failure_category
  });
  // The controlled native-USB reference uses the RA4M1 CDC port (PID 0x006D).
  // Arduino CLI cannot safely perform its regular 1200-bps touch through that
  // port. Restore WVU Firmware provides the explicit double-Reset fallback.
  $: selectedBoardUsesNativeUsb = boards.find((board) => board.port === selectedPort)?.usb_pid === 0x006d;
  $: canReset = recoveryActions.canReset && !selectedBoardUsesNativeUsb;
  $: canRetryHandshake = recoveryActions.canRetryHandshake;
  // BMEG records stream continuously. This deliberately conservative estimate includes
  // the profile-defined raw fields plus a CSV of the same session; it is guidance,
  // not a storage-limit calculation.
  $: estimatedMegabytesPerMinute = activeProfile
    ? activeProfile.acquisition.sample_rate_hz
      * 60
      * (14 + (activeProfile.acquisition.acquisition_mode === 'pulseox_4state'
        ? 16
        : Math.max(1, activeChannels.length) * 2))
      * 2.5
      / (1024 * 1024)
    : 0;
  $: effectiveOutputFolder = effectiveRecordingFolder(projectFolder, outputDirectory);

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

  async function runOperation<T>(operation: ActiveOperation, action: () => Promise<T>): Promise<T> {
    const prior = activeOperation;
    activeOperation = operation;
    try {
      return await action();
    } finally {
      activeOperation = prior;
    }
  }

  async function refreshEnvironmentSummary() {
    try {
      firmwareEnvironment = await invoke<FirmwareEnvironment>('firmware_environment');
      arduinoToolsReady = firmwareEnvironment.ready;
    } catch (error) {
      firmwareEnvironment = {
        expected_fqbn: 'arduino:renesas_uno:unor4wifi', boards: [], ready: false,
        problem: 'Arduino tools could not be prepared. Reinstall the application or ask your instructor for help.'
      };
      arduinoToolsReady = false;
    }
  }

  async function prepareArduinoTools() {
    try {
      const runtime = await runOperation(
        { title: 'Preparing Arduino tools…', stage: 'Setting up the included offline Arduino tools for this computer.', cancelable: false },
        () => invoke<ArduinoRuntimeStatus>('prepare_arduino_runtime')
      );
      arduinoToolsReady = runtime.ready;
      if (!runtime.ready) {
        firmwareEnvironment = {
          expected_fqbn: 'arduino:renesas_uno:unor4wifi', boards: [], ready: false,
          problem: 'Arduino tools are not ready. Reinstall the application or ask your instructor for help.'
        };
      }
    } catch (error) {
      arduinoToolsReady = false;
      firmwareEnvironment = {
        expected_fqbn: 'arduino:renesas_uno:unor4wifi', boards: [], ready: false,
        problem: 'Arduino tools could not be prepared. Reinstall the application or ask your instructor for help.'
      };
      statusMessage = `Arduino tools need attention. ${String(error)}`;
    }
  }

  async function verifySelectedFirmware(
    port = selectedPort,
    allowWithinBoardOperation = false
  ): Promise<FirmwareVerification | undefined> {
    if (!port || isActive || (!allowWithinBoardOperation && !boardControlState.canVerifyFirmware)) return undefined;
    return runOperation(
      { title: 'Verifying firmware…', stage: 'Opening the selected UNO R4 WiFi and performing a read-only WVU protocol handshake.', cancelable: false },
      async () => {
        try {
          const verification = await invoke<FirmwareVerification>('verify_wvu_reference_firmware', { port });
          await refreshFirmwareCompatibility();
          await pollSession();
          statusMessage = verification.compatible
            ? `Firmware ready on ${port}.`
            : 'The Arduino was detected, but its WVU firmware is not ready. Use Restore WVU Firmware above if needed.';
          return verification;
        } catch (error) {
          await refreshFirmwareCompatibility();
          statusMessage = 'The Arduino could not be verified. Refresh Board, then try Restore WVU Firmware if the problem continues.';
          return undefined;
        }
      }
    );
  }

  async function refreshBoards(reason: 'startup' | 'manual' | 'transition' = 'manual') {
    if (boardScanInFlight || (reason === 'manual' && !boardControlState.canRefreshBoards)) return;
    boardScanInFlight = true;
    boardScanStatus = 'scanning';
    boardScanError = '';
    try {
      await runOperation(
          { title: 'Detecting Arduino boards…', stage: reason === 'startup' ? 'Looking for a connected Arduino UNO R4 WiFi.' : 'Refreshing the list of connected Arduino boards.', cancelable: false },
        async () => {
          const scan = reconcileBoardCache(selectedPort, await invoke<Board[]>('list_boards'));
          boards = scan.boards;
          boardScanLastCompleted = new Date().toLocaleTimeString();
          boardScanStatus = 'complete';
          selectedPort = scan.selectedPort;
          if (scan.verificationPort) await verifySelectedFirmware(scan.verificationPort, true);
          statusMessage = boards.length
            ? `${boards.length} Arduino UNO R4 WiFi board${boards.length === 1 ? '' : 's'} found${selectedPort ? `; ${selectedPort} is selected.` : '. Select a board to continue.'}`
            : 'Arduino not detected. Check the USB connection, then select Refresh Board.';
        }
      );
    } catch (error) {
      boardScanStatus = 'error';
      boardScanError = String(error);
      statusMessage = 'The board list could not be refreshed. Check the USB connection and try Refresh Board again.';
    } finally {
      boardScanInFlight = false;
    }
  }

  async function pollSession() {
    if (polling) return;
    polling = true;
    try {
      session = await invoke<SessionStatus>('get_session_status');
      samples = await invoke<Point[]>('get_recent_display_data', {
        windowSeconds: plotTimeWindowSeconds,
        maxPoints: MAX_RENDERED_DISPLAY_POINTS
      });
      displayRevision += 1;
      if (session.state === 'Faulted' && session.last_error) {
        statusMessage = 'The Arduino connection needs attention. Verify the firmware or refresh the board before starting a new recording.';
      }
      if (session.last_summary && session.last_summary.bmeg_path !== displayedSummaryPath) {
        displayedSummaryPath = session.last_summary.bmeg_path;
        statusMessage = `${session.last_summary.recording_status}: ${session.last_summary.samples} validated samples at ${session.last_summary.measured_rate_hz.toFixed(3)} Hz.`;
      }
    } catch (error) {
      statusMessage = 'The recording status could not be updated. Try reconnecting the Arduino.';
    } finally {
      polling = false;
    }
  }

  async function refreshFirmwareCompatibility() {
    try {
      const firmwareJobWasActive = firmwareWorkflow.job?.active === true;
      firmwareWorkflow = await invoke<FirmwareWorkflowStatus>('get_firmware_workflow_status');
      firmwareCompatibility = firmwareWorkflow.compatibility;
      if (firmwareJobWasActive && !firmwareWorkflow.job?.active) {
        // A native-USB restore intentionally returns on a different COM port
        // from the ESP32 bridge used for upload. Refresh exactly once after a
        // terminal firmware job so the selected board cannot remain stale.
        await refreshBoards('transition');
        // Refresh the independent acquisition state immediately as well; it
        // must not wait for a route change or retain a stale Faulted snapshot.
        await pollSession();
      }
    } catch {
      firmwareCompatibility = 'unknown';
      firmwareWorkflow = { compatibility: 'unknown' };
    }
  }

  async function cancelFirmwareJob() {
    try {
      await invoke('cancel_firmware_job');
      await refreshFirmwareCompatibility();
    } catch (error) {
      statusMessage = `Firmware job cancellation error: ${String(error)}`;
    }
  }

  async function selectedBoardChanged() {
    if (selectedPort && boardControlState.canVerifyFirmware) await verifySelectedFirmware(selectedPort);
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

  async function chooseProjectFolder() {
    const selected = await open({ directory: true, multiple: false, defaultPath: projectFolder || undefined });
    if (typeof selected !== 'string') return;
    try {
      const saved = await invoke<{ project_folder: string }>('set_project_folder', { projectFolder: selected });
      projectFolder = saved.project_folder;
      statusMessage = 'Project folder updated. New recordings will use this location.';
    } catch (error) {
      statusMessage = `Project folder could not be used: ${String(error)}`;
    }
  }

  async function restoreWvuFirmware() {
    if (!boardControlState.canRestoreFirmware) return;
    const approved = await confirm(
      `Restore WVU Firmware on ${selectedPort}? This replaces the current Arduino sketch.`,
      { title: 'Restore WVU Firmware', kind: 'warning', okLabel: 'Restore firmware', cancelLabel: 'Cancel' }
    );
    if (!approved) return;
    try {
      firmwareWorkflow = {
        ...firmwareWorkflow,
        job: await invoke<FirmwareJob>('restore_wvu_reference_firmware', { request: { port: selectedPort, confirmation: true } })
      };
      statusMessage = 'Restoring WVU Firmware. The board will be verified when the update finishes.';
    } catch (error) {
      statusMessage = `WVU Firmware could not be restored: ${String(error)}`;
    }
  }

  async function labRevisionSaved(profile: AcquisitionProfile) {
    await refreshProfiles();
    selectedProfileId = profile.profile_id;
    traceProfileKey = '';
    selectProfile();
  }

  async function refreshCalibrations() {
    if (!activeProfile || !calibrationChannelId) {
      savedCalibrations = [];
      selectedXgzpCalibrationId = '';
      activeXgzpCalibration = undefined;
      return;
    }
    try {
      const stored = await invoke<CalibrationPreset[]>('list_calibrations', {
        profileId: activeProfile.profile_id,
        channelId: calibrationChannelId
      });
      // Keep a calibration that Rust has just accepted active in this session,
      // even if a file-system list refresh temporarily arrives before it.
      savedCalibrations = mergeCalibrationPresets(stored, savedCalibrations);
      const refreshedActive = calibrationById(savedCalibrations, selectedXgzpCalibrationId);
      if (refreshedActive) {
        activeXgzpCalibration = refreshedActive;
      } else if (activeXgzpCalibration?.calibration_id !== selectedXgzpCalibrationId) {
        selectedXgzpCalibrationId = '';
        activeXgzpCalibration = undefined;
      }
    } catch (error) {
      statusMessage = `Could not load local calibrations: ${String(error)}`;
    }
  }

  function buildRecordingCalibration(
    profile: AcquisitionProfile | undefined,
    channels: Array<{ id: string }>,
    xgzpCalibration: CalibrationPreset | undefined,
    adcReference: number,
    mpxvSupply: number,
    displayUnits: Record<string, DisplayUnit>
  ): RecordingCalibration {
    const activeCalibrations: CalibrationPreset[] = [];
    if (profile) {
      for (const channel of profile.acquisition.channels ?? []) {
        const fixedMpxv = channel.allowed_conversions?.includes('mpxv_pressure')
          || channel.id === 'mpxv'
          || (profile.category === 'course_emg_force' && channel.id === 'pressure');
        if (fixedMpxv) {
          activeCalibrations.push(fixedMpxvCalibration(profile.profile_id, channel.id, mpxvSupply, adcReference));
        }
      }
    }
    if (xgzpCalibration) activeCalibrations.push(xgzpCalibration);
    return {
      adc_reference_v: adcReference,
      mpxv_sensor_supply_v: mpxvSupply,
      channel_units: displayUnits,
      active_calibrations: activeCalibrations
    };
  }

  function channelUnitsAllowed(channelId: string): DisplayUnit[] {
    return channelUnitOptions[channelId] ?? ['counts', 'volts'];
  }

  function setChannelUnit(channelId: string, unit: string) {
    if (!channelUnitsAllowed(channelId).includes(unit as DisplayUnit)) {
      statusMessage = `${channelId} does not have that calibrated unit available.`;
      return;
    }
    channelUnits = { ...channelUnits, [channelId]: unit as DisplayUnit };
  }

  async function selectXgzpCalibration(calibrationId: string) {
    selectedXgzpCalibrationId = calibrationId;
    activeXgzpCalibration = calibrationById(savedCalibrations, calibrationId);
    if (activeXgzpCalibration) {
      // Selecting a calibration explicitly opts into its engineering-unit
      // display and freezes the same preset into the next recording.
      channelUnits = {
        ...channelUnits,
        [calibrationChannelId]: activeXgzpCalibration.output_units.trim().toLowerCase() === 'mmhg'
          ? 'mmhg'
          : 'calibrated'
      };
    } else if (channelUnits[calibrationChannelId] === 'mmhg') {
      channelUnits = { ...channelUnits, [calibrationChannelId]: 'volts' };
    }
    await refreshCalibrations();
  }

  async function selectCalibrationChannel(channelId: string) {
    calibrationChannelId = channelId;
    selectedXgzpCalibrationId = '';
    activeXgzpCalibration = undefined;
    savedCalibrations = [];
    await refreshCalibrations();
  }

  function parseManualPoints() {
    const points = manualCalibrationPoints.split(/\r?\n/).map((line) => line.trim()).filter(Boolean).map((line) => {
      const [inputVoltage, referenceValue] = line.split(',').map(Number);
      return { input_voltage: inputVoltage, reference_value: referenceValue };
    });
    if (points.length < 2 || points.some((point) => !Number.isFinite(point.input_voltage) || !Number.isFinite(point.reference_value))) {
      throw new Error('Enter at least two finite points, one “volts, reference value” pair per line.');
    }
    return points;
  }

  async function calculateCalibrationFit() {
    calibrationError = '';
    try {
      if (calibrationMethod === 'xgzp_recording') {
        const bmegPath = session.last_summary?.bmeg_path;
        if (!bmegPath || activeProfile?.category !== 'course_blood_pressure' || calibrationChannelId !== 'xgzp') {
          throw new Error('Finish a Blood Pressure + PPG recording before fitting XGZP against its synchronized MPXV channel.');
        }
        const fit = await invoke<{ slope: number; offset: number; r_squared: number; paired_samples: number }>('fit_xgzp_calibration', {
          request: xgzpFitRequestPayload(
            bmegPath,
            Number(calibrationStartSeconds),
            Number(calibrationEndSeconds),
            Number(adcReferenceV),
            Number(mpxvSensorSupplyV)
          )
        });
        calibrationFit = fit;
      } else {
        const fit = await invoke<{ slope: number; offset: number; r_squared: number; paired_samples: number }>('fit_manual_linear_calibration', { points: parseManualPoints() });
        calibrationFit = fit;
      }
      const fit = calibrationFit;
      if (fit) statusMessage = `Linear calibration fit: ${fit.paired_samples} paired samples; R² ${fit.r_squared.toFixed(4)}. Review it before saving.`;
    } catch (error) {
      calibrationFit = undefined;
      calibrationError = `Calibration fit error: ${String(error)}`;
      statusMessage = calibrationError;
    }
  }

  function openXgzpCalibration() {
    calibrationFit = undefined;
    calibrationError = '';
    const recordingDuration = session.last_summary?.board_elapsed_seconds ?? 0;
    if (recordingDuration > 0) {
      calibrationStartSeconds = 0;
      calibrationEndSeconds = Math.max(0.1, recordingDuration);
    }
    if (calibrationChannelId !== 'xgzp' || activeProfile?.category !== 'course_blood_pressure') {
      calibrationMethod = 'manual_points';
    }
    calibrationDialogOpen = true;
  }

  async function saveCurrentCalibration() {
    if (!activeProfile || !calibrationFit) return;
    const channelId = calibrationChannelId;
    if (!channelId) return;
    try {
      const calibration: CalibrationPreset = {
          schema_version: 1,
          calibration_id: `${activeProfile.profile_id}.${channelId}.${Date.now()}`.replace(/[^a-zA-Z0-9._-]/g, '_').toLowerCase(),
          profile_id: activeProfile.profile_id,
          channel_id: channelId,
          calibration_type: 'linear',
          input_quantity: 'volts',
          output_quantity: manualCalibrationQuantity.trim() || 'engineering value',
          output_units: manualCalibrationUnits || 'mmHg',
          parameters: { slope: calibrationFit.slope, offset: calibrationFit.offset },
          created_at: new Date().toISOString(),
          label: calibrationLabel.trim() || 'Local linear calibration'
      };
      await invoke<CalibrationPreset>('save_calibration', { calibration });
      // The save result is already validated by Rust. Activate it immediately so
      // the XGZP unit selector cannot wait on a separate list refresh/render.
      selectedXgzpCalibrationId = calibration.calibration_id;
      activeXgzpCalibration = calibration;
      savedCalibrations = mergeCalibrationPresets(savedCalibrations, [calibration]);
      channelUnits = {
        ...channelUnits,
        [channelId]: calibration.output_units.trim().toLowerCase() === 'mmhg' ? 'mmhg' : 'calibrated'
      };
      await refreshCalibrations();
      calibrationDialogOpen = false;
      statusMessage = `Saved local calibration “${calibration.label}”. Raw BMEG samples remain unchanged; the calibration will be snapshotted only in future recordings.`;
    } catch (error) { statusMessage = `Could not save calibration: ${String(error)}`; }
  }

  async function deleteSelectedCalibration() {
    if (!activeXgzpCalibration) return;
    try {
      await invoke('delete_calibration', { calibrationId: activeXgzpCalibration.calibration_id });
      selectedXgzpCalibrationId = '';
      activeXgzpCalibration = undefined;
      channelUnits = {
        ...channelUnits,
        [calibrationChannelId]: ['mmhg', 'calibrated'].includes(channelUnits[calibrationChannelId] ?? '')
          ? 'volts'
          : channelUnits[calibrationChannelId]
      };
      await refreshCalibrations();
      statusMessage = 'Deleted the local calibration preset. Existing recording snapshots are unchanged.';
    } catch (error) { statusMessage = `Could not delete calibration: ${String(error)}`; }
  }

  function selectProfile() {
    benchNoticeAcknowledged = false;
    // Force a clean visibility map for the newly selected profile. The reactive
    // profile key also protects programmatic profile changes.
    traceProfileKey = '';
    durationPreset = String(profileTimedPresets.includes(60) ? 60 : profileTimedPresets[0] ?? 10);
    statusMessage = activeProfile
      ? `${activeProfile.display_name} is selected. Lab settings are ready.`
      : 'Choose a course lab to continue.';
    void refreshCalibrations();
  }

  function setChannelVisible(channelId: string, isVisible: boolean) {
    traceVisibility = setTraceVisibility(traceVisibility, channelId, isVisible);
  }

  function updatePlotTimeWindowInput(value: string) {
    plotTimeWindowInput = value;
    // Preserve the last valid extent while a student temporarily clears the
    // numeric field, but let valid edits refresh the shared display snapshot.
    const next = previewPlotTimeWindow(value, plotTimeWindowSeconds);
    if (next !== plotTimeWindowSeconds) {
      plotTimeWindowSeconds = next;
      void pollSession();
    }
  }

  function commitPlotTimeWindow() {
    const next = normalizePlotTimeWindow(plotTimeWindowInput, plotTimeWindowSeconds);
    plotTimeWindowSeconds = next;
    plotTimeWindowInput = String(next);
    void pollSession();
  }

  function changePlotCount(delta: number) {
    plotGroups = setPlotGroupCount(plotChannels, plotGroups, plotGroups.length + delta);
  }

  function assignChannelToGroup(channelId: string, plotNumber: string) {
    plotGroups = assignChannelToPlot(plotChannels, plotGroups, channelId, Number(plotNumber) - 1);
  }

  function useOverlayAll() {
    plotGroups = overlayAll(plotChannels);
  }

  function useOnePlotPerSignal() {
    plotGroups = onePlotPerSignal(plotChannels);
  }

  function reportPlotError(stage: string, detail: string) {
    lastPlotError = { timestamp: new Date().toISOString(), stage, detail };
    // A visualization failure is never a reason to stop the serial worker or
    // finalize its BMEG writer.  The chart retries once locally; raw recording
    // and the bounded display snapshot stay independent.
    statusMessage = isActive
      ? 'A live plot needed to refresh. Recording continues and raw data are still being saved.'
      : 'A live plot needed to refresh. Change the plot arrangement to retry it.';
    void invoke('record_frontend_plot_error', { stage, detail }).catch((error) => {
      console.warn('Could not store live-plot diagnostic', error);
    });
  }

  function bufferedRailCount(channelId: string): number {
    const channelIndex = plotChannels.findIndex((channel) => channel.id === channelId);
    if (channelIndex < 0) return 0;
    const fullScale = Math.pow(2, activeProfile?.acquisition.adc_resolution_bits ?? 12) - 1;
    return samples.reduce((count, sample) => {
      const value = sample.values[channelIndex];
      return count + (value === 0 || value === fullScale ? 1 : 0);
    }, 0);
  }

  function activeDigitalOutputStatus(): string {
    const outputs = activeProfile?.acquisition.digital_outputs ?? [];
    if (!outputs.length) return 'No active digital outputs for this lab';
    const mask = session.digital_output_mask ?? 0;
    const bitForPin: Record<string, number> = { D4: 1, D5: 2, D6: 4 };
    return outputs
      .map((output) => `${output.pin} ${output.label} — ${output.behavior === 'acquisition_sequenced' ? 'Sequenced' : mask & bitForPin[output.pin] ? 'HIGH' : 'LOW'}`)
      .join('; ');
  }

  async function commitProfileMode(mode: OperatingMode) {
    modeChangeInFlight = true;
    try {
      operatingMode = await invoke<OperatingMode>('set_profile_mode', {
        mode,
        acknowledgement: mode === 'instructor_authoring' && instructorAcknowledgement
      });
      statusMessage = operatingMode === 'instructor_authoring'
        ? 'Instructor mode is enabled. You can manage lab settings.'
        : 'Student mode is enabled. Lab settings are protected.';
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
    statusMessage = 'Confirm that Instructor mode can change acquisition settings, then select Instructor mode.';
  }

  async function startRecording() {
    if (startInFlight) {
      startFeedback = 'The Arduino is busy. Wait for the current operation to finish and try again.';
      statusMessage = startFeedback;
      return;
    }
    if (!startReadiness.canStart) {
      startFeedback = startReadiness.message ?? 'Recording cannot start yet.';
      statusMessage = startFeedback;
      return;
    }
    startInFlight = true;
    lastStartFailure = undefined;
    startFeedback = source === 'hardware' ? 'Connecting to Arduino…' : 'Starting simulator recording…';
    statusMessage = startFeedback;
    try {
      session = source === 'simulator'
        ? await invoke<SessionStatus>('start_profile_simulator_recording', { projectFolder, outputFolder: outputDirectory, duration, profileId: selectedProfileId, benchNoticeAcknowledged, calibration: currentRecordingCalibration })
        : await invoke<SessionStatus>('start_profile_hardware_recording', hardwareStartInvokePayload({
            port: selectedPort,
            project_folder: projectFolder,
            output_folder: outputDirectory,
            duration,
            profile_id: selectedProfileId,
            bench_notice_acknowledged: benchNoticeAcknowledged,
            calibration: currentRecordingCalibration
          }));
      startFeedback = source === 'hardware' ? 'Configuring recording…' : 'Starting recording…';
      statusMessage = startFeedback;
    } catch (error) {
      const failure = recordingStartFailure(error);
      lastStartFailure = {
        ...failure,
        timestamp: new Date().toLocaleString(),
        port: selectedPort || 'No board selected',
        lab: activeProfile ? `${activeProfile.display_name} ${activeProfile.profile_version}` : 'No lab selected'
      };
      startFeedback = failure.userMessage;
      statusMessage = startFeedback;
    } finally {
      startInFlight = false;
      await pollSession();
    }
  }

  function sessionStateLabel(state: string): string {
    if (state === 'Acquiring') return 'Recording…';
    if (state === 'Connecting') return 'Connecting…';
    if (state === 'Connected' || state === 'Configured') return 'Configuring recording…';
    if (state === 'Stopping') return 'Stopping…';
    if (state === 'Faulted') return 'Connection needs attention';
    return 'Ready';
  }

  function recordingFaultMessage(): string {
    switch (session.connection_diagnostics?.failure_category) {
      case 'firmware_protocol_error':
        return 'The Arduino reported that it stopped the recording. Verify Firmware, then Refresh Board before starting a new recording.';
      case 'no_data_timeout':
        return 'The Arduino stopped sending samples. Refresh Board and verify the firmware before starting a new recording.';
      case 'device_disconnected':
        return 'The Arduino connection was interrupted. Reconnect the board, refresh it, and start a new recording.';
      default:
        return 'The recording ended because the Arduino serial connection reported an error. Refresh Board and start a new recording. Open Advanced details if the problem continues.';
    }
  }

  async function stopRecording() {
    try {
      session = await invoke<SessionStatus>('stop_recording');
      statusMessage = 'Stopping and saving the recording…';
    } catch (error) {
      statusMessage = 'The recording could not be stopped cleanly. Open Advanced details for troubleshooting information.';
    }
  }

  async function addMarker() {
    try {
      const marker = await invoke<{ timestamp_us: number; label: string }>('add_recording_marker', { label: '' });
      statusMessage = `Marker added at ${marker.timestamp_us} µs.`;
    } catch (error) {
      statusMessage = `Marker error: ${String(error)}`;
    }
  }

  async function disconnect() {
    try {
      session = await invoke<SessionStatus>('disconnect_session');
      statusMessage = 'Session disconnected. A future recording requires an explicit new start.';
    } catch (error) {
      statusMessage = 'The Arduino connection could not be closed cleanly. Unplug and reconnect it before starting a new recording.';
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

  onMount(() => {
    // The shell renders first. One application-level scan then populates the shared
    // cache; route changes consume that cache and never invoke Arduino CLI discovery.
    void (async () => {
      try {
        projectFolder = (await invoke<{ project_folder: string }>('get_project_folder')).project_folder;
      } catch (error) {
        statusMessage = `Project folder needs attention: ${String(error)}`;
      }
      await prepareArduinoTools();
      if (arduinoToolsReady) {
        await refreshEnvironmentSummary();
        await refreshBoards('startup');
      }
    })();
    void pollSession();
    void refreshFirmwareCompatibility();
    void (async () => {
      await refreshProfiles();
      await refreshCalibrations();
    })();
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
    <h1>WVU Bioinstrumentation Studio</h1>
  </header>

  <main class="content wide-content">
    <section class="panel board-panel" aria-labelledby="board-title">
      <div class="panel-heading"><div><h2 id="board-title">Board</h2><p class="help">Select the Arduino UNO R4 WiFi used for this recording.</p></div><span class:locked={firmwareCompatibility === 'wvu_protocol_compatible'} class="mode-badge">{firmwareCompatibility === 'wvu_protocol_compatible' ? 'Firmware ready' : selectedPort ? 'Firmware update required' : 'Arduino not detected'}</span></div>
      <div class="control-grid">
        <label>Board
          <select bind:value={selectedPort} onchange={() => void selectedBoardChanged()} disabled={!boardControlState.canSelectBoard}>
            <option value="">Select an Arduino UNO R4 WiFi</option>
            {#each boards as board}<option value={board.port}>{board.name} — {board.port}</option>{/each}
          </select>
        </label>
        <div class="field-action"><span>Board actions</span><div class="button-pair"><button onclick={() => void refreshBoards()} disabled={!boardControlState.canRefreshBoards}>Refresh Board</button><button onclick={() => void verifySelectedFirmware()} disabled={!boardControlState.canVerifyFirmware}>Verify Firmware</button><button class="gold" onclick={restoreWvuFirmware} disabled={!boardControlState.canRestoreFirmware}>Restore WVU Firmware</button></div></div>
      </div>
      <p class="device-cache-status" role="status">Board: {selectedPort ? `${boards.find((board) => board.port === selectedPort)?.name ?? 'Arduino UNO R4 WiFi'} — ${selectedPort}` : 'Not connected'} · Firmware: {firmwareCompatibility === 'wvu_protocol_compatible' ? 'Ready' : selectedPort ? 'Update required' : '—'} · Arduino tools: {arduinoToolsReady ? 'Ready' : 'Preparing…'}</p>
          <details class="advanced-details"><summary>Advanced details</summary><div class="diagnostic-grid"><p>Protocol: {session.protocol_version}</p><p>Firmware status: {firmwareCompatibility}</p>{#if session.connection_diagnostics?.firmware_build}<p>Firmware build: {session.connection_diagnostics.firmware_build}</p>{/if}{#if session.connection_diagnostics?.firmware_board_id}<p>Board ID: {session.connection_diagnostics.firmware_board_id}</p>{/if}<p>Arduino tools: {firmwareEnvironment.cli_version ?? firmwareEnvironment.problem ?? 'preparing'}</p><p>Last board refresh: {boardScanLastCompleted || 'not yet completed'}</p><p>Received packets: {session.integrity.received_packets}</p><p>CRC failures: {session.integrity.crc_failures}</p>{#if session.connection_diagnostics?.terminal_error_stage}<p>Capture failure stage: {session.connection_diagnostics.terminal_error_stage}</p><p>Capture failure type: {session.connection_diagnostics.terminal_error_classification}</p><p>I/O error kind: {session.connection_diagnostics.terminal_error_kind}</p><p>Windows error: {session.connection_diagnostics.terminal_error_raw_os_error ?? 'none'}</p><p>Capture error detail: {session.connection_diagnostics.terminal_error_detail}</p><p>Capture failure elapsed: {session.connection_diagnostics.terminal_error_elapsed_ms} ms</p><p>Port still listed: {yesNoUnknown(session.connection_diagnostics.selected_port_present_after_error)}</p><p>Same VID/PID listed: {yesNoUnknown(session.connection_diagnostics.same_vid_pid_present_after_error)}</p><p>Same USB serial listed: {yesNoUnknown(session.connection_diagnostics.same_serial_present_after_error)}</p><p>UNO R4 listed: {yesNoUnknown(session.connection_diagnostics.uno_r4_present_after_error)}</p>{#if session.connection_diagnostics.port_enumeration_error}<p>Port enumeration error: {session.connection_diagnostics.port_enumeration_error}</p>{/if}<p>Last valid packet: {session.connection_diagnostics.last_valid_packet_utc ?? 'not recorded'}</p><p>Last valid sample: {session.connection_diagnostics.last_valid_sample_utc ?? 'not recorded'}</p><p>Last successful PING: {session.connection_diagnostics.last_successful_ping_utc ?? 'not recorded'}</p><p>Last PONG/status: {session.connection_diagnostics.last_pong_or_status_utc ?? 'not recorded'}</p>{/if}{#if lastStartFailure}<p>Last recording start: {lastStartFailure.timestamp}</p><p>Stage: {lastStartFailure.stage}</p><p>Code: {lastStartFailure.code}</p><p>Board: {lastStartFailure.port}</p><p>Lab: {lastStartFailure.lab}</p><p>Detail: {lastStartFailure.technicalDetail}</p>{/if}{#if lastPlotError}<p>Last live-plot error: {lastPlotError.timestamp}</p><p>Plot stage: {lastPlotError.stage}</p><p>Plot detail: {lastPlotError.detail}</p>{/if}{#if session.last_error}<p>Recent connection error: {session.last_error}</p>{/if}</div></details>
    </section>

    <section class="panel project-panel" aria-labelledby="project-folder-title">
      <h2 id="project-folder-title">Project folder</h2>
      <div class="control-grid"><label>Project folder<input value={projectFolder} readonly title={projectFolder} /></label><div class="field-action"><span>Location</span><button onclick={chooseProjectFolder} disabled={session.state !== 'Disconnected'}>Browse</button></div></div>
      <p class="help">Recordings are saved under this folder. Choose the Output folder in Session setup to create a trial subfolder.</p>
    </section>

    <section class="acquisition-section" aria-labelledby="acquisition-title">
        <h2 id="acquisition-title">Acquisition</h2>
        <p class="notice">Teaching use only — not a medical device. Follow BMEG 420L lab instructions and instructor safety procedures. Raw counts and Arduino-input volts are preserved; this app makes no diagnostic or clinical decision.</p>
        {#if source === 'hardware' && firmwareCompatibility !== 'wvu_protocol_compatible'}
          <p class="warning" role="status">Hardware recording is unavailable until Firmware is ready. Use Verify Firmware or Restore WVU Firmware above.</p>
        {/if}

        <section class="panel profile-panel" aria-labelledby="profile-title">
          <div class="panel-heading"><div><h3 id="profile-title">Lab</h3><p class="help">Choose the assigned course lab. Its required channels and recording settings are applied automatically.</p></div><span class:locked={!instructorModeActive} class="mode-badge">{instructorModeActive ? 'Instructor mode' : 'Student mode'}</span></div>
          <OperatingModeControl bind:operatingMode bind:instructorAcknowledgement disabled={session.state !== 'Disconnected' || modeChangeInFlight} {onModeConfirmed} {onInstructorBlocked} />
          {#if instructorModeActive}
            <LabManager selectedProfile={activeProfile} onSaved={labRevisionSaved} onStatus={(message) => statusMessage = message} />
          {/if}
          <label>Selected lab
            <select bind:value={selectedProfileId} onchange={selectProfile} disabled={session.state !== 'Disconnected'}>
              {#each acquisitionProfiles as profile}<option value={profile.profile_id}>{profile.display_name} — {profile.profile_version}</option>{/each}
            </select>
          </label>
          {#if activeProfile}
            <div class="profile-details">
              <span><strong>Channels</strong>{activeChannels.map((channel) => channel.label).join('; ')}</span>
              <span><strong>Rate</strong>{activeProfile.acquisition.sample_rate_hz} {activeProfile.acquisition.acquisition_mode === 'pulseox_4state' ? 'cycles/s' : 'frames/s'}</span>
              <span><strong>ADC</strong>{activeProfile.acquisition.adc_resolution_bits} bit</span>
              <span><strong>Units</strong>ADC counts and Arduino input volts</span>
            </div>
            <details class="advanced-details" open={instructorModeActive}>
              <summary>Lab details</summary>
              <div class="profile-details">
                <span><strong>Pin mapping</strong>{activeChannels.map((channel) => `${channel.pin} = ${channel.label}`).join('; ')}</span>
                {#if activeProfile.acquisition.acquisition_mode === 'pulseox_4state'}<span><strong>Pulse-ox sequence</strong>RED → DARK → IR → DARK; {activeProfile.acquisition.state_dwell_us} µs per state</span>{/if}
                {#if instructorModeActive}<span><strong>Instructor reference</strong>{activeProfile.profile_id} / {activeProfile.profile_version}</span>{/if}
              </div>
            </details>
            {#if instructorModeActive && activeProfile.safety.notices.length}
              <details class="advanced-details"><summary>Safety and lab notes</summary>{#each activeProfile.safety.notices as notice}<p class="help profile-notice">{notice}</p>{/each}</details>
            {/if}
            {#if activeProfile.safety.bench_only && ['ecg', 'emg'].includes(activeProfile.category)}
              <label class="acknowledgement"><input type="checkbox" bind:checked={benchNoticeAcknowledged} disabled={session.state !== 'Disconnected'} /> I acknowledge this {activeProfile.category.toUpperCase()} profile is bench-validation only. No human-connected recording is authorized.</label>
            {/if}
          {:else}
            <p class="error">No course lab is available. Restart the application or contact your instructor.</p>
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
            <label>Output folder <input bind:value={outputDirectory} placeholder="Participant01\Trial03" disabled={session.state !== 'Disconnected'} aria-describedby="output-help" /></label>
            <label>Analog pins <input value={activeProfile ? activeProfile.acquisition.acquisition_mode === 'pulseox_4state' ? `TX ${activeProfile.acquisition.analog_inputs?.tx ?? 'A0'}; RX ${activeProfile.acquisition.analog_inputs?.rx ?? 'A1'}` : activeChannels.map((channel) => channel.pin).join(', ') : '—'} readonly title="Protected by the selected profile" /></label>
            <label>ADC resolution <input value={activeProfile ? `${activeProfile.acquisition.adc_resolution_bits} bit` : '—'} readonly title="Protected by the selected profile" /></label>
            <label>Frame / cycle rate <input value={activeProfile ? `${activeProfile.acquisition.sample_rate_hz} ${pulseoxProfile ? 'cycles/s' : 'frames/s'}` : '—'} readonly title="Protected by the selected profile" /></label>
            <label>Test note <input bind:value={note} readonly /></label>
          </div>
          <p id="output-help" class="help">Effective destination: <strong>{effectiveOutputFolder || 'Choose a Project folder'}</strong>. Output folder is relative to Project folder; nested trial folders are allowed. Files use timestamped names and existing files are never overwritten.</p>
          {#if outputFolderError}<p class="error" role="alert">{outputFolderError}</p>{/if}
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

        <section class="panel calibration-panel" aria-labelledby="calibration-title">
          <div class="plot-heading"><div><h3 id="calibration-title">Calibration &amp; Units</h3><p class="help">Raw ADC counts remain authoritative in BMEG. These settings control only live display and derived CSV columns, and are frozen into each new recording’s metadata.</p></div></div>
          <div class="control-grid">
            <label>ADC reference voltage (V)<input type="number" min="0.1" max="10" step="0.001" bind:value={adcReferenceV} disabled={session.state !== 'Disconnected'} /></label>
            {#if hasMpxvChannel}
              <label>MPXV sensor supply (Vs, V)<input type="number" min="0.1" max="10" step="0.001" bind:value={mpxvSensorSupplyV} disabled={session.state !== 'Disconnected'} /></label>
            {/if}
            {#if linearCalibrationChannels.length}
              <label>Linear calibration channel
                <select value={calibrationChannelId} onchange={(event) => void selectCalibrationChannel(event.currentTarget.value)} disabled={session.state !== 'Disconnected'}>
                  {#each linearCalibrationChannels as channel}<option value={channel.id}>{channel.label}</option>{/each}
                </select>
              </label>
              <label>{calibrationChannelId === 'xgzp' ? 'XGZP calibration' : 'Channel calibration'}
                <select value={selectedXgzpCalibrationId} onchange={(event) => void selectXgzpCalibration(event.currentTarget.value)} disabled={session.state !== 'Disconnected'}>
                  <option value="">None — counts/volts only</option>
                  {#each savedCalibrations as calibration}<option value={calibration.calibration_id}>{calibration.label} ({calibration.output_units})</option>{/each}
                </select>
              </label>
              <p class="help">Active {calibrationChannelId === 'xgzp' ? 'XGZP' : 'channel'} calibration: {activeXgzpCalibration?.label ?? 'none — counts/volts only'}.</p>
              <div class="button-pair calibration-actions"><button onclick={openXgzpCalibration} disabled={session.state !== 'Disconnected'}>Calibrate {calibrationChannelId === 'xgzp' ? 'XGZP' : 'channel'}</button><button onclick={deleteSelectedCalibration} disabled={session.state !== 'Disconnected' || !activeXgzpCalibration}>Delete calibration</button></div>
            {/if}
          </div>
          <div class="unit-grid" aria-label="Per-channel display units">
            {#each plotChannels as channel}
              <label>{channel.label}
                <select value={channelUnits[channel.id] ?? 'counts'} onchange={(event) => setChannelUnit(channel.id, event.currentTarget.value)}>
                  {#each channelUnitOptions[channel.id] ?? ['counts', 'volts'] as unit}<option value={unit}>{displayUnitLabel(unit, currentRecordingCalibration, channel.id)}</option>{/each}
                </select>
              </label>
            {/each}
          </div>
          {#if hasMpxvChannel}<p class="help">MPXV uses P<sub>kPa</sub> = (Vout / Vs − 0.04) / 0.009 and P<sub>mmHg</sub> = 7.5006 × P<sub>kPa</sub>. A channel configured for generic linear calibration offers mmHg only after you save its local fit.</p>{/if}
          {#if activeProfile?.category === 'course_emg_force'}<p class="help">The A3 conversion is labeled <strong>Pressure (kPa)</strong>; it does not infer muscular force.</p>{/if}
          {#if pulseoxProfile}<p class="help">Pulse-ox units apply directly to the eight raw LED-state values. No background subtraction or optical processing is applied.</p>{/if}
        </section>

        <div class="action-row recording-actions">
          <button class="gold" onclick={startRecording} disabled={isActive || startInFlight || boardOperationBusy}>Connect, configure, and start recording</button>
          <button class="stop" onclick={stopRecording} disabled={!isActive || session.state === 'Stopping'}>Stop recording</button>
          <button onclick={disconnect} disabled={session.state === 'Disconnected'}>Disconnect</button>
          {#if canRetryHandshake}<button onclick={retryHandshake}>Retry handshake</button>{/if}
          {#if canReset}<button onclick={resetBoardAndRetry}>Reset board and retry</button>{/if}
        </div>
        {#if !canStart}
          <p class="warning" role="status">{startReadiness.message}</p>
        {:else if startFeedback}
          <p class="status" role="status">{startFeedback}</p>
        {/if}

        <section class="metric-grid" aria-label="Acquisition metrics">
          <span><strong>Status</strong>{sessionStateLabel(session.state)}</span><span><strong>Arduino</strong>{session.board || 'Not connected'} {session.port ? `(${session.port})` : ''}</span>
          <span><strong>Duration</strong>{session.duration?.mode === 'until_stopped' ? 'Until stopped' : session.duration?.seconds ? `${session.duration.seconds} s timed` : durationMode === 'until_stopped' ? 'Until stopped' : `${timedSeconds} s timed`}</span>
          <span><strong>Elapsed host</strong>{formatDuration(session.elapsed_seconds)}</span>
          {#if session.remaining_seconds !== undefined}<span><strong>Remaining</strong>{formatDuration(session.remaining_seconds)}</span>{/if}
          <span><strong>Storage</strong>{formatStorage(session.available_disk_bytes)}</span><span><strong>Frames</strong>{session.samples}</span><span><strong>Measured rate</strong>{session.measured_rate_hz.toFixed(3)} Hz</span>
          {#if activeProfile?.acquisition.digital_outputs?.length}<span><strong>Lab outputs</strong>{activeDigitalOutputStatus()}</span>{/if}
        </section>
        {#if session.storage_warning}<p class="warning" role="status">{session.storage_warning}</p>{/if}
        {#if session.last_error}<p class="error" role="alert">{recordingFaultMessage()}</p>{/if}
        <section class="panel plot-panel">
          <div class="plot-heading"><h3>{pulseoxProfile ? 'Bounded live raw pulse-ox plot' : 'Bounded live synchronized raw plot'}</h3><span class="help">Each plot autoscales to its selected display unit.</span></div>
          <section class="plot-arrangement" aria-labelledby="plot-arrangement-title">
            <div class="plot-arrangement-heading"><div><h4 id="plot-arrangement-title">Plot arrangement</h4><p class="help">Display-only grouping. Every captured signal remains in the raw BMEG, CSV, and metadata even when hidden.</p></div><div class="plot-count-control" aria-label="Number of plots"><span>Number of plots</span><button aria-label="Use one fewer plot" onclick={() => changePlotCount(-1)} disabled={plotGroups.length <= 1 || !plotChannels.length}>−</button><strong>{plotGroups.length}</strong><button aria-label="Use one more plot" onclick={() => changePlotCount(1)} disabled={plotGroups.length >= plotChannels.length || !plotChannels.length}>+</button></div></div>
            <div class="button-pair"><button onclick={useOverlayAll} disabled={!plotChannels.length}>Overlay all</button><button onclick={useOnePlotPerSignal} disabled={!plotChannels.length}>One plot per signal</button></div>
            <div class="plot-assignment-grid" aria-label="Signal plot assignments">
              {#each plotChannels as channel}
                <label>{channel.label}<select value={String(Math.max(0, plotGroups.findIndex((group) => group.channelIds.includes(channel.id))) + 1)} onchange={(event) => assignChannelToGroup(channel.id, event.currentTarget.value)} disabled={!plotGroups.length}>{#each plotGroups as _group, index}<option value={String(index + 1)}>Plot {index + 1}</option>{/each}</select></label>
              {/each}
            </div>
          </section>
          <div class="trace-controls" aria-label="Visible traces">{#each plotChannels as channel}<label class="choice"><input type="checkbox" checked={traceVisibility[channel.id] !== false} onchange={(event) => setChannelVisible(channel.id, event.currentTarget.checked)} /> {channel.label}</label>{/each}</div>
          {#if pulseoxProfile}<p class="help">TX and RX plots show each raw RED, DARK 1, IR, and DARK 2 measurement. The eight raw values are recorded unchanged.</p>{/if}
          {#if !visibleTraceIds.length}
            <p class="help">No traces are selected for display. Recording and export continue for every profile field.</p>
          {:else}
            <div class="stacked-plots" aria-label="Synchronized plot groups">
              {#each renderedPlotGroups as group, index (group.id)}
                <section class="stacked-plot" aria-label={`Plot ${index + 1}: ${group.channelIds.map((id) => plotChannels.find((channel) => channel.id === id)?.label ?? id).join(', ')}`}>
                  <div class="stacked-plot-heading"><strong>Plot {index + 1}: {group.channelIds.map((id) => plotChannels.find((channel) => channel.id === id)?.label ?? id).join(' + ')}</strong><span>{unitsForGroup(group.channelIds, channelUnits, currentRecordingCalibration)}{group.channelIds.some((id) => bufferedRailCount(id)) ? `; ${group.channelIds.reduce((count, id) => count + bufferedRailCount(id), 0)} buffered rail samples` : ''}</span></div>
                  <LivePlot {samples} channels={plotChannels} visibleChannelIds={group.channelIds} {channelUnits} calibration={currentRecordingCalibration} adcBits={activeProfile?.acquisition.adc_resolution_bits ?? 12} {displayRevision} timeOriginUs={session.display_origin_timestamp_us} hardwareSource={source === 'hardware'} onPlotError={reportPlotError} />
                </section>
              {/each}
            </div>
          {/if}
          <div class="plot-time-window" aria-label="Shared plot time window">
            <label for="plot-time-window">Plot time window</label>
            <input
              id="plot-time-window"
              type="number"
              min="0.5"
              max="30"
              step="0.5"
              inputmode="decimal"
              aria-describedby="plot-time-window-help"
              value={plotTimeWindowInput}
              oninput={(event) => updatePlotTimeWindowInput(event.currentTarget.value)}
              onblur={commitPlotTimeWindow}
              onkeydown={(event) => { if (event.key === 'Enter') { event.preventDefault(); commitPlotTimeWindow(); event.currentTarget.blur(); } }}
            />
            <span aria-hidden="true">s</span>
            <span id="plot-time-window-help" class="help">0.5–30 s. Applies to all live plots. Recording is unaffected.</span>
          </div>
          <div class="action-row marker-row"><button onclick={addMarker} disabled={session.state !== 'Acquiring'}>Add marker</button></div>
        </section>
        {#if session.last_summary}
          <section class="panel paths">
            <h3>Finalized files</h3>
            <p>{session.last_summary.completion_status === 'complete' ? 'Recording complete.' : 'Recording ended before completion.'}</p>
            <p title={session.last_summary.bmeg_path}>{session.last_summary.bmeg_path}</p>
            <p title={session.last_summary.metadata_path}>{session.last_summary.metadata_path}</p>
            <p title={session.last_summary.csv_path}>{session.last_summary.csv_path}</p>
            <details class="advanced-details"><summary>Recording details</summary><p>Stop reason: {session.last_summary.stop_reason}.</p>{#if instructorModeActive && session.last_summary.profile}<p>Lab reference: {session.last_summary.profile.profile.profile_id} {session.last_summary.profile.profile.profile_version}</p>{/if}</details>
          </section>
        {/if}
      <p class="status" aria-live="polite">{statusMessage}</p>
    </section>
    </main>
</div>

{#if calibrationDialogOpen}
  <div class="operation-backdrop">
    <dialog open class="calibration-dialog" aria-modal="true" aria-labelledby="calibration-dialog-title">
      <div class="plot-heading"><h2 id="calibration-dialog-title">Calibrate {activeChannels.find((channel) => channel.id === calibrationChannelId)?.label ?? 'channel'}</h2><button onclick={() => { calibrationDialogOpen = false; calibrationFit = undefined; calibrationError = ''; }}>Close</button></div>
      <p class="warning">Course calibration only. This fits engineering pressure units from your selected data; it does not make a blood-pressure determination or clinical conclusion.</p>
      <fieldset disabled={session.state !== 'Disconnected'}>
        <legend>Calibration method</legend>
        <div class="choice-row">{#if calibrationChannelId === 'xgzp' && activeProfile?.category === 'course_blood_pressure'}<label class="choice"><input type="radio" bind:group={calibrationMethod} value="xgzp_recording" /> Use completed synchronized BP recording</label>{/if}<label class="choice"><input type="radio" bind:group={calibrationMethod} value="manual_points" /> Enter manual linear points</label></div>
      </fieldset>
      {#if calibrationMethod === 'xgzp_recording'}
        <p class="help">The completed recording provides A1 MPXV as the reference and A2 XGZP as the fitted input. Select a stable interval in seconds.</p>
        <div class="control-grid"><label>Start (seconds)<input type="number" min="0" step="0.1" bind:value={calibrationStartSeconds} /></label><label>End (seconds)<input type="number" min="0.1" step="0.1" bind:value={calibrationEndSeconds} /></label></div>
      {:else}
        <label>Manual points: channel volts, reference engineering value<textarea rows="5" bind:value={manualCalibrationPoints} spellcheck="false"></textarea></label>
        <label>Output quantity<input bind:value={manualCalibrationQuantity} placeholder="pressure" /></label>
        <label>Output units<input bind:value={manualCalibrationUnits} placeholder="mmHg" /></label>
      {/if}
      <div class="action-row"><button class="gold" onclick={calculateCalibrationFit}>Calculate linear fit</button></div>
      {#if calibrationError}<p class="error" role="alert">{calibrationError}</p>{/if}
      {#if calibrationFit}
        <section class="metric-grid" aria-label="Calibration fit"><span><strong>Slope</strong>{calibrationFit.slope.toFixed(6)}</span><span><strong>Offset</strong>{calibrationFit.offset.toFixed(6)}</span><span><strong>R²</strong>{calibrationFit.r_squared.toFixed(6)} (informational)</span><span><strong>Paired samples</strong>{calibrationFit.paired_samples}</span></section>
        <label>Calibration label<input bind:value={calibrationLabel} maxlength="80" /></label>
        <div class="action-row"><button class="gold" onclick={saveCurrentCalibration}>Save calibration</button></div>
      {/if}
    </dialog>
  </div>
{/if}

{#if activeOperation || firmwareWorkflow.job?.active}
  <div class="operation-backdrop" role="presentation">
    <div class="operation-modal" role="dialog" aria-modal="true" aria-labelledby="operation-title" aria-describedby="operation-stage">
      <div class="spinner" aria-hidden="true"></div>
      <div>
        <h2 id="operation-title">{activeOperation?.title ?? `${firmwareWorkflow.job?.kind ?? 'Firmware'} in progress…`}</h2>
        <p id="operation-stage">{activeOperation?.stage ?? firmwareWorkflow.job?.message ?? firmwareWorkflow.job?.stage}</p>
        <p class="help">This operation has no reliable numeric percentage. The application remains responsive; conflicting actions are temporarily blocked.</p>
        {#if !activeOperation && firmwareWorkflow.job?.active}<button class="stop" onclick={cancelFirmwareJob}>Cancel {firmwareWorkflow.job.kind}</button>{/if}
      </div>
    </div>
  </div>
{/if}

<style>
  :global(*) { box-sizing: border-box; }
  :global(body) { margin: 0; min-width: 0; overflow-x: hidden; font-family: "Segoe UI", Arial, sans-serif; background: #F7F7F7; color: #17222e; }
  :global(button), :global(input), :global(select) { font: inherit; }
  .app-shell { min-height: 100vh; min-width: 0; }
  .app-header { min-width: 0; display: flex; flex-wrap: wrap; align-items: center; gap: clamp(.65rem, 2vw, 1.2rem); padding: clamp(.75rem, 2vw, 1.2rem) clamp(1rem, 3vw, 2rem); background: #002855; color: #F7F7F7; }
  .app-header img { width: clamp(132px, 19vw, 174px); max-width: 100%; max-height: 84px; object-fit: contain; flex: 0 1 auto; }
  .unit-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(13rem, 1fr)); gap: .65rem; margin-top: .75rem; }
  .calibration-actions { align-self: end; }
  .calibration-dialog { width: min(46rem, calc(100vw - 2rem)); max-height: min(88vh, 52rem); overflow: auto; background: #fff; border: 1px solid #8aa0b6; border-radius: .35rem; padding: clamp(1rem, 3vw, 1.5rem); box-shadow: 0 .4rem 1.5rem #001b3640; }
  textarea { width: 100%; min-height: 7rem; resize: vertical; font: 0.92rem ui-monospace, Consolas, monospace; }
  h1 { margin: 0; font-size: clamp(1.15rem, 2.2vw, 1.45rem); overflow-wrap: anywhere; }
  button { min-height: 2.5rem; border: 1px solid #002855; border-radius: .3rem; background: #fff; color: #002855; padding: .55rem .75rem; font-weight: 650; text-align: left; cursor: pointer; overflow-wrap: anywhere; }
  button:focus-visible, input:focus-visible, select:focus-visible { outline: 3px solid #EEAA00; outline-offset: 2px; }
  button:disabled { cursor: not-allowed; opacity: .55; } button:not(:disabled):hover { background: #002855; color: #fff; } button.gold { background: #EEAA00; color: #17222e; } button.stop { background: #9d2424; border-color: #761a1a; color: #fff; }
  .content { min-width: 0; width: 100%; max-width: 112rem; margin: 0 auto; padding: clamp(1rem, 3vw, 1.75rem); overflow-wrap: anywhere; }
  .device-cache-status { margin: 0 0 .8rem; padding: .55rem .7rem; border: 1px solid #d7dde2; border-radius: .3rem; background: #fff; color: #42515d; overflow-wrap: anywhere; }
  h2 { margin-top: 0; } h3 { margin: 0 0 .8rem; } .notice, .warning { border-left: 5px solid #EEAA00; background: #fff8e8; padding: .75rem; } .warning { border-color: #9b6700; }
  .panel { min-width: 0; margin: 1rem 0; padding: clamp(.8rem, 2vw, 1rem); background: #fff; border: 1px solid #d7dde2; border-radius: .35rem; }
  .advanced-details { min-width: 0; margin-top: .8rem; } .advanced-details > summary { cursor: pointer; color: #002855; font-weight: 700; } .advanced-details[open] > summary { margin-bottom: .75rem; }
  .panel-heading { display: flex; min-width: 0; flex-wrap: wrap; justify-content: space-between; gap: .75rem; align-items: start; } .panel-heading .help { max-width: 75ch; }
  .mode-badge { border: 1px solid #855a00; border-radius: 999px; padding: .35rem .6rem; background: #fff4dd; color: #684600; font-weight: 700; white-space: nowrap; } .mode-badge.locked { border-color: #176c33; background: #eaf6ee; color: #174f27; }
  .profile-panel > label { margin-top: .8rem; } .profile-details { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 16rem), 1fr)); gap: .55rem; margin-top: .8rem; } .profile-details span { min-width: 0; padding: .6rem .7rem; border: 1px solid #d7dde2; border-radius: .3rem; background: #f9fbfc; overflow-wrap: anywhere; } .profile-details strong { display: block; color: #42515d; font-size: .78rem; text-transform: uppercase; letter-spacing: .03em; } .profile-notice { margin: .6rem 0 0; }
  .acknowledgement { display: flex; gap: .55rem; align-items: flex-start; margin-top: .75rem; padding: .7rem; background: #fff4dd; border: 1px solid #9b6700; font-weight: 700; } .acknowledgement input { width: auto; min-height: auto; margin-top: .2rem; }
  .button-pair { display: flex; flex-wrap: wrap; gap: .6rem; margin-top: .8rem; } .button-pair button { flex: 0 1 18rem; }
  .control-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 15rem), 1fr)); gap: .8rem; align-items: end; }
  label, .field-action { min-width: 0; display: grid; gap: .3rem; font-weight: 600; } .field-action > span { font-size: .9rem; }
  input, select { min-width: 0; width: 100%; min-height: 2.4rem; border: 1px solid #8493a0; border-radius: .25rem; background: #fff; padding: .35rem .5rem; }
  input[readonly] { background: #eef1f3; color: #374450; } .help { margin: .7rem 0 0; color: #4b5965; } .error { color: #8b1515; font-weight: 650; }
  fieldset { min-width: 0; border: 0; padding: 0; margin: 0; } legend { font-weight: 650; margin-bottom: .3rem; } .choice-row, .duration-controls, .action-row, .plot-heading { display: flex; min-width: 0; flex-wrap: wrap; gap: .7rem; align-items: end; } .choice { display: flex; align-items: center; gap: .4rem; } .choice input { width: auto; min-height: auto; }
  .duration-controls { margin-top: .75rem; } .action-row { margin: 1rem 0; } .recording-actions button { flex: 0 1 20rem; } .metric-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 12rem), 1fr)); gap: .55rem; margin: 1rem 0; } .metric-grid span { min-width: 0; padding: .6rem .7rem; border: 1px solid #d7dde2; border-radius: .3rem; background: #fff; overflow-wrap: anywhere; } .metric-grid strong { display: block; color: #42515d; font-size: .78rem; text-transform: uppercase; letter-spacing: .03em; }
  .plot-panel { min-width: 0; } .plot-heading { justify-content: space-between; align-items: center; }
  .plot-arrangement { min-width: 0; margin-top: .75rem; padding: .75rem; border: 1px solid #d7dde2; border-radius: .3rem; background: #f9fbfc; } .plot-arrangement-heading { display: flex; min-width: 0; flex-wrap: wrap; justify-content: space-between; gap: .75rem; align-items: start; } .plot-arrangement h4 { margin: 0; } .plot-arrangement .help { max-width: 72ch; } .plot-count-control { display: flex; flex-wrap: wrap; align-items: center; gap: .45rem; font-weight: 700; } .plot-count-control button { min-height: 2.1rem; min-width: 2.1rem; padding: .2rem .55rem; text-align: center; } .plot-count-control strong { min-width: 1.5rem; text-align: center; } .plot-assignment-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 15rem), 1fr)); gap: .65rem; margin-top: .8rem; } .trace-controls { display: flex; flex-wrap: wrap; gap: .45rem .8rem; margin: .75rem 0; } .trace-controls .choice { font-size: .9rem; } .stacked-plots { display: grid; gap: .85rem; min-width: 0; } .stacked-plot { min-width: 0; min-height: 0; padding: .7rem; border: 1px solid #d7dde2; border-radius: .3rem; background: #f9fbfc; } .stacked-plot-heading { display: flex; flex-wrap: wrap; justify-content: space-between; gap: .5rem; margin-bottom: .5rem; color: #42515d; } .plot-time-window { display: flex; flex-wrap: wrap; align-items: center; gap: .45rem; margin-top: .85rem; } .plot-time-window label { width: auto; font-weight: 700; } .plot-time-window input { width: 6rem; } .plot-time-window .help { flex: 1 1 20rem; margin: 0; } .marker-row { align-items: end; }
  .paths p, .status { overflow-wrap: anywhere; word-break: break-word; } .paths p { margin: .35rem 0; } .status { margin: 1rem 0 0; padding: .7rem; border: 1px solid #d7dde2; background: #fff; border-radius: .3rem; } .diagnostic-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 16rem), 1fr)); gap: .75rem; }
  .operation-backdrop { position: fixed; inset: 0; z-index: 1000; display: grid; place-items: center; padding: 1rem; background: rgb(12 27 42 / 52%); } .operation-modal { width: min(100%, 35rem); display: flex; gap: 1rem; align-items: flex-start; padding: 1.2rem; border: 2px solid #002855; border-radius: .45rem; background: #fff; box-shadow: 0 1rem 3rem rgb(0 0 0 / 25%); } .operation-modal h2 { margin: 0; } .operation-modal p { margin: .4rem 0; overflow-wrap: anywhere; } .spinner { flex: 0 0 2rem; width: 2rem; height: 2rem; border: .3rem solid #d7dde2; border-top-color: #002855; border-radius: 50%; animation: spin .8s linear infinite; } @keyframes spin { to { transform: rotate(360deg); } }
  @media (max-width: 650px) { .app-header { align-items: flex-start; } .app-header img { max-width: 150px; } .content { padding: 1rem; } .recording-actions button { flex-basis: 100%; } .plot-heading { align-items: flex-start; } }
</style>
