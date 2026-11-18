<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { confirm, open } from '@tauri-apps/plugin-dialog';
  import '$lib/styles/studio-form.css';
  import MessageCenter from '$lib/components/MessageCenter.svelte';
  import Modal from '$lib/components/Modal.svelte';
  import RecordingBar from '$lib/components/RecordingBar.svelte';
  import SetupCard from '$lib/components/SetupCard.svelte';
  import BoardPanel from '$lib/components/BoardPanel.svelte';
  import LabCard from '$lib/components/LabCard.svelte';
  import SessionCard from '$lib/components/SessionCard.svelte';
  import DurationCard from '$lib/components/DurationCard.svelte';
  import CalibrationCard from '$lib/components/CalibrationCard.svelte';
  import CalibrationDialog from '$lib/components/CalibrationDialog.svelte';
  import LiveSignalsPanel from '$lib/components/LiveSignalsPanel.svelte';
  import FirmwareJobProgress from '$lib/components/FirmwareJobProgress.svelte';
  import RecordingResultCard from '$lib/components/RecordingResultCard.svelte';
  import { boardStatus, lastFirmwareOperation } from '$lib/board-panel';
  import { recordingResultView } from '$lib/recording-result';
  import { shouldAutoSelectHardware } from '$lib/source-select';
  import { revealIfHidden, scrollToAndFocus } from '$lib/page-navigation';
  import { faultConcernsSource, problemBanner, simulatorRecordingFault, type NextAction } from '$lib/messages';
  import { CHECKLIST_TARGETS, readinessChecklist, type ChecklistTarget } from '$lib/readiness';
  import { initialCardState, openCard, recordingDisclosure, withModeDefaults, type CardOpenState } from '$lib/setup-cards';
  import {
    boardSummary,
    calibrationSummary,
    durationSummary,
    labSummary,
    plotArrangementSummary,
    projectSummary,
    sessionSummary
  } from '$lib/setup-summaries';
  import { formatCount, formatRate, plannedDurationText, recordingActive, recordingChip, recordingProgress } from '$lib/recording-status';
  import { markerElapsedSeconds, markerFeedback, placeMarkers, type RecordingMarker } from '$lib/markers';
  import type { LabProfile } from '$lib/labs';
  import { connectionActions } from '$lib/connection-actions';
  import { commandErrorDetail, commandErrorKeptPreviousTools, commandErrorRepairable, formatCommandError } from '$lib/errors';
  import { resolvedDigitalOutputs } from '$lib/lab-authoring';
  import { firmwareStepAnnouncement, firmwareStepIndex } from '$lib/firmware-job';
  import { durationRequest, type RecordingDurationRequest } from '$lib/duration';
  import {
    allowedDurationMode,
    catalogNotice,
    labDurationModes,
    labDurationRefusal,
    labMinimumCustomSeconds,
    labTimedDurationAllowed,
    type CatalogStatus
  } from '$lib/lab-catalog';
  import {
    initialChannelUnits,
    isFixedMpxvChannel,
    reconcileChannelUnits,
    recordingCalibrationFor,
    supportedDisplayUnits,
    type CalibrationPreset,
    type DisplayUnit,
    type RecordingCalibration
  } from '$lib/calibration';
  import { calibrationVoltageError } from '$lib/calibration-inputs';
  import {
    calibrationChannelFor,
    defaultDurationPreset,
    DEFAULT_TIMED_PRESETS_SECONDS,
    linearCalibrationChannels
  } from '$lib/calibration-selection';
  import type { OperatingMode } from '$lib/operating-mode';
  import {
    defaultPlotGroups,
    initialTraceVisibility,
    isPulseOxProfile,
    labRevisionKey,
    normalizePlotGroups,
    numberedVisiblePlotGroups,
    plotChannelsFor,
    profileChannels,
    visibleChannelIds,
    type PlotGroup,
    type VisibleChannelMap
  } from '$lib/multichannel';
  import { reconcileBoardCache } from '$lib/board-cache';
  import { boardControls, firmwareReadyOnPort, type FirmwareCompatibility } from '$lib/board-controls';
  import {
    hardwareStartInvokePayload,
    recordingStartFailure,
    recordingStartReadiness,
    simulatorStartInvokePayload,
    startFeedbackView,
    type HardwareStartRequest,
    type RecordingStartReadiness,
    type SimulatorStartRequest
  } from '$lib/recording-start';
  import { effectiveRecordingFolder, relativeOutputFolderError } from '$lib/project-folder';
  import {
    DEFAULT_PLOT_TIME_WINDOW_SECONDS,
    MAX_RENDERED_DISPLAY_POINTS,
    shouldFetchDisplayData,
    statusPollDue
  } from '$lib/live-display';
  import {
    displayedFreeBytes,
    formatStorage,
    storageEstimateInvokePayload,
    storageShortfallWarning,
    type RecordingStorageEstimate,
    type StorageEstimateRequest
  } from '$lib/storage';
  import logoUrl from '../../assets/branding/WVU-CBE Logo.svg';

  import {
    emptyIntegrity,
    type ActiveOperation,
    type ArduinoRuntimeStatus,
    type Board,
    type FirmwareEnvironment,
    type FirmwareJob,
    type FirmwareVerification,
    type FirmwareWorkflowStatus,
    type HandshakeRetryResult,
    type PlotErrorRecord,
    type Point,
    type SessionStatus,
    type StartFailureRecord
  } from '$lib/session-types';
  import AdvancedDetails from '$lib/components/AdvancedDetails.svelte';

  type AcquisitionProfile = LabProfile;

  let samples: Point[] = [];
  let displayRevision = 0;
  // One display-only time extent is shared by every rendered plot group. It
  // changes only the bounded display query, never the active acquisition.
  let plotTimeWindowSeconds = DEFAULT_PLOT_TIME_WINDOW_SECONDS;
  let boards: Board[] = [];
  let selectedPort = '';
  let boardScanStatus: 'idle' | 'scanning' | 'complete' | 'error' = 'idle';
  let boardScanLastCompleted = '';
  let boardScanError = '';
  let boardScanInFlight = false;
  let activeOperation: ActiveOperation | undefined;
  // Every operation still running, oldest first (see `runOperation`).
  let runningOperations: Array<{ token: number; operation: ActiveOperation }> = [];
  let operationToken = 0;
  let firmwareEnvironment: FirmwareEnvironment = { expected_fqbn: 'arduino:renesas_uno:unor4wifi', boards: [], ready: false };
  let source: 'simulator' | 'hardware' = 'simulator';
  let projectFolder = '';
  // Shown when the saved Project folder is unavailable or its setting was unreadable.
  let projectFolderNotice = '';
  // Lab catalog health (skipped or unloadable labs), shown in both modes.
  let catalogStatus: CatalogStatus | undefined;
  let outputDirectory = '';
  let durationMode: 'timed' | 'until_stopped' = 'timed';
  let durationPreset = '60';
  let customSeconds = 60;
  let duration: RecordingDurationRequest = { mode: 'timed', seconds: 60 };
  let note = '';
  // The fields as typed (Svelte binds an emptied number field as null) and
  // the last valid values, which the display and the recording use.
  let adcReferenceV: number | null = 5;
  let mpxvSensorSupplyV: number | null = 5;
  let appliedAdcReferenceV = 5;
  let appliedMpxvSupplyV = 5;
  let channelUnits: Record<string, DisplayUnit> = {};
  let channelUnitOptions: Record<string, DisplayUnit[]> = {};
  let currentRecordingCalibration: RecordingCalibration = {
    adc_reference_v: 5,
    mpxv_sensor_supply_v: 5,
    channel_units: {},
    active_calibrations: []
  };
  // The saved list lives in the Calibration & Units card; the page keeps the
  // selection the display and the next recording use.
  let calibrationCard: CalibrationCard | undefined;
  let calibrationChannelId = '';
  let selectedXgzpCalibrationId = '';
  // This is the calibration actually applied to live display/future recording.
  // It is deliberately an object, not only an ID-derived reactive value, so a
  // successful save can take effect immediately even while a list refresh runs.
  let activeXgzpCalibration: CalibrationPreset | undefined;
  let calibrationDialogOpen = false;
  // The message center's information line (action feedback, transient).
  let statusMessage = '';
  // A button shown after the information line while it shows `message`.
  let infoAction: { message: string; label: string; onClick: () => void } | undefined;
  // The text set while a Start is under way, replaced once acquisition begins.
  let startProgressMessage = '';
  let startFeedback = '';
  // What the last Start refusal or failure was about (see `forgetStaleStartFeedback`).
  let startFeedbackContextSeen = '';
  let startFeedbackTimer: number | undefined;
  let lastStartFailure: StartFailureRecord | undefined;
  let lastPlotError: PlotErrorRecord | undefined;
  let startInFlight = false;
  let displayedSummaryPath = '';
  let startReadiness: RecordingStartReadiness = { canStart: false };
  // Preparing until the first check finishes; Needs attention (with the
  // problem text and Repair Arduino tools) if preparation or the check fails.
  let arduinoToolsState: 'preparing' | 'ready' | 'failed' = 'preparing';
  let arduinoToolsProblem = '';
  let arduinoToolsDetail = '';
  // False when the tools are missing or damaged: only a reinstall helps.
  let arduinoToolsRepairable = true;
  // Why the window did not close (for example during a firmware restore).
  let closeBlockedNotice = '';
  let session: SessionStatus = {
    state: 'Disconnected', board: '', port: '', protocol_version: '0.1', simulator: false,
    samples: 0, packets: 0, measured_rate_hz: 0, integrity: emptyIntegrity,
    elapsed_seconds: 0
  };
  // The session status read in progress, if any (see `pollSession`).
  let pollInFlight: Promise<void> | undefined;
  // When the last status read started (idle reads are throttled).
  let lastStatusPollAt = 0;
  // Fetch display data on the next poll even while idle: the first poll and
  // a time-window change.
  let displayRefreshRequested = true;
  // Session state seen by the previous poll, so a fault message is set once
  // when the session becomes Faulted instead of on every 40 ms poll.
  let lastPolledState = 'Disconnected';
  // False until the first status read succeeds (a summary seen then is from before a reload).
  let sessionReadOnce = false;
  // The firmware status read in progress, if any (see `refreshFirmwareCompatibility`).
  let firmwarePollInFlight: Promise<void> | undefined;
  let firmwareCompatibility: FirmwareCompatibility = 'unknown';
  let firmwareWorkflow: FirmwareWorkflowStatus = { compatibility: 'unknown' };
  // Pre-Start storage projection and Project-folder free space (backend).
  let storageEstimate: RecordingStorageEstimate | undefined;
  let storageEstimateRequestId = 0;
  let storageEstimateTimer: number | undefined;
  let acquisitionProfiles: AcquisitionProfile[] = [];
  let selectedProfileId = 'wvu.bmeg420l.general.analog.development.v2';
  let operatingMode: OperatingMode = 'student';
  let modeChangeInFlight = false;
  let benchNoticeAcknowledged = false;
  let instructorAcknowledgement = false;
  let traceVisibility: VisibleChannelMap = {};
  let traceProfileKey = '';
  let plotGroups: PlotGroup[] = [];
  // Which setup cards are expanded (display-only). While a recording runs
  // they collapse, and `cardsSavedForRecording` holds the layout to restore.
  let cardOpen: CardOpenState = initialCardState(false);
  let cardsSavedForRecording: CardOpenState | undefined;
  // The mode whose card defaults were last applied (see `syncModeDisclosure`).
  let disclosureInstructor: boolean | undefined;
  let displayOptionsOpen = false;
  // Source default (see `shouldAutoSelectHardware`): Simulator until WVU
  // firmware is first verified on the selected board, then Hardware, once.
  // Set when the student picks a Source in the recording bar; the
  // auto-select never overrides that choice.
  let sourceChosenByUser = false;
  let sourceAutoSelected = false;
  // Firmware readiness at the previous check, so only a transition counts.
  let firmwareWasReady = false;
  // A recording ran or a Start was sent; the Source is never switched after that.
  let recordingSeen = false;
  // When the auto-select happened, so the next board message can mention it.
  let sourceAutoSelectedAt = 0;
  // A Verify Firmware handshake is running (the Board chip says Checking…).
  let verifyInFlight = false;
  // Display-only pause: the plots keep their frame; recording continues.
  let plotPaused = false;
  // Markers added during the current (or last) recording, with their
  // elapsed time computed when they were added. Cleared at the next Start.
  let recordingMarkers: Array<RecordingMarker & { seconds?: number }> = [];
  // Facts about the last finished recording that the summary does not carry.
  let summaryFacts: { path: string; markerCount: number; failureCategory?: string } | undefined;
  // The firmware step last announced in the information line.
  let announcedFirmwareStep = '';
  // The information line is transient; it clears itself after a while.
  const INFO_MESSAGE_MS = 15_000;
  // A message with a button (Show Last recording) stays long enough to use it.
  const INFO_ACTION_MESSAGE_MS = 60_000;
  let infoTimer: number | undefined;
  // Once a recording has started in this window, Live signals (then the
  // Last recording card) lead the page and Setup follows: the plots are on
  // screen while recording, and the result right after it. Before that,
  // Setup leads. The reading order follows the visual order.
  let liveFirst = false;
  // Acquisition state seen by the last check, so each transition acts once.
  let wasRecording = false;
  let wasAcquiring = false;
  // The sticky recording bar and message center (its height keeps focused
  // and scrolled-to controls out from under it).
  let topStack: HTMLElement | undefined;
  // At least this much of the first plot is brought on screen when a recording starts.
  const MIN_VISIBLE_PLOT_PX = 280;

  $: timedSeconds = durationPreset === 'custom' ? Number(customSeconds) : Number(durationPreset);
  // The lab selected for the next recording (setup, calibration, Start).
  $: activeProfile = acquisitionProfiles.find((profile) => profile.profile_id === selectedProfileId);
  $: activeChannels = profileChannels(activeProfile);
  $: pulseoxProfile = isPulseOxProfile(activeProfile);
  // A recording owns the session. Verify Firmware and Retry connection pass
  // through Connecting/Connected too; they are board checks, not recordings,
  // so they never collapse the setup, switch the plots, or count as a
  // recording for the Source auto-select.
  $: isActive = recordingActive(session);
  $: noteRecordingTransition(isActive, session.state === 'Acquiring');
  // Setup is locked only while a recording runs or a Start is being sent. A
  // Faulted session owns no serial handle, so it never locks the setup (the
  // backend clears the fault when the next recording starts).
  $: setupLocked = isActive || startInFlight;
  // While a recording runs the plots follow the lab actually being recorded
  // (`session.profile`), even if the lab list changed meanwhile. The key is a
  // string, so the plots are rebuilt only when that revision changes, not on
  // every status poll; `recordedLab()` reads the snapshot for that key.
  $: recordedLabKey = isActive ? labRevisionKey(session.profile?.profile) : '';
  $: plotProfile = recordedLabKey ? recordedLab() : activeProfile;
  $: plotPulseox = isPulseOxProfile(plotProfile);
  $: plotChannels = plotChannelsFor(plotProfile);
  $: calibratableChannels = linearCalibrationChannels(activeChannels);
  $: hasMpxvChannel = activeChannels.some((channel) => isFixedMpxvChannel(activeProfile?.category, channel));
  $: if (!calibratableChannels.some((channel) => channel.id === calibrationChannelId)) {
    calibrationChannelId = calibratableChannels[0]?.id ?? '';
  }
  $: plotProfileKey = `${plotProfile?.profile_id ?? ''}:${plotPulseox ? 'pulseox-raw' : 'analog'}:${plotChannels.map((channel) => channel.id).join('|')}`;
  // Visibility is reset only when the plotted lab's display fields change.  It is
  // never reconciled during a live checkbox toggle, which keeps checkbox DOM state and
  // uPlot series derived from one authoritative map. Display-only: the selected
  // calibration is reset by `selectProfile`, not here.
  $: if (plotProfileKey !== traceProfileKey) {
    traceProfileKey = plotProfileKey;
    traceVisibility = initialTraceVisibility(plotChannels);
    plotGroups = plotPulseox
      ? defaultPlotGroups(plotProfile?.category, plotChannels)
      : plotProfile?.plot_defaults?.groups?.length
      ? normalizePlotGroups(plotChannels, plotProfile.plot_defaults.groups.map((group, index) => ({ id: `profile-${index + 1}`, channelIds: group.channel_ids })))
      : defaultPlotGroups(plotProfile?.category, plotChannels);
    // Each channel's default display unit from the lab, when it is offered.
    channelUnits = initialChannelUnits(plotChannels, plotProfile?.category);
  }
  $: visibleTraceIds = visibleChannelIds(plotChannels, traceVisibility);
  // Numbered by slot, so headings match the "Plot N" assignment selectors.
  $: renderedPlotGroups = numberedVisiblePlotGroups(plotChannels, plotGroups, traceVisibility);
  // Keep this as a reactive value used directly by the template.  A helper
  // called only from markup can hide its dependencies from Svelte's legacy
  // compiler and leave <option> elements stale after a calibration changes.
  $: channelUnitOptions = Object.fromEntries(plotChannels.map((channel) => [
    channel.id,
    supportedDisplayUnits(
      plotProfile?.category,
      channel.id,
      Boolean(activeXgzpCalibration && selectedXgzpCalibrationId && activeXgzpCalibration.channel_id === channel.id),
      channel.allowed_conversions ?? [],
      activeXgzpCalibration?.channel_id === channel.id ? activeXgzpCalibration.output_units : 'mmHg'
    )
  ]));
  // A unit that is no longer offered (a deselected, deleted, or foreign
  // calibration) returns to volts, so no value is shown for a calibration
  // that is not applied. Display-only.
  $: {
    const reconciled = reconcileChannelUnits(channelUnits, channelUnitOptions);
    if (reconciled !== channelUnits) channelUnits = reconciled;
  }
  // Invalid Vref/Vs entries are reported and block Start; the display and the
  // recording keep the last valid value instead of turning an empty field into 0 V.
  $: adcReferenceError = calibrationVoltageError(adcReferenceV, 'the ADC reference voltage');
  $: mpxvSupplyError = calibrationVoltageError(mpxvSensorSupplyV, 'the MPXV sensor supply');
  $: if (!adcReferenceError) appliedAdcReferenceV = Number(adcReferenceV);
  $: if (!mpxvSupplyError) appliedMpxvSupplyV = Number(mpxvSensorSupplyV);
  $: calibrationInputError = adcReferenceError || (hasMpxvChannel ? mpxvSupplyError : '');
  // This is intentionally a direct reactive expression rather than a markup
  // helper call. It gives the live uPlot components the same fresh calibration
  // snapshot that a subsequent Start command will freeze into metadata.
  $: currentRecordingCalibration = recordingCalibrationFor(activeProfile, activeXgzpCalibration, appliedAdcReferenceV, appliedMpxvSupplyV, channelUnits);
  $: instructorModeActive = operatingMode === 'instructor_authoring';
  $: profileTimedPresets = activeProfile?.acquisition.timed_presets_seconds ?? DEFAULT_TIMED_PRESETS_SECONDS;
  // The lab's own duration rules; Start enforces the same rules in Rust.
  $: allowedDurationModes = labDurationModes(activeProfile);
  $: durationMode = allowedDurationMode(activeProfile, durationMode);
  $: minimumCustomSeconds = labMinimumCustomSeconds(activeProfile);
  $: timedDurationValid = labTimedDurationAllowed(activeProfile, timedSeconds);
  $: durationRefusal = labDurationRefusal(activeProfile, durationMode, timedSeconds);
  $: duration = durationRequest(durationMode, timedSeconds);
  $: catalogNoticeText = catalogNotice(catalogStatus, instructorModeActive);
  $: outputFolderError = relativeOutputFolderError(outputDirectory);
  // A failed verification deliberately leaves the session Faulted so its
  // diagnostics are available. Faulted does not own a serial handle and must
  // therefore still permit board selection, retry, and firmware restoration.
  $: boardOperationBusy = boardScanInFlight
    || Boolean(activeOperation)
    || firmwareWorkflow.job?.active === true
    || startInFlight;
  // Start is allowed only on the port where compatible firmware was proven,
  // matching the backend's `is_acquisition_allowed`.
  $: firmwareReadyOnSelectedPort = firmwareReadyOnPort(firmwareCompatibility, selectedPort, firmwareWorkflow.verified_port);
  $: boardControlState = boardControls({
    selectedBoard: Boolean(selectedPort),
    recordingActive: isActive,
    boardOperationBusy,
    arduinoToolsReady: arduinoToolsState === 'ready'
  });
  // Board panel: the chip, the one fitting action, and the last firmware operation.
  $: boardPanelStatus = boardStatus({
    selectedPort,
    compatibility: firmwareCompatibility,
    firmwareReady: firmwareReadyOnSelectedPort,
    checking: boardScanInFlight || verifyInFlight,
    firmwareJobActive: firmwareWorkflow.job?.active === true,
    lastFailureCategory: firmwareWorkflow.last_failure?.category
  });
  $: lastFirmwareOp = lastFirmwareOperation(firmwareWorkflow);
  $: announceFirmwareStep(firmwareWorkflow.job);
  $: applySourceAutoSelect(firmwareReadyOnSelectedPort, isActive || startInFlight);
  // Repair replaces the tools a recording or board operation is using, so it
  // waits until none is running (the backend refuses it as well).
  $: canRepairArduinoTools = !isActive && !boardOperationBusy;
  // Mirrors `requires_bench_acknowledgement`: a legacy-category gate for
  // bench-only "ecg"/"emg" labs (older or imported). Current course labs use
  // `course_*` categories and carry their notices in `safety.notices`.
  $: acknowledgementSatisfied = !activeProfile?.safety.bench_only
    || !['ecg', 'emg'].includes(activeProfile.category)
    || benchNoticeAcknowledged;
  $: startReadiness = recordingStartReadiness({
    source,
    selectedBoard: Boolean(selectedPort),
    firmwareReady: firmwareReadyOnSelectedPort,
    sessionState: session.state,
    sessionActive: isActive,
    faultOrigin: session.fault_origin,
    recordingRan: Boolean(session.last_summary),
    boardOperationBusy,
    startInFlight,
    activeProfile: Boolean(activeProfile),
    projectFolder: Boolean(projectFolder),
    outputFolderError,
    durationValid: !durationRefusal,
    durationMessage: durationRefusal,
    calibrationInputError,
    acknowledgementSatisfied
  });
  $: canStart = startReadiness.canStart;
  // A Start refusal or failure describes the setup it was given. Once any
  // of these change (the student fixed something), it no longer applies and
  // is cleared, so it cannot linger beside a Ready chip. A cause outside the
  // app (a drive plugged back in, another program closed) may never show
  // here, so the text also expires like the information line.
  $: forgetStaleStartFeedback(JSON.stringify([
    source, selectedPort, firmwareReadyOnSelectedPort, labRevisionKey(activeProfile), projectFolder, outputDirectory,
    durationMode, timedSeconds, acknowledgementSatisfied, calibrationInputError, session.state === 'Faulted',
    projectFolderNotice, Boolean(storageShortfall)
  ]));
  $: scheduleStartFeedbackExpiry(startFeedback, startInFlight);
  $: startFeedbackDisplay = startFeedbackView({
    sessionState: session.state,
    sessionActive: isActive,
    startInFlight,
    readiness: startReadiness,
    feedback: startFeedback
  });
  $: recoveryActions = connectionActions({
    source,
    active: isActive,
    selectedPort,
    failureCategory: session.connection_diagnostics?.failure_category
  });
  // Recovery is Retry connection, then Verify Firmware, then Restore WVU
  // Firmware (which handles the native-USB double reset).
  $: canRetryHandshake = recoveryActions.canRetryHandshake;
  $: effectiveOutputFolder = effectiveRecordingFolder(projectFolder, outputDirectory);
  // The storage projection comes from the backend (`estimate_recording_storage`),
  // the same BMEG + CSV sizes the storage guard uses. Display units do not
  // change it, so they are left out of the request. A finished recording
  // changes the free space, so its path is part of the key.
  $: lastRecordingPath = session.last_summary?.bmeg_path ?? '';
  $: storageEstimateRequest = activeProfile && !durationRefusal
    ? {
        profile_id: activeProfile.profile_id,
        duration,
        calibration: { ...currentRecordingCalibration, channel_units: {} },
        project_folder: projectFolder || null
      } satisfies StorageEstimateRequest
    : undefined;
  $: storageEstimateKey = storageEstimateRequest ? `${JSON.stringify(storageEstimateRequest)}|${lastRecordingPath}` : '';
  $: scheduleStorageEstimate(storageEstimateKey);
  $: freeBytes = displayedFreeBytes(isActive, session.available_disk_bytes, storageEstimate?.available_free_bytes);
  $: storageShortfall = durationMode === 'timed' ? storageShortfallWarning(storageEstimate) : undefined;

  // ---- Recording bar, readiness checklist, and message center ----
  $: selectedBoardEntry = boards.find((board) => board.port === selectedPort);
  // Only a hardware Start waits for a Faulted session to be cleared.
  $: blockingFault = session.state === 'Faulted' && source === 'hardware';
  $: faultInput = {
    faultOrigin: session.fault_origin,
    failureCategory: session.connection_diagnostics?.failure_category,
    lastError: session.last_error,
    // Not `session.simulator` alone: a later Verify or Retry connection keeps
    // that flag (the watermark needs it) but is a board check.
    simulatorSession: simulatorRecordingFault(session),
    recordingRan: Boolean(session.last_summary),
    source,
    port: selectedPort
  };
  // A fault the student must deal with for the chosen Source. A failed board
  // check during Simulator practice is shown in the Board card instead.
  $: faultShown = session.state === 'Faulted' && faultConcernsSource(faultInput);
  $: readinessItems = readinessChecklist({
    source,
    selectedPort,
    firmwareReady: firmwareReadyOnSelectedPort,
    blockingFault,
    faultAction: faultShown ? problem?.nextAction : undefined,
    labName: activeProfile?.display_name,
    acknowledgementSatisfied,
    durationError: durationRefusal || undefined,
    adcReferenceError: adcReferenceError || undefined,
    mpxvSupplyError: hasMpxvChannel ? mpxvSupplyError || undefined : undefined,
    projectFolder,
    outputFolderError
  });
  $: barChecklist = !isActive && !startInFlight && !canStart ? readinessItems : undefined;
  // The checklist already names a failing setup item; the sentence is kept
  // for the other reasons (busy, Start in progress, a Start failure).
  $: barCaption = isActive
    ? ''
    : startFeedbackDisplay.tone === 'warning' && barChecklist?.some((item) => item.status === 'fail')
      ? ''
      : startFeedbackDisplay.text;
  $: chip = recordingChip({ sessionState: session.state, sessionActive: isActive, faulted: faultShown, startInFlight, canStart, source, boardSelected: Boolean(selectedPort) });
  $: progress = recordingProgress(session.duration, session.elapsed_seconds, session.remaining_seconds);
  // Before Start the bar follows the Source choice; during a recording it
  // follows the session actually running.
  $: simulatedView = isActive ? session.simulator : source === 'simulator';
  // The plots keep the last recording's frames after it ends, so the
  // watermark follows where the shown data came from; with nothing shown
  // yet it follows the Source choice.
  $: plotDataSimulated = isActive || samples.length ? session.simulator : source === 'simulator';
  $: problem = problemBanner({
    sessionState: session.state,
    active: isActive,
    fault: faultInput,
    storageWarning: session.storage_warning,
    arduinoTools: { state: arduinoToolsState, problem: arduinoToolsProblem, repairable: arduinoToolsRepairable },
    projectFolderNotice,
    board: {
      source,
      selectedPort,
      firmwareReady: firmwareReadyOnSelectedPort,
      compatibility: firmwareCompatibility,
      lastFailureCategory: firmwareWorkflow.last_failure?.category,
      firmwareJobActive: firmwareWorkflow.job?.active === true,
      // Nothing is reported about the board until its check has finished.
      checking: boardScanInFlight || verifyInFlight
    },
    storageShortfall,
    catalogNotice: catalogNoticeText
  });
  // Whether a banner action can run now. The function is rebuilt whenever
  // anything it reads changes, so the buttons follow every dependency.
  $: actionBlocked = (action: NextAction | undefined): boolean =>
    action === 'retry_connection' ? !canRetryHandshake || boardOperationBusy
      : action === 'verify_firmware' ? !boardControlState.canVerifyFirmware
      : action === 'restore_firmware' ? !boardControlState.canRestoreFirmware
      : action === 'clear_connection' ? startInFlight || session.state === 'Disconnected'
      : action === 'choose_folder' ? setupLocked
      : action === 'repair_arduino_tools' ? !canRepairArduinoTools
      : action === 'refresh_board' ? !boardControlState.canRefreshBoards
      : false;
  $: problemActionDisabled = actionBlocked(problem?.nextAction);
  $: secondaryActionDisabled = actionBlocked(problem?.secondary?.action);
  // A fault shown in the banner: Clear connection is always offered, in the bar unless the banner already offers it.
  $: showClearConnection = faultShown && problem?.nextAction !== 'clear_connection';
  $: scheduleInfoClear(statusMessage);
  $: shownInfoAction = infoAction?.message === statusMessage ? infoAction : undefined;

  // ---- Setup cards (display-only disclosure) ----
  $: syncRecordingDisclosure(isActive);
  $: syncModeDisclosure(instructorModeActive);
  $: boardCardSummary = boardSummary({
    port: selectedPort,
    boardName: selectedBoardEntry?.name,
    firmwareReady: firmwareReadyOnSelectedPort,
    compatibility: firmwareCompatibility,
    firmwareJobActive: firmwareWorkflow.job?.active === true,
    toolsState: arduinoToolsState
  });
  $: projectCardSummary = projectSummary(projectFolder, projectFolderNotice);
  $: labCardSummary = labSummary(activeProfile, instructorModeActive);
  $: rateText = activeProfile ? `${activeProfile.acquisition.sample_rate_hz} ${pulseoxProfile ? 'cycles/s' : 'frames/s'}` : '';
  $: sessionCardSummary = sessionSummary({ outputFolder: outputDirectory, note, rateText });
  $: durationCardSummary = durationSummary(durationMode, timedSeconds, !durationRefusal);
  $: calibrationCardSummary = calibrationSummary({
    channelIds: plotChannels.map((channel) => channel.id),
    units: channelUnits,
    calibrationLabel: activeXgzpCalibration?.label
  });
  $: plotSummary = plotArrangementSummary(plotGroups.length, visibleTraceIds.length, plotChannels.length);

  // ---- Live signals (display-only) and the finished recording ----
  $: plotMarkers = placeMarkers(recordingMarkers);
  $: lastSummary = session.last_summary ?? undefined;
  $: resultView = lastSummary
    ? recordingResultView(lastSummary, {
        markerCount: summaryFacts?.path === lastSummary.bmeg_path ? summaryFacts.markerCount : 0,
        failureCategory: summaryFacts?.path === lastSummary.bmeg_path ? summaryFacts.failureCategory : undefined
      })
    : undefined;

  /**
   * Source default: switch to Hardware once, on the first transition to
   * firmware ready on the selected port while nothing is recording; never
   * during or after a recording, and never after the student chose a Source.
   */
  function applySourceAutoSelect(ready: boolean, sessionActive: boolean) {
    if (sessionActive) recordingSeen = true;
    const switchNow = shouldAutoSelectHardware({
      source,
      autoSelected: sourceAutoSelected,
      userChose: sourceChosenByUser,
      recordingSeen,
      sessionActive,
      wasReady: firmwareWasReady,
      ready
    });
    firmwareWasReady = ready;
    if (!switchNow) return;
    source = 'hardware';
    sourceAutoSelected = true;
    sourceAutoSelectedAt = Date.now();
    statusMessage = withSourceNotice(`Firmware ready on ${selectedPort}.`);
  }

  /** Adds "Source is now Hardware" to a board message right after the auto-select. */
  function withSourceNotice(message: string): string {
    if (!sourceAutoSelectedAt || Date.now() - sourceAutoSelectedAt > 5_000) return message;
    return `${message} Source is now Hardware · ${selectedPort}; choose Simulator (practice) in the recording bar to practise without the board.`;
  }

  /** Announces each new restore step once, in the page's single live region. */
  function announceFirmwareStep(job: FirmwareJob | null | undefined) {
    if (!job?.active) {
      announcedFirmwareStep = '';
      return;
    }
    const key = `${job.id}:${firmwareStepIndex(job.stage) ?? job.stage}:${job.manual_reset_deadline_utc ? 'reset' : ''}`;
    if (key === announcedFirmwareStep) return;
    announcedFirmwareStep = key;
    statusMessage = job.manual_reset_deadline_utc
      ? 'Restoring WVU Firmware: press the RESET button on the board twice quickly.'
      : firmwareStepAnnouncement(job) || statusMessage;
  }

  /** The lab snapshot of the running recording (see `recordedLabKey`). */
  function recordedLab(): AcquisitionProfile | undefined {
    return session.profile?.profile ?? undefined;
  }

  /** Collapses the setup cards while a recording runs and restores them afterwards. */
  function syncRecordingDisclosure(active: boolean) {
    const next = recordingDisclosure({ open: cardOpen, saved: cardsSavedForRecording }, active);
    if (next.open !== cardOpen) cardOpen = next.open;
    if (next.saved !== cardsSavedForRecording) cardsSavedForRecording = next.saved;
  }

  /**
   * A recording started or acquisition began. Display-only: the plots lead
   * the page from now on and are brought on screen; once samples flow, the
   * "Starting…" text in the information line is replaced.
   */
  function noteRecordingTransition(active: boolean, acquiring: boolean) {
    if (active && !wasRecording) {
      liveFirst = true;
      void revealLivePlots();
    }
    wasRecording = active;
    if (acquiring && !wasAcquiring && startProgressMessage && statusMessage === startProgressMessage) {
      statusMessage = 'Recording started.';
    }
    if (!active) startProgressMessage = '';
    wasAcquiring = acquiring;
  }

  /**
   * Scrolls the Live signals panel up under the sticky bar when less than
   * MIN_VISIBLE_PLOT_PX of the first plot is on screen. Focus stays where it is.
   */
  async function revealLivePlots() {
    await tick();
    const panel = document.getElementById('live-panel');
    const plot = panel?.querySelector<HTMLElement>('.stacked-plot') ?? panel;
    if (panel && plot) revealIfHidden(panel, plot, topStack?.getBoundingClientRect().bottom ?? 0, MIN_VISIBLE_PLOT_PX);
  }

  /** Clears the text under Start once the setup it described has changed. */
  function forgetStaleStartFeedback(context: string) {
    if (context === startFeedbackContextSeen) return;
    startFeedbackContextSeen = context;
    if (!startInFlight) startFeedback = '';
  }

  /** A Start refusal or failure leaves the bar when the information line clears. */
  function scheduleStartFeedbackExpiry(feedback: string, inFlight: boolean) {
    if (startFeedbackTimer !== undefined) window.clearTimeout(startFeedbackTimer);
    startFeedbackTimer = undefined;
    if (!feedback || inFlight) return;
    startFeedbackTimer = window.setTimeout(() => {
      startFeedbackTimer = undefined;
      if (!startInFlight) startFeedback = '';
    }, INFO_MESSAGE_MS);
  }

  /** The information line's "Show Last recording": scrolls to the card and focuses its heading. */
  async function showLastRecording() {
    await tick();
    const heading = document.getElementById('result-title');
    if (heading) scrollToAndFocus(heading, { block: 'start' });
  }

  /**
   * Publishes the sticky stack's height as --sticky-height, which sets the
   * page's scroll-padding-top: focused and scrolled-to controls stop below
   * the bar however tall it is (a banner, a wrapped row, a checklist).
   */
  function publishStickyHeight() {
    if (!topStack) return;
    document.documentElement.style.setProperty('--sticky-height', `${Math.ceil(topStack.getBoundingClientRect().height)}px`);
  }

  /**
   * Student mode starts with Calibration & Units collapsed; Instructor mode
   * opens it. Applied when the mode changes, not on every update, so a card
   * the user opened or closed stays that way.
   */
  function syncModeDisclosure(instructor: boolean) {
    if (disclosureInstructor === instructor) return;
    const first = disclosureInstructor === undefined;
    disclosureInstructor = instructor;
    if (first && !instructor) return;
    if (cardsSavedForRecording) cardsSavedForRecording = withModeDefaults(cardsSavedForRecording, instructor);
    else cardOpen = withModeDefaults(cardOpen, instructor);
  }

  function scheduleInfoClear(message: string) {
    if (infoTimer !== undefined) window.clearTimeout(infoTimer);
    infoTimer = undefined;
    if (message) {
      infoTimer = window.setTimeout(() => {
        infoTimer = undefined;
        statusMessage = '';
      }, infoAction?.message === message ? INFO_ACTION_MESSAGE_MS : INFO_MESSAGE_MS);
    }
  }

  /**
   * A readiness item was selected: open the card that holds its control,
   * then scroll to it and focus it. If the control cannot take focus (for
   * example a disabled button) the card heading takes it instead.
   */
  async function focusReadinessTarget(target: ChecklistTarget) {
    const location = CHECKLIST_TARGETS[target];
    if (location.card) cardOpen = openCard(cardOpen, location.card);
    await tick();
    const element = location.elementIds
      .map((id) => document.getElementById(id))
      .find((candidate): candidate is HTMLElement => candidate !== null);
    const fallback = location.card ? document.getElementById(`${location.card}-toggle`) : null;
    const destination = element ?? fallback;
    if (destination) scrollToAndFocus(destination, { fallback });
  }

  /** The message center's one next-action button. */
  async function runNextAction(action: NextAction) {
    switch (action) {
      case 'retry_connection': return retryHandshake();
      case 'verify_firmware': return void await verifySelectedFirmware();
      case 'restore_firmware': return restoreWvuFirmware();
      case 'clear_connection': return disconnect();
      case 'choose_folder': return chooseProjectFolder();
      case 'repair_arduino_tools': return repairArduinoTools();
      case 'refresh_board': return refreshBoardAndVerify();
    }
  }

  /**
   * Refresh Board from the message center. A board that came back on the
   * same COM port is verified as well, which also clears the earlier fault
   * (verification is a read-only handshake; it never uploads firmware).
   */
  async function refreshBoardAndVerify() {
    const verified = await refreshBoards('manual');
    // Let the reactive board controls catch up before they are read.
    await tick();
    if (!verified && selectedPort && session.state === 'Faulted' && !isActive) await verifySelectedFirmware(selectedPort);
  }

  /** The student picked a Source in the recording bar. */
  function sourceChosenByStudent() {
    sourceChosenByUser = true;
  }

  /**
   * Asks the backend for the storage projection and the Project folder's
   * free space shortly after the inputs settle. Only the newest answer is
   * applied. Reads only; never an acquisition command.
   */
  function scheduleStorageEstimate(key: string) {
    if (storageEstimateTimer !== undefined) window.clearTimeout(storageEstimateTimer);
    storageEstimateTimer = undefined;
    const requestId = ++storageEstimateRequestId;
    const request = storageEstimateRequest;
    if (!key || !request) {
      storageEstimate = undefined;
      return;
    }
    storageEstimateTimer = window.setTimeout(async () => {
      storageEstimateTimer = undefined;
      try {
        const estimate = await invoke<RecordingStorageEstimate>('estimate_recording_storage', storageEstimateInvokePayload(request));
        if (requestId === storageEstimateRequestId) storageEstimate = estimate;
      } catch (error) {
        if (requestId === storageEstimateRequestId) storageEstimate = undefined;
        console.warn('Could not estimate recording storage', error);
      }
    }, 250);
  }

  /**
   * Shows the newest operation still running. Each run removes only its own
   * entry, so operations that overlap without nesting can never bring back
   * a finished operation's modal.
   */
  async function runOperation<T>(operation: ActiveOperation, action: () => Promise<T>): Promise<T> {
    const token = ++operationToken;
    runningOperations = [...runningOperations, { token, operation }];
    activeOperation = operation;
    try {
      return await action();
    } finally {
      runningOperations = runningOperations.filter((entry) => entry.token !== token);
      activeOperation = runningOperations[runningOperations.length - 1]?.operation;
    }
  }

  const TOOLS_CHECK_FAILED = 'The Arduino tools could not be checked. Select Repair Arduino tools. If that does not help, restart the application or contact your instructor.';

  function arduinoToolsFailed(problem: string, detail = '', repairable = true) {
    arduinoToolsState = 'failed';
    arduinoToolsProblem = problem;
    arduinoToolsDetail = detail;
    arduinoToolsRepairable = repairable;
    firmwareEnvironment = { expected_fqbn: 'arduino:renesas_uno:unor4wifi', boards: [], ready: false, problem };
  }

  async function refreshEnvironmentSummary() {
    try {
      firmwareEnvironment = await invoke<FirmwareEnvironment>('firmware_environment');
      arduinoToolsState = firmwareEnvironment.ready ? 'ready' : 'failed';
      arduinoToolsProblem = firmwareEnvironment.ready ? '' : firmwareEnvironment.problem ?? TOOLS_CHECK_FAILED;
      arduinoToolsDetail = '';
      arduinoToolsRepairable = true;
    } catch {
      arduinoToolsFailed(TOOLS_CHECK_FAILED);
    }
  }

  async function prepareArduinoTools() {
    try {
      const runtime = await runOperation(
        { title: 'Preparing Arduino tools…', stage: 'Setting up the included offline Arduino tools for this computer.', cancelable: false },
        () => invoke<ArduinoRuntimeStatus>('prepare_arduino_runtime')
      );
      if (runtime.ready) {
        arduinoToolsState = 'ready';
        arduinoToolsProblem = '';
        arduinoToolsDetail = '';
      } else {
        arduinoToolsFailed(TOOLS_CHECK_FAILED);
      }
    } catch (error) {
      // An `ArduinoToolsFailure`: title, explanation, and Repair or reinstall.
      // The Board card (or, with Source = Hardware, the banner) shows it once;
      // the information line does not repeat it.
      arduinoToolsFailed(formatCommandError(error), commandErrorDetail(error), commandErrorRepairable(error));
    }
  }

  /** Extracts the included offline tools again; nothing is downloaded. */
  async function repairArduinoTools() {
    if (!canRepairArduinoTools) return;
    try {
      await runOperation(
        { title: 'Repairing Arduino tools…', stage: 'Extracting the included offline Arduino tools again. This can take a few minutes.', cancelable: false },
        () => invoke<ArduinoRuntimeStatus>('repair_arduino_runtime')
      );
      await refreshEnvironmentSummary();
    } catch (error) {
      if (commandErrorKeptPreviousTools(error)) {
        // The backend checked that the tools already in use are still in
        // place, so their state (and Restore) stays as it was.
        arduinoToolsDetail = commandErrorDetail(error);
        statusMessage = `The Arduino tools were not repaired; the tools already in place were kept. ${formatCommandError(error)}`;
        return;
      }
      arduinoToolsFailed(formatCommandError(error), commandErrorDetail(error), commandErrorRepairable(error));
      statusMessage = 'The Arduino tools were not repaired. The Board card explains what to do next.';
      return;
    }
    if (arduinoToolsState !== 'ready') {
      statusMessage = 'The Arduino tools still need attention. The Board card explains what to do next.';
      return;
    }
    await refreshBoards('startup');
    statusMessage = `Arduino tools were repaired. ${statusMessage}`;
  }

  async function verifySelectedFirmware(
    port = selectedPort,
    allowWithinBoardOperation = false
  ): Promise<FirmwareVerification | undefined> {
    if (!port || isActive || (!allowWithinBoardOperation && !boardControlState.canVerifyFirmware)) return undefined;
    verifyInFlight = true;
    try {
      return await runOperation(
        { title: 'Verifying firmware…', stage: 'Opening the selected UNO R4 WiFi and performing a read-only WVU protocol handshake.', cancelable: false },
        async () => {
          try {
            const verification = await invoke<FirmwareVerification>('verify_wvu_reference_firmware', { port });
            await refreshFirmwareCompatibility();
            await pollSession();
            statusMessage = verification.compatible
              ? withSourceNotice(`Firmware ready on ${port}.`)
              : `Verify Firmware finished: ${port} is not running the WVU firmware.`;
            return verification;
          } catch (error) {
            await refreshFirmwareCompatibility();
            await pollSession();
            // For example "Serial port is busy … close Arduino IDE … Verify
            // Firmware again"; the previous firmware status is unchanged.
            statusMessage = formatCommandError(error);
            return undefined;
          }
        }
      );
    } finally {
      verifyInFlight = false;
    }
  }

  /**
   * Returns true when the refresh also verified a newly selected board.
   * `verifyNewSelection: false` selects such a board without verifying it
   * (see `reconcileBoardCache`).
   */
  async function refreshBoards(
    reason: 'startup' | 'manual' | 'transition' = 'manual',
    { verifyNewSelection = true }: { verifyNewSelection?: boolean } = {}
  ): Promise<boolean> {
    if (boardScanInFlight || (reason === 'manual' && !boardControlState.canRefreshBoards)) return false;
    boardScanInFlight = true;
    boardScanStatus = 'scanning';
    boardScanError = '';
    let verified = false;
    try {
      await runOperation(
          { title: 'Detecting Arduino boards…', stage: reason === 'startup' ? 'Looking for a connected Arduino UNO R4 WiFi.' : 'Refreshing the list of connected Arduino boards.', cancelable: false },
        async () => {
          const scan = reconcileBoardCache(selectedPort, await invoke<Board[]>('list_boards'), { verifyNewSelection });
          boards = scan.boards;
          boardScanLastCompleted = new Date().toLocaleTimeString();
          boardScanStatus = 'complete';
          selectedPort = scan.selectedPort;
          if (scan.verificationPort) {
            verified = true;
            await verifySelectedFirmware(scan.verificationPort, true);
          }
          statusMessage = withSourceNotice(boards.length
            ? `${boards.length} Arduino UNO R4 WiFi board${boards.length === 1 ? '' : 's'} found${selectedPort ? `; ${selectedPort} is selected.` : '. Select a board to continue.'}`
            : 'Arduino not detected. Check the USB connection, then select Refresh Board.');
        }
      );
    } catch (error) {
      boardScanStatus = 'error';
      boardScanError = String(error);
      statusMessage = 'The board list could not be refreshed. Check the USB connection and try Refresh Board again.';
    } finally {
      boardScanInFlight = false;
    }
    return verified;
  }

  /**
   * The 40 ms timer skips a tick while a status read is still running, and
   * reads only a few times a second while no recording runs or starts.
   */
  function pollSessionTick() {
    if (pollInFlight) return;
    if (!statusPollDue(isActive || startInFlight, lastStatusPollAt, performance.now())) return;
    void pollSession();
  }

  /**
   * Reads a fresh session status. A command that changed the session awaits
   * this and then sets its own message: a read that started before the
   * command finished is waited for first, so it cannot overwrite that message
   * afterwards with a stale or generic one.
   */
  async function pollSession(): Promise<void> {
    while (pollInFlight) await pollInFlight;
    const poll = readSessionStatus();
    pollInFlight = poll;
    try {
      await poll;
    } finally {
      if (pollInFlight === poll) pollInFlight = undefined;
    }
  }

  async function readSessionStatus() {
    lastStatusPollAt = performance.now();
    try {
      session = await invoke<SessionStatus>('get_session_status');
      // Idle polls neither fetch nor redraw the display data: only an active
      // session adds frames. One more fetch after it ends shows the final
      // frames, and a time-window change asks for one explicitly. While the
      // display is paused nothing is fetched (status polling continues);
      // Resume asks for the newest frames.
      if (!plotPaused && shouldFetchDisplayData({ state: session.state, previousState: lastPolledState, refreshRequested: displayRefreshRequested })) {
        displayRefreshRequested = false;
        samples = await invoke<Point[]>('get_recent_display_data', {
          windowSeconds: plotTimeWindowSeconds,
          maxPoints: MAX_RENDERED_DISPLAY_POINTS
        });
        displayRevision += 1;
      }
      // A Faulted session is explained by the message center's banner
      // (`problemBanner`), which stays until the fault is cleared; the
      // information line is left for action feedback.
      lastPolledState = session.state;
      const firstRead = !sessionReadOnce;
      sessionReadOnce = true;
      const failureCategory = session.connection_diagnostics?.failure_category ?? undefined;
      if (session.last_summary && session.last_summary.bmeg_path !== displayedSummaryPath) {
        displayedSummaryPath = session.last_summary.bmeg_path;
        const summary = session.last_summary;
        // The markers and the fault cause belong to this recording; they
        // are kept with its path so a later session cannot change them.
        summaryFacts = {
          path: summary.bmeg_path,
          markerCount: recordingMarkers.length,
          failureCategory: session.state === 'Faulted' ? failureCategory : undefined
        };
        if (firstRead) {
          // The page was reloaded after a recording: the backend still holds
          // it, so the after-recording layout is kept (Live signals and the
          // Last recording card first) without announcing the recording again.
          liveFirst = true;
          recordingSeen = true;
        } else {
          statusMessage = `${summary.completion_status === 'complete' ? 'Recording saved' : 'Recording ended before completion'}: ${formatCount(summary.samples)} samples at ${formatRate(summary.measured_rate_hz)}. The files are listed under Last recording.`;
          infoAction = { message: statusMessage, label: 'Show Last recording', onClick: () => void showLastRecording() };
        }
      } else if (summaryFacts && !summaryFacts.failureCategory && session.state === 'Faulted' && failureCategory
        && session.last_summary?.bmeg_path === summaryFacts.path && session.last_summary.stop_reason === 'fault') {
        // The fault is published just after the summary.
        summaryFacts = { ...summaryFacts, failureCategory };
      }
    } catch (error) {
      statusMessage = 'The recording status could not be updated. Try reconnecting the Arduino.';
    }
  }

  /** The 750 ms timer skips a tick while a firmware status read is running. */
  function firmwareStatusTick() {
    if (!firmwarePollInFlight) void refreshFirmwareCompatibility();
  }

  /**
   * Reads a fresh firmware status. Reads never overlap (a slow read is waited
   * for, not duplicated), so exactly one read sees a firmware job finish and
   * reports it. The report itself runs after the read is released, because
   * it verifies the board, which reads the firmware status again.
   */
  async function refreshFirmwareCompatibility(): Promise<void> {
    while (firmwarePollInFlight) await firmwarePollInFlight;
    let jobFinished = false;
    const read = (async () => { jobFinished = await readFirmwareStatus(); })();
    firmwarePollInFlight = read;
    try {
      await read;
    } finally {
      if (firmwarePollInFlight === read) firmwarePollInFlight = undefined;
    }
    if (jobFinished) await reportFinishedFirmwareJob();
  }

  /** Returns true when this read saw an active firmware job finish. */
  async function readFirmwareStatus(): Promise<boolean> {
    try {
      const firmwareJobWasActive = firmwareWorkflow.job?.active === true;
      firmwareWorkflow = await invoke<FirmwareWorkflowStatus>('get_firmware_workflow_status');
      firmwareCompatibility = firmwareWorkflow.compatibility;
      return firmwareJobWasActive && !firmwareWorkflow.job?.active;
    } catch {
      firmwareCompatibility = 'unknown';
      firmwareWorkflow = { compatibility: 'unknown' };
      return false;
    }
  }

  async function reportFinishedFirmwareJob() {
    closeBlockedNotice = '';
    const finishedJob = firmwareWorkflow.last_restore;
    const finishedFailure = finishedJob?.failure ?? firmwareWorkflow.last_failure;
    // A native-USB restore intentionally returns on a different COM port
    // from the ESP32 bridge used for upload. Refresh exactly once after a
    // terminal firmware job so the selected board cannot remain stale. Only
    // a completed restore verifies a newly selected board: after a failure
    // or cancel, the Board card keeps the restore's own failure, log, and
    // advice (Verify and Restore stay available).
    await refreshBoards('transition', { verifyNewSelection: finishedJob?.stage === 'complete' });
    // Refresh the independent acquisition state immediately as well; it
    // must not wait for a route change or retain a stale Faulted snapshot.
    await pollSession();
    // Report the restore result last, so the board count does not hide it.
    statusMessage = finishedJob?.stage === 'complete'
      ? `WVU Firmware was restored and verified on ${finishedJob.final_port ?? selectedPort}.`
      : finishedFailure
        ? `WVU Firmware was not restored. ${formatCommandError(finishedFailure)}`
        : 'WVU Firmware was not restored. Open Advanced details for more information.';
  }

  async function cancelFirmwareJob() {
    try {
      await invoke('cancel_firmware_job');
      await refreshFirmwareCompatibility();
    } catch (error) {
      statusMessage = formatCommandError(error);
    }
  }

  async function selectedBoardChanged() {
    // The new board is checked at once; until then nothing is reported about it.
    verifyInFlight = Boolean(selectedPort);
    try {
      // `boardControlState` is a `$:` value; let it see the new selection first.
      await tick();
      if (selectedPort && boardControlState.canVerifyFirmware) await verifySelectedFirmware(selectedPort);
    } finally {
      verifyInFlight = false;
    }
  }

  /**
   * Reloads the lab list. The selection is not changed here: callers apply
   * it with `selectProfile`, which also reloads that lab's calibrations.
   */
  async function refreshProfiles() {
    try {
      acquisitionProfiles = await invoke<AcquisitionProfile[]>('list_acquisition_profiles');
      const confirmedMode = await invoke<OperatingMode>('get_profile_mode');
      if (!modeChangeInFlight) operatingMode = confirmedMode;
    } catch (error) {
      statusMessage = `Profile error: ${String(error)}`;
    }
    try {
      catalogStatus = await invoke<CatalogStatus>('get_catalog_status');
    } catch (error) {
      console.warn('Could not read the lab catalog status', error);
    }
  }

  async function chooseProjectFolder() {
    const selected = await open({ directory: true, multiple: false, defaultPath: projectFolder || undefined });
    if (typeof selected !== 'string') return;
    try {
      const saved = await invoke<{ project_folder: string }>('set_project_folder', { projectFolder: selected });
      projectFolder = saved.project_folder;
      projectFolderNotice = '';
      // The same folder may have more free space now (the usual answer to
      // "Not enough free space"), so it is measured again.
      await tick();
      scheduleStorageEstimate(storageEstimateKey);
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
    const port = selectedPort;
    try {
      // The command checks the board before the job starts; keep conflicting
      // actions blocked while it does.
      const job = await runOperation(
        { title: 'Restoring WVU Firmware…', stage: `Checking ${port} before the restore begins.`, cancelable: false },
        () => invoke<FirmwareJob>('restore_wvu_reference_firmware', { request: { port, confirmation: true } })
      );
      firmwareWorkflow = { ...firmwareWorkflow, job };
      statusMessage = 'Restoring WVU Firmware. The board will be verified when the update finishes.';
    } catch (error) {
      statusMessage = `WVU Firmware could not be restored. ${formatCommandError(error)}`;
    }
  }

  /** The selected lab if it is still listed, else the first listed lab. */
  function listedProfileId(profileId: string): string {
    return acquisitionProfiles.some((profile) => profile.profile_id === profileId)
      ? profileId
      : acquisitionProfiles[0]?.profile_id ?? '';
  }

  async function labRevisionSaved(profile: AcquisitionProfile) {
    await refreshProfiles();
    // Lab Manager is disabled while recording; never switch a running session's page.
    if (isActive) return;
    selectProfile(listedProfileId(profile.profile_id));
  }

  /**
   * The catalog changed without a new revision to select (Retire, Reset in
   * Lab Manager, or a refused Start because the lab changed). Reloads the
   * list and re-applies the lab if the revision on screen is no longer the
   * active one, so Start never records a lab other than the one shown.
   */
  async function labCatalogChanged() {
    const shownRevision = labRevisionKey(activeProfile);
    await refreshProfiles();
    if (isActive) return;
    const profileId = listedProfileId(selectedProfileId);
    const current = acquisitionProfiles.find((profile) => profile.profile_id === profileId);
    if (profileId !== selectedProfileId || labRevisionKey(current) !== shownRevision) {
      selectProfile(profileId, { announce: false });
    }
  }

  /**
   * Applies a lab selection. The new lab is looked up here rather than read
   * from `$:` values (`activeProfile`, `profileTimedPresets`, ...), which
   * still describe the previous lab until after this handler returns.
   */
  function selectProfile(profileId: string, options: { announce?: boolean } = {}) {
    selectedProfileId = profileId;
    const profile = acquisitionProfiles.find((candidate) => candidate.profile_id === profileId);
    benchNoticeAcknowledged = false;
    // Force a clean visibility map for the newly selected profile. The reactive
    // profile key also protects programmatic profile changes.
    traceProfileKey = '';
    durationPreset = defaultDurationPreset(profile);
    if (options.announce !== false) {
      statusMessage = profile
        ? `${profile.display_name} is selected. Lab settings are ready.`
        : 'Choose a course lab to continue.';
    }
    // The previous lab's calibrations never carry over to this one.
    const channelId = calibrationChannelFor(profileChannels(profile), calibrationChannelId);
    calibrationChannelId = channelId;
    selectedXgzpCalibrationId = '';
    activeXgzpCalibration = undefined;
    calibrationCard?.loadLab(profile?.profile_id ?? '', channelId);
  }

  /** Display-only: re-reads the bounded display data with the new window. */
  function refreshDisplayWindow() {
    displayRefreshRequested = true;
    void pollSession();
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

  async function commitProfileMode(mode: OperatingMode) {
    modeChangeInFlight = true;
    try {
      operatingMode = await invoke<OperatingMode>('set_profile_mode', {
        mode,
        acknowledgement: mode === 'instructor_authoring' && instructorAcknowledgement
      });
      statusMessage = operatingMode === 'instructor_authoring'
        ? 'Instructor mode is enabled. You can manage lab settings.'
        : 'Student mode is enabled. Lab management is hidden. Instructor mode is a safety step against accidental changes, not a password lock.';
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
    if (modeChangeInFlight || setupLocked) return;
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
    if (!activeProfile) return;
    startInFlight = true;
    lastStartFailure = undefined;
    startFeedback = source === 'hardware' ? 'Connecting to Arduino…' : 'Starting simulator recording…';
    statusMessage = startFeedback;
    startProgressMessage = statusMessage;
    // The lab revision on screen; Rust refuses Start if the catalog's active
    // revision of this lab differs (for example after Reset in Manage Labs).
    const request: SimulatorStartRequest = {
      project_folder: projectFolder,
      output_folder: outputDirectory,
      duration,
      profile_id: activeProfile.profile_id,
      profile_version: activeProfile.profile_version,
      bench_notice_acknowledged: benchNoticeAcknowledged,
      recording_note: note,
      calibration: currentRecordingCalibration
    };
    try {
      session = source === 'simulator'
        ? await invoke<SessionStatus>('start_profile_simulator_recording', simulatorStartInvokePayload(request))
        : await invoke<SessionStatus>('start_profile_hardware_recording', hardwareStartInvokePayload<HardwareStartRequest>({
            ...request,
            port: selectedPort
          }));
      // The session state now speaks for itself under Start; no Start text
      // may linger after the recording ends.
      startFeedback = '';
      // A new recording: its own markers, and live plots (not a paused frame).
      recordingMarkers = [];
      if (plotPaused) {
        plotPaused = false;
        displayRefreshRequested = true;
      }
      statusMessage = source === 'hardware' ? 'Configuring recording…' : 'Starting recording…';
      // Replaced by "Recording started." once samples flow (see noteRecordingTransition).
      startProgressMessage = statusMessage;
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
      // The lab changed under the page: reload it so what is shown is what
      // the next Start records.
      if (failure.code === 'lab_changed') await labCatalogChanged();
    } finally {
      startInFlight = false;
      await pollSession();
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

  /**
   * Adds a marker with an optional short label. The confirmation gives the
   * elapsed recording time (the plot's time axis), not board microseconds.
   */
  async function addMarker(label: string): Promise<boolean> {
    try {
      const marker = await invoke<RecordingMarker>('add_recording_marker', { label });
      const origin = session.display_origin_timestamp_us;
      recordingMarkers = [...recordingMarkers, { ...marker, seconds: markerElapsedSeconds(marker.timestamp_us, origin) }];
      statusMessage = markerFeedback(marker, origin);
      return true;
    } catch (error) {
      statusMessage = `The marker was not added. ${formatCommandError(error)}`;
      return false;
    }
  }

  /**
   * Show in folder: File Explorer opens with the file selected. The backend
   * accepts only an existing file inside the current Project folder.
   */
  async function revealRecordingFile(path: string) {
    try {
      await invoke('reveal_recording_file', { path });
      statusMessage = 'File Explorer opened with the file selected.';
    } catch (error) {
      statusMessage = `The file could not be shown in its folder. ${formatCommandError(error)}`;
    }
  }

  /**
   * Clear connection (Faulted) or stop: the backend stops a running
   * recording, or clears an idle fault. Polling shows when saving ends.
   */
  async function disconnect() {
    try {
      session = await invoke<SessionStatus>('disconnect_session');
      statusMessage = recordingActive(session)
        ? 'Stopping and saving the recording…'
        : 'Connection cleared. Start a new recording when you are ready.';
    } catch (error) {
      statusMessage = formatCommandError(error);
    }
  }

  async function retryHandshake() {
    if (!canRetryHandshake || boardOperationBusy) return;
    const port = selectedPort;
    await runOperation(
      { title: 'Retrying connection…', stage: `Opening ${port} and repeating the WVU protocol handshake. The board is not reset and no firmware is uploaded.`, cancelable: false },
      async () => {
        try {
          const result = await invoke<HandshakeRetryResult>('retry_hardware_handshake', { port });
          await pollSession();
          // A handshake that found other firmware withdraws Ready (`verified_port`).
          await refreshFirmwareCompatibility();
          statusMessage = result.handshake_succeeded
            ? firmwareReadyOnSelectedPort
              ? `Connection confirmed on ${port}. Select Start recording for a new session.`
              : `Connection confirmed on ${port}. Select Verify Firmware before recording.`
            : `The connection was not confirmed on ${port}. ${result.diagnostics.recommended_action}`;
        } catch (error) {
          statusMessage = formatCommandError(error);
        }
      }
    );
  }

  onMount(() => {
    // The shell renders first. One application-level scan then populates the shared
    // cache; route changes consume that cache and never invoke Arduino CLI discovery.
    void (async () => {
      try {
        const settings = await invoke<{ project_folder: string; available?: boolean; notice?: string | null }>('get_project_folder');
        projectFolder = settings.project_folder;
        // A missing USB/network folder is reported, never recreated elsewhere.
        projectFolderNotice = settings.notice ?? '';
      } catch (error) {
        statusMessage = `Project folder needs attention: ${String(error)}`;
      }
      await prepareArduinoTools();
      if (arduinoToolsState === 'ready') {
        await refreshEnvironmentSummary();
        await refreshBoards('startup');
      }
    })();
    // The backend keeps the window open during a firmware restore (and while
    // a recording is saved on close) and says why.
    let stopCloseBlockedListener: (() => void) | undefined;
    let unmounted = false;
    listen<{ reason: string; message: string }>('close-blocked', (event) => {
      closeBlockedNotice = event.payload.message;
      statusMessage = event.payload.message;
    }).then((unlisten) => {
      if (unmounted) unlisten();
      else stopCloseBlockedListener = unlisten;
    }).catch(() => undefined);
    void pollSession();
    void refreshFirmwareCompatibility();
    void (async () => {
      await refreshProfiles();
      // Applies the initial lab (and loads its calibrations) explicitly.
      if (!isActive) selectProfile(listedProfileId(selectedProfileId), { announce: false });
    })();
    // 25 Hz while recording, a few reads a second while idle; no per-sample events.
    const timer = window.setInterval(pollSessionTick, 40);
    const firmwareTimer = window.setInterval(firmwareStatusTick, 750);
    // The sticky stack changes height with banners, wrapping, and the checklist.
    const stickyObserver = typeof ResizeObserver === 'function' ? new ResizeObserver(publishStickyHeight) : undefined;
    if (topStack) stickyObserver?.observe(topStack);
    publishStickyHeight();
    return () => {
      unmounted = true;
      stickyObserver?.disconnect();
      stopCloseBlockedListener?.();
      window.clearInterval(timer);
      window.clearInterval(firmwareTimer);
      if (infoTimer !== undefined) window.clearTimeout(infoTimer);
      if (startFeedbackTimer !== undefined) window.clearTimeout(startFeedbackTimer);
      if (storageEstimateTimer !== undefined) window.clearTimeout(storageEstimateTimer);
    };
  });
</script>

<svelte:head><title>WVU Bioinstrumentation Studio</title></svelte:head>

<div class="app-shell">
  <header class="app-header">
    <img src={logoUrl} alt="West Virginia University Chemical and Biomedical Engineering" />
    <h1>WVU Bioinstrumentation Studio</h1>
  </header>

  <!-- Always in view: the recording controls and the one message center. -->
  <div class="top-stack" bind:this={topStack}>
    <RecordingBar
      {chip}
      simulated={simulatedView}
      active={isActive}
      acquiring={session.state === 'Acquiring'}
      stopping={session.state === 'Stopping'}
      {progress}
      frames={formatCount(session.samples)}
      rate={formatRate(session.measured_rate_hz)}
      plannedDuration={plannedDurationText(durationMode, timedSeconds)}
      bind:source
      sourceLocked={setupLocked}
      hardwarePort={selectedPort}
      startDisabled={isActive || startInFlight || boardOperationBusy}
      stopDisabled={!isActive || session.state === 'Stopping'}
      caption={barCaption}
      checklist={barChecklist}
      {showClearConnection}
      clearDisabled={startInFlight}
      onStart={() => void startRecording()}
      onStop={() => void stopRecording()}
      onAddMarker={addMarker}
      onClearConnection={() => void disconnect()}
      onChecklistSelect={(target) => void focusReadinessTarget(target)}
      onSourceChange={sourceChosenByStudent}
    />
    <div class="message-slot" class:has-messages={Boolean(problem || statusMessage)}>
      <MessageCenter banner={problem} actionDisabled={problemActionDisabled} secondaryDisabled={secondaryActionDisabled} onAction={(action) => void runNextAction(action)} info={statusMessage} infoAction={shownInfoAction} />
    </div>
  </div>

  <main class="content wide-content">
    {#if liveFirst}
      {@render liveSignalsAndResult()}
    {:else}
      <p class="notice teaching-notice"><strong>Teaching use only — not a medical device.</strong> Follow BMEG 420L lab instructions and instructor safety procedures. Raw counts and Arduino-input volts are preserved; this app makes no diagnostic or clinical decision.</p>
    {/if}

    <section class="setup" aria-labelledby="setup-title">
      <div class="section-heading">
        <h2 id="setup-title">Setup</h2>
        {#if isActive}<p class="help">Setup is read-only while recording. Open a card to review it.</p>{/if}
      </div>

      <SetupCard id="board" title="Board" summary={boardCardSummary} bind:open={cardOpen.board} locked={isActive}>
        <BoardPanel
          {boards}
          bind:selectedPort
          status={boardPanelStatus}
          controls={boardControlState}
          toolsState={arduinoToolsState}
          toolsProblem={arduinoToolsProblem}
          toolsRepairable={arduinoToolsRepairable}
          toolsWarningShown={problem?.id !== 'tools:failed'}
          canRepairTools={canRepairArduinoTools}
          lastOperation={lastFirmwareOp}
          firmwareReady={firmwareReadyOnSelectedPort}
          onSelectChange={() => void selectedBoardChanged()}
          onRefresh={() => void refreshBoards()}
          onVerify={() => void verifySelectedFirmware()}
          onRestore={() => void restoreWvuFirmware()}
          onRepairTools={() => void repairArduinoTools()}
          onFeedback={(message) => statusMessage = message}
        />
      </SetupCard>

      <SetupCard id="project" title="Project folder" summary={projectCardSummary} bind:open={cardOpen.project} locked={isActive}>
        <div class="studio-form">
          <div class="control-grid"><label>Project folder<input value={projectFolder} readonly title={projectFolder} /></label><div class="field-action"><span>Location</span><button type="button" id="project-folder-browse" onclick={chooseProjectFolder} disabled={setupLocked}>Browse</button></div></div>
          {#if projectFolderNotice}<p class="warning">{projectFolderNotice}</p>{/if}
          <p class="help">Recordings are saved under this folder. Choose the Output folder in Session setup to create a trial subfolder.</p>
        </div>
      </SetupCard>

      <SetupCard id="lab" title="Lab" summary={labCardSummary} bind:open={cardOpen.lab} locked={isActive}>
        <LabCard
          bind:operatingMode
          bind:instructorAcknowledgement
          bind:benchNoticeAcknowledged
          bind:selectedProfileId
          profiles={acquisitionProfiles}
          profile={activeProfile}
          channels={activeChannels}
          pulseox={pulseoxProfile}
          {rateText}
          catalogNotice={catalogNoticeText}
          locked={setupLocked}
          {modeChangeInFlight}
          onSelectLab={(profileId) => selectProfile(profileId)}
          {onModeConfirmed}
          {onInstructorBlocked}
          onLabSaved={labRevisionSaved}
          onCatalogChanged={labCatalogChanged}
          onStatus={(message) => statusMessage = message}
        />
      </SetupCard>

      <SetupCard id="session" title="Session setup" summary={sessionCardSummary} bind:open={cardOpen.session} locked={isActive}>
        <SessionCard bind:outputDirectory bind:note profile={activeProfile} channels={activeChannels} pulseox={pulseoxProfile} {rateText} {effectiveOutputFolder} {outputFolderError} locked={setupLocked} />
      </SetupCard>

      <SetupCard id="duration" title="Recording duration" summary={durationCardSummary} bind:open={cardOpen.duration} locked={isActive}>
        <DurationCard
          bind:durationMode
          bind:durationPreset
          bind:customSeconds
          allowedModes={allowedDurationModes}
          presets={profileTimedPresets}
          {minimumCustomSeconds}
          {timedDurationValid}
          {storageEstimate}
          {storageShortfall}
          locked={setupLocked}
        />
      </SetupCard>

      <SetupCard id="calibration" title={'Calibration & Units'} summary={calibrationCardSummary} bind:open={cardOpen.calibration} locked={isActive}>
        <CalibrationCard
          bind:adcReferenceV
          bind:mpxvSensorSupplyV
          {adcReferenceError}
          {mpxvSupplyError}
          {appliedAdcReferenceV}
          {appliedMpxvSupplyV}
          bind:this={calibrationCard}
          {hasMpxvChannel}
          profileId={selectedProfileId}
          {calibratableChannels}
          bind:channelId={calibrationChannelId}
          bind:selectedCalibrationId={selectedXgzpCalibrationId}
          bind:activeCalibration={activeXgzpCalibration}
          {plotChannels}
          bind:channelUnits
          {channelUnitOptions}
          recordingCalibration={currentRecordingCalibration}
          category={activeProfile?.category}
          pulseox={plotPulseox}
          locked={setupLocked}
          onCalibrate={() => calibrationDialogOpen = true}
          onStatus={(message) => statusMessage = message}
        />
      </SetupCard>
    </section>

    {#if !liveFirst}{@render liveSignalsAndResult()}{/if}

    <AdvancedDetails
      {session}
      {firmwareCompatibility}
      {firmwareWorkflow}
      {firmwareEnvironment}
      {arduinoToolsState}
      {arduinoToolsDetail}
      {boardScanLastCompleted}
      {boardScanError}
      {lastStartFailure}
      {lastPlotError}
    />
  </main>
</div>

<!-- Live signals and, after a recording, its result card directly below. -->
{#snippet liveSignalsAndResult()}
  <LiveSignalsPanel
    pulseox={plotPulseox}
    teachingNotice={liveFirst}
    arduino={`${session.board || 'Not connected'}${session.port ? ` (${session.port})` : ''}`}
    storage={formatStorage(freeBytes)}
    digitalOutputs={plotProfile ? resolvedDigitalOutputs(plotProfile.acquisition) : []}
    outputMask={session.digital_output_mask}
    bind:paused={plotPaused}
    recording={isActive}
    bind:windowSeconds={plotTimeWindowSeconds}
    channels={plotChannels}
    bind:traceVisibility
    bind:plotGroups
    renderedGroups={renderedPlotGroups}
    visibleCount={visibleTraceIds.length}
    simulated={plotDataSimulated}
    {samples}
    {channelUnits}
    calibration={currentRecordingCalibration}
    adcBits={plotProfile?.acquisition.adc_resolution_bits ?? 12}
    {displayRevision}
    timeOriginUs={session.display_origin_timestamp_us ?? undefined}
    hardwareSource={isActive ? !session.simulator : source === 'hardware'}
    markers={plotMarkers}
    bind:displayOptionsOpen
    arrangementSummary={plotSummary}
    onDisplayRefresh={refreshDisplayWindow}
    onStatus={(message) => statusMessage = message}
    onPlotError={reportPlotError}
  />
  <!-- Hidden while the next recording runs, so the plots keep the screen. -->
  {#if lastSummary && resultView && !isActive}
    <RecordingResultCard
      view={resultView}
      stopReason={lastSummary.stop_reason}
      labReference={instructorModeActive && lastSummary.profile ? `${lastSummary.profile.profile.profile_id} ${lastSummary.profile.profile.profile_version}` : undefined}
      onReveal={(path) => void revealRecordingFile(path)}
      onFeedback={(message) => statusMessage = message}
    />
  {/if}
{/snippet}

<CalibrationDialog
  bind:open={calibrationDialogOpen}
  profile={activeProfile}
  channelId={calibrationChannelId}
  channelLabel={activeChannels.find((channel) => channel.id === calibrationChannelId)?.label ?? ''}
  recordingPath={lastRecordingPath}
  recordingSeconds={session.last_summary?.board_elapsed_seconds ?? 0}
  hardwareRecording={Boolean(session.last_summary) && !session.simulator}
  {adcReferenceError}
  {mpxvSupplyError}
  {appliedAdcReferenceV}
  {appliedMpxvSupplyV}
  locked={setupLocked}
  onSaved={(calibration) => calibrationCard?.applySaved(calibration)}
  onStatus={(message) => statusMessage = message}
/>

<!-- Busy operations and the firmware job. Not closable: Escape never cancels an upload. -->
<Modal open={Boolean(activeOperation || firmwareWorkflow.job?.active)} labelledBy="operation-title" describedBy="operation-stage" closable={false} size="narrow">
  {#if !activeOperation && firmwareWorkflow.job?.active}
    <!-- The restore stepper. From the upload onward the backend refuses Cancel: stopping a flash part-way could leave the board without firmware. -->
    <FirmwareJobProgress job={firmwareWorkflow.job} {closeBlockedNotice} onCancel={() => void cancelFirmwareJob()} />
  {:else}
    <div class="operation-modal">
      <div class="spinner" aria-hidden="true"></div>
      <div>
        <h2 id="operation-title">{activeOperation?.title ?? 'Working…'}</h2>
        <p id="operation-stage">{activeOperation?.stage ?? ''}</p>
        <p class="help">This operation has no reliable numeric percentage. The application remains responsive; conflicting actions are temporarily blocked.</p>
      </div>
    </div>
  {/if}
</Modal>

<style>
  /* WVU navy/gold palette and shared tokens; components read these too. */
  :global(:root) {
    --wvu-navy: #002855;
    --wvu-gold: #EEAA00;
    --ink: #17222e;
    --muted: #4b5965;
    --page-bg: #F7F7F7;
    --surface: #fff;
    --surface-muted: #f2f5f8;
    --border: #d7dde2;
    --border-strong: #8493a0;
    --danger: #9d2424;
    --danger-bg: #fdf0f0;
    --warn: #9b6700;
    --warn-bg: #fff8e8;
    --ok: #176c33;
    --ok-bg: #eaf6ee;
    /* Navy ring with a white gap: ≥3:1 against white, gold, and navy. */
    --focus-ring: #002855;
  }
  :global(*) { box-sizing: border-box; }
  /* Focused and scrolled-to controls stop below the sticky stack; --sticky-height is its measured height. */
  :global(html) { scroll-padding-top: calc(var(--sticky-height, 10rem) + .5rem); }
  :global(body) { margin: 0; min-width: 0; overflow-x: hidden; font-family: "Segoe UI", Arial, sans-serif; background: var(--page-bg); color: var(--ink); }
  :global(button), :global(input), :global(select), :global(textarea) { font: inherit; }
  :global(:focus-visible) { outline: 3px solid var(--focus-ring); outline-offset: 2px; box-shadow: 0 0 0 2px #fff; }
  @media (prefers-reduced-motion: reduce) {
    :global(*), :global(*::before), :global(*::after) { animation-duration: 0.01ms !important; animation-iteration-count: 1 !important; transition-duration: 0.01ms !important; scroll-behavior: auto !important; }
  }
  .app-shell { min-height: 100vh; min-width: 0; }
  .app-header { min-width: 0; display: flex; flex-wrap: wrap; align-items: center; gap: clamp(.65rem, 2vw, 1.2rem); padding: clamp(.4rem, 1.2vw, .75rem) clamp(1rem, 3vw, 2rem); background: var(--wvu-navy); color: var(--page-bg); }
  .app-header img { width: clamp(132px, 17vw, 174px); max-width: 100%; max-height: 56px; object-fit: contain; flex: 0 1 auto; }
  h1 { margin: 0; font-size: clamp(1.15rem, 2.2vw, 1.45rem); overflow-wrap: anywhere; }
  .top-stack { position: sticky; top: 0; z-index: 50; max-height: 60vh; overflow-y: auto; }
  /*
   * The stack is always on screen, but the page's scroll-padding would treat
   * its own controls as hidden under it and scroll the page up when one takes
   * focus (Tab, or focus moving from Stop to Start). This margin cancels that
   * padding for them; inside an overflowing stack they still scroll into view.
   */
  .top-stack :global(*) { scroll-margin-top: calc(-1 * var(--sticky-height, 0px) - 1rem); }
  .message-slot { background: var(--page-bg); }
  .message-slot.has-messages { padding: .3rem clamp(.6rem, 1.6vw, 1.1rem); border-bottom: 1px solid var(--border); }
  .content { min-width: 0; width: 100%; max-width: 112rem; margin: 0 auto; padding: clamp(.6rem, 1.6vw, 1rem) clamp(1rem, 3vw, 1.75rem); overflow-wrap: anywhere; }
  .teaching-notice { margin: 0 0 .5rem; padding: .45rem .75rem; border-left: 5px solid var(--wvu-gold); background: var(--warn-bg); font-size: .92rem; }
  .section-heading { display: flex; flex-wrap: wrap; align-items: baseline; gap: .25rem 1rem; }
  .section-heading h2 { margin: .6rem 0 0; }
  .section-heading .help, .operation-modal .help { margin: 0; color: var(--muted); }
  .operation-modal { display: flex; gap: 1rem; align-items: flex-start; } .operation-modal h2 { margin: 0; } .operation-modal p { margin: .4rem 0; overflow-wrap: anywhere; }
  .spinner { flex: 0 0 2rem; width: 2rem; height: 2rem; border: .3rem solid var(--border); border-top-color: var(--wvu-navy); border-radius: 50%; animation: spin .8s linear infinite; } @keyframes spin { to { transform: rotate(360deg); } }
  @media (max-width: 650px) { .app-header { align-items: flex-start; } .app-header img { max-width: 150px; } .content { padding: 1rem; } }
</style>
