export type RecordingStartReadinessInput = {
  source: 'hardware' | 'simulator';
  selectedBoard: boolean;
  firmwareReady: boolean;
  sessionState: string;
  boardOperationBusy: boolean;
  startInFlight: boolean;
  activeProfile: boolean;
  projectFolder: boolean;
  outputFolderError?: string;
  durationValid: boolean;
  acknowledgementSatisfied: boolean;
};

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
      message: 'A recording or connection is already active. Stop it before starting another recording.'
    };
  }
  if (input.sessionState === 'Faulted') {
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
      message: 'Choose a valid timed duration of at least 10 seconds, or select Until stopped.'
    };
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

export function studentRecordingStartError(error: unknown): string {
  return recordingStartFailure(error).userMessage;
}
