import type { RecordingCalibration } from './calibration';
import type { RecordingDurationRequest } from './duration';

export type RecordingStartReadinessInput = {
  source: 'hardware' | 'simulator';
  selectedBoard: boolean;
  firmwareReady: boolean;
  sessionState: string;
  /**
   * A recording owns the session (`recordingActive`). False while Verify or
   * Retry connection passes through Connecting/Connected. Defaults to the state.
   */
  sessionActive?: boolean;
  /** Which operation raised a Faulted state (`SessionStatus.fault_origin`). */
  faultOrigin?: string | null;
  /**
   * A recording ran and ended (`SessionStatus.last_summary` is set). A
   * storage fault without one refused the recording before START.
   * Defaults to true.
   */
  recordingRan?: boolean;
  boardOperationBusy: boolean;
  startInFlight: boolean;
  activeProfile: boolean;
  projectFolder: boolean;
  outputFolderError?: string;
  durationValid: boolean;
  /** The lab's own reason when `durationValid` is false. */
  durationMessage?: string;
  /** An invalid ADC reference or MPXV supply field. */
  calibrationInputError?: string;
  acknowledgementSatisfied: boolean;
};

/**
 * Wire shape of the Rust `StartProfileSimulatorRequest`. Tauri does not
 * rename nested fields, so these are the serde (snake_case) names.
 * `profile_version` is the lab revision the page showed; Rust refuses Start
 * when the catalog's active revision differs.
 */
export type SimulatorStartRequest = {
  project_folder: string;
  output_folder: string;
  duration: RecordingDurationRequest;
  profile_id: string;
  profile_version: string;
  bench_notice_acknowledged: boolean;
  recording_note: string;
  calibration: RecordingCalibration | null;
};

/** Wire shape of the Rust `StartProfileHardwareRequest`. */
export type HardwareStartRequest = SimulatorStartRequest & { port: string };

export type RecordingStartReadiness = {
  canStart: boolean;
  message?: string;
};

export type RecordingStartFailure = {
  stage: string;
  code: string;
  userMessage: string;
  technicalDetail: string;
};

/**
 * The Tauri command deliberately receives one named `request` argument.  Keep
 * this wrapper beside the Start readiness logic so a future command-signature
 * edit cannot silently drift the IPC key again.
 */
export function hardwareStartInvokePayload<T>(request: T): { request: T } {
  return { request };
}

/** Keep the simulator command on the same one-argument Tauri IPC boundary. */
export function simulatorStartInvokePayload<T>(request: T): { request: T } {
  return { request };
}

const activeSessionStates = new Set([
  'Connecting',
  'Connected',
  'Configured',
  'Acquiring',
  'Stopping'
]);

/**
 * Gives the Start button one visible, student-facing reason when a new recording
 * cannot begin. Firmware recovery and acquisition state remain separate: a prior
 * idle fault must never be mistaken for a firmware-readiness failure.
 */
export function recordingStartReadiness(input: RecordingStartReadinessInput): RecordingStartReadiness {
  if (input.startInFlight || input.boardOperationBusy) {
    return {
      canStart: false,
      message: 'The Arduino is busy. Wait for the current operation to finish and try again.'
    };
  }
  if (activeSessionStates.has(input.sessionState)) {
    return {
      canStart: false,
      message: input.sessionActive ?? true
        ? 'A recording or connection is already active. Stop it before starting another recording.'
        : 'The board is being checked. Wait for the check to finish, then try Start.'
    };
  }
  // A simulator Start owns no board, and the backend clears the previous
  // fault when it begins; only a hardware Start waits for the fault to be
  // cleared (by Verify, or by Clear connection for a save or request fault).
  const blockingFault = input.sessionState === 'Faulted' && input.source === 'hardware';
  if (blockingFault && input.faultOrigin === 'storage') {
    // A save or folder problem is not a board problem; Verify Firmware would not help.
    return {
      canStart: false,
      message: input.recordingRan ?? true
        ? 'The previous recording was not saved completely. Select Clear connection, then start a new recording.'
        : 'The recording could not start because of the Project folder or its drive. Check it, select Clear connection, then start again.'
    };
  }
  if (blockingFault && input.faultOrigin === 'request') {
    // The request was refused before anything reached the board.
    return {
      canStart: false,
      message: 'The previous recording did not start. Select Clear connection, then start a new recording.'
    };
  }
  if (blockingFault) {
    return {
      canStart: false,
      message: 'The previous Arduino connection needs to be cleared. Verify the firmware again, then try Start.'
    };
  }
  if (!input.activeProfile) {
    return { canStart: false, message: 'Choose the assigned lab before recording.' };
  }
  if (!input.projectFolder) {
    return {
      canStart: false,
      message: 'Choose a writable Project folder before recording.'
    };
  }
  if (input.outputFolderError) {
    return { canStart: false, message: input.outputFolderError };
  }
  if (!input.durationValid) {
    return {
      canStart: false,
      message: input.durationMessage || 'Choose a valid timed duration of at least 10 seconds, or select Until stopped.'
    };
  }
  if (input.calibrationInputError) {
    // The recording would freeze a voltage other than the one in the field.
    return { canStart: false, message: input.calibrationInputError };
  }
  if (!input.acknowledgementSatisfied) {
    return {
      canStart: false,
      message: 'Confirm the required course notice before recording.'
    };
  }
  if (input.source === 'hardware' && !input.selectedBoard) {
    return {
      canStart: false,
      message: 'No Arduino is selected. Connect the UNO R4 WiFi or click Refresh Board.'
    };
  }
  if (input.source === 'hardware' && !input.firmwareReady) {
    return {
      canStart: false,
      message: 'WVU firmware is not ready. Verify or restore the firmware before recording.'
    };
  }
  return { canStart: true };
}

function objectStartFailure(error: unknown): RecordingStartFailure | undefined {
  let candidate: unknown = error;
  if (typeof error === 'string') {
    try { candidate = JSON.parse(error); } catch { return undefined; }
  }
  if (!candidate || typeof candidate !== 'object') return undefined;
  const value = candidate as Record<string, unknown>;
  if (typeof value.stage !== 'string' || typeof value.technicalDetail !== 'string') return undefined;
  return {
    stage: value.stage,
    code: typeof value.code === 'string' ? value.code : 'start_failed',
    userMessage: typeof value.userMessage === 'string'
      ? value.userMessage
      : 'Recording could not start. Try again. If the problem continues, open Advanced details and share the information with your instructor.',
    technicalDetail: value.technicalDetail
  };
}

export function recordingStartFailure(error: unknown): RecordingStartFailure {
  const structured = objectStartFailure(error);
  if (structured) return structured;
  const detail = String(error);
  const normalized = detail.toLowerCase();
  if (normalized.includes('destination') || normalized.includes('output folder') || normalized.includes('project folder')) {
    return { stage: 'VALIDATE_PATHS', code: 'recording_folder', userMessage: 'The recording folder is not writable. Choose another Project or Output folder before recording.', technicalDetail: detail };
  }
  if (normalized.includes('firmware compatibility') || normalized.includes('controlled wvu firmware')) {
    return { stage: 'CHECK_FIRMWARE', code: 'firmware_status', userMessage: 'WVU firmware is not ready. Verify or restore the firmware before recording.', technicalDetail: detail };
  }
  if (normalized.includes('one session') || normalized.includes('already running') || normalized.includes('serial') || normalized.includes('port busy')) {
    return { stage: 'SERIAL_OPEN', code: 'serial_busy', userMessage: 'The Arduino is busy or could not be opened. Wait for the current operation to finish, then try Refresh Board.', technicalDetail: detail };
  }
  if (normalized.includes('not a detected arduino') || normalized.includes('enumerated serial port')) {
    return { stage: 'BOARD_DISCOVERY', code: 'board_unavailable', userMessage: 'The selected Arduino is no longer available. Reconnect it, then click Refresh Board.', technicalDetail: detail };
  }
  return {
    stage: 'START_REQUEST',
    code: 'unknown',
    userMessage: 'Recording could not start. Try again. If the problem continues, open Advanced details and share the information with your instructor.',
    technicalDetail: detail
  };
}

export type StartFeedbackView = { tone: 'none' | 'status' | 'warning'; text: string };

function activeSessionText(state: string): string {
  if (state === 'Acquiring') return 'Recording. Press Stop recording to finish.';
  if (state === 'Connecting') return 'Connecting to the Arduino…';
  if (state === 'Stopping') return 'Stopping and saving the recording…';
  return 'Configuring recording…';
}

/**
 * The one line under Start. A running recording is a normal state, not a
 * warning; a readiness problem is shown only while no recording runs; and
 * the text of a Start attempt is shown only while it is still current (the
 * page clears it once Start returns).
 */
export function startFeedbackView(input: {
  sessionState: string;
  /** A recording owns the session; a board check does not (defaults to the state). */
  sessionActive?: boolean;
  startInFlight: boolean;
  readiness: RecordingStartReadiness;
  feedback: string;
}): StartFeedbackView {
  if (input.startInFlight) return { tone: 'status', text: input.feedback || 'Starting recording…' };
  if (activeSessionStates.has(input.sessionState) && (input.sessionActive ?? true)) {
    return { tone: 'status', text: activeSessionText(input.sessionState) };
  }
  if (!input.readiness.canStart) {
    return { tone: 'warning', text: input.readiness.message ?? 'Recording cannot start yet.' };
  }
  if (input.feedback) return { tone: 'status', text: input.feedback };
  return { tone: 'none', text: '' };
}
