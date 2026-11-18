/**
 * The one place that turns backend codes into student-facing text.
 *
 * Every `ConnectionFailureCategory`, `FaultOrigin`, and
 * `FirmwareErrorCategory` the Rust backend can report maps to a short
 * headline (what happened), one sentence (what to do), and at most one next
 * action button. The raw codes stay in Advanced details; students never see
 * them. `messages.test.ts` reads the Rust enums, so a new backend category
 * without text here fails the tests.
 */

/** The single button the message center may offer. */
export type NextAction =
  | 'retry_connection'
  | 'verify_firmware'
  | 'restore_firmware'
  | 'clear_connection'
  | 'choose_folder'
  | 'repair_arduino_tools'
  | 'refresh_board';

export const NEXT_ACTION_LABELS: Record<NextAction, string> = {
  retry_connection: 'Retry connection',
  verify_firmware: 'Verify Firmware',
  restore_firmware: 'Restore WVU Firmware',
  clear_connection: 'Clear connection',
  choose_folder: 'Choose another folder',
  repair_arduino_tools: 'Repair Arduino tools',
  refresh_board: 'Refresh Board'
};

export type PlainMessage = {
  headline: string;
  sentence: string;
  nextAction?: NextAction;
};

/** Rust `session::ConnectionFailureCategory` (snake_case). */
export type ConnectionFailureCategory =
  | 'port_busy'
  | 'port_open_no_bytes'
  | 'non_protocol_bytes'
  | 'protocol_crc_failure'
  | 'wrong_protocol_version'
  | 'missing_firmware_identity'
  | 'incompatible_firmware_identity'
  | 'handshake_incomplete'
  | 'serial_transport_error'
  | 'firmware_protocol_error'
  | 'no_data_timeout'
  | 'device_disconnected'
  | 'recording_storage_error'
  | 'lab_not_supported_by_firmware'
  | 'configuration_rejected'
  | 'start_not_confirmed';

/** Rust `session::FaultOrigin` (snake_case). */
export type FaultOrigin = 'verification' | 'connection' | 'recording' | 'storage' | 'request';

/** Rust `firmware_workflow::FirmwareErrorCategory` (snake_case). */
export type FirmwareErrorCategory =
  | 'arduino_cli_missing'
  | 'core_missing'
  | 'confirmation_required'
  | 'compile_failed'
  | 'port_busy'
  | 'board_not_found'
  | 'ambiguous_board'
  | 'upload_failed'
  | 'application_port_not_found'
  | 'manual_reset_required'
  | 'protocol_verification_failed'
  | 'wrong_firmware_identity'
  | 'canceled'
  | 'upload_cannot_be_canceled'
  | 'tool_timed_out'
  | 'internal_error';

const CLOSE_OTHER_APPS = 'Close Arduino IDE, Serial Monitor, or Arduino Cloud Agent';

export const CONNECTION_FAILURE_MESSAGES: Record<ConnectionFailureCategory, PlainMessage> = {
  port_busy: {
    headline: 'Another program is using the board',
    sentence: `${CLOSE_OTHER_APPS}, then retry the connection.`,
    nextAction: 'retry_connection'
  },
  port_open_no_bytes: {
    headline: 'The board did not answer',
    sentence: 'Retry the connection. If it still does not answer, verify the firmware.',
    nextAction: 'retry_connection'
  },
  non_protocol_bytes: {
    headline: 'The board is not running the WVU firmware',
    sentence: 'Check that the right board is selected, then restore the WVU firmware.',
    nextAction: 'restore_firmware'
  },
  protocol_crc_failure: {
    headline: 'Data from the board was damaged',
    sentence: 'Check the USB cable, then retry the connection.',
    nextAction: 'retry_connection'
  },
  wrong_protocol_version: {
    headline: 'The board has a different WVU firmware version',
    sentence: 'Restore the WVU firmware to update it.',
    nextAction: 'restore_firmware'
  },
  missing_firmware_identity: {
    headline: 'The board did not identify its firmware',
    sentence: 'Restore the WVU firmware before recording.',
    nextAction: 'restore_firmware'
  },
  incompatible_firmware_identity: {
    headline: 'The board has a different WVU firmware version',
    sentence: 'Restore the WVU firmware to update it.',
    nextAction: 'restore_firmware'
  },
  handshake_incomplete: {
    headline: 'The board did not finish connecting',
    sentence: 'Retry the connection. If it fails again, verify or restore the firmware.',
    nextAction: 'retry_connection'
  },
  serial_transport_error: {
    headline: 'The USB connection reported an error',
    sentence: 'Check the USB cable, then verify the firmware to reconnect.',
    nextAction: 'verify_firmware'
  },
  firmware_protocol_error: {
    headline: 'The Arduino stopped the recording',
    sentence: 'This often happens when the computer pauses or sleeps. Verify the firmware, then start a new recording.',
    nextAction: 'verify_firmware'
  },
  no_data_timeout: {
    headline: 'The Arduino stopped sending samples',
    sentence: 'Check the USB cable, verify the firmware, then start a new recording.',
    nextAction: 'verify_firmware'
  },
  device_disconnected: {
    headline: 'The board was disconnected',
    sentence: 'Reconnect the UNO R4 WiFi, then refresh the board list.',
    nextAction: 'refresh_board'
  },
  recording_storage_error: {
    headline: 'The recording could not be saved completely',
    sentence: 'Check that the Project folder’s drive is connected and has free space (or choose another folder), then clear the connection. Files already saved were kept.',
    nextAction: 'clear_connection'
  },
  lab_not_supported_by_firmware: {
    headline: 'The board’s firmware cannot run this lab',
    sentence: 'Restore the WVU firmware, or ask your instructor to check the lab settings.',
    nextAction: 'restore_firmware'
  },
  configuration_rejected: {
    headline: 'The board did not accept the lab settings',
    sentence: 'Retry the connection, then verify the firmware before starting again.',
    nextAction: 'retry_connection'
  },
  start_not_confirmed: {
    headline: 'The board did not confirm the start',
    sentence: 'Retry the connection, then verify the firmware before starting again.',
    nextAction: 'retry_connection'
  }
};

export const FAULT_ORIGIN_MESSAGES: Record<FaultOrigin, PlainMessage> = {
  verification: {
    headline: 'The firmware could not be verified',
    sentence: 'Check the USB connection, then verify the firmware again.',
    nextAction: 'verify_firmware'
  },
  connection: {
    headline: 'The Arduino did not finish connecting',
    sentence: 'Retry the connection. If it fails again, verify the firmware.',
    nextAction: 'retry_connection'
  },
  recording: {
    headline: 'The recording stopped because of an error',
    sentence: 'Verify the firmware, then start a new recording. Open Advanced details if it happens again.',
    nextAction: 'verify_firmware'
  },
  storage: {
    headline: 'The recording was not saved completely',
    sentence: 'Check that the Project folder’s drive is connected and has free space (or choose another folder), then clear the connection. Files already saved were kept.',
    nextAction: 'clear_connection'
  },
  request: {
    headline: 'The recording did not start',
    sentence: 'Clear the connection, check the lab settings, then start again.',
    nextAction: 'clear_connection'
  }
};

export const FIRMWARE_FAILURE_MESSAGES: Record<FirmwareErrorCategory, PlainMessage> = {
  arduino_cli_missing: {
    headline: 'The Arduino tools are unavailable',
    sentence: 'Repair the Arduino tools. If that does not help, reinstall the application or contact your instructor.',
    nextAction: 'repair_arduino_tools'
  },
  core_missing: {
    headline: 'The Arduino tools are incomplete',
    sentence: 'Repair the Arduino tools. If that does not help, reinstall the application or contact your instructor.',
    nextAction: 'repair_arduino_tools'
  },
  confirmation_required: {
    headline: 'Confirmation required',
    sentence: 'Confirm that you want to restore the WVU firmware.',
    nextAction: 'restore_firmware'
  },
  compile_failed: {
    headline: 'The WVU firmware could not be prepared',
    sentence: 'Repair the Arduino tools and try again. If it fails again, contact your instructor.',
    nextAction: 'repair_arduino_tools'
  },
  port_busy: {
    headline: 'Another program is using the board',
    sentence: `${CLOSE_OTHER_APPS}, then verify the firmware again.`,
    nextAction: 'verify_firmware'
  },
  board_not_found: {
    headline: 'The board was not found',
    sentence: 'Reconnect the UNO R4 WiFi, then refresh the board list.',
    nextAction: 'refresh_board'
  },
  ambiguous_board: {
    headline: 'More than one board is connected',
    sentence: 'Unplug the other Arduino boards, then refresh the board list.',
    nextAction: 'refresh_board'
  },
  upload_failed: {
    headline: 'The firmware upload failed',
    sentence: `${CLOSE_OTHER_APPS}, press the board’s RESET button twice quickly, then restore the WVU firmware again.`,
    nextAction: 'restore_firmware'
  },
  application_port_not_found: {
    headline: 'The board did not come back after the upload',
    sentence: 'Refresh the board list (the COM number may have changed), then verify the firmware.',
    nextAction: 'refresh_board'
  },
  manual_reset_required: {
    headline: 'The board needs a double reset',
    sentence: 'Restore the WVU firmware again and press the board’s RESET button twice quickly when asked.',
    nextAction: 'restore_firmware'
  },
  protocol_verification_failed: {
    headline: 'The firmware could not be verified',
    sentence: 'Restore the WVU firmware before recording.',
    nextAction: 'restore_firmware'
  },
  wrong_firmware_identity: {
    headline: 'The board needs the WVU firmware',
    sentence: 'Restore the WVU firmware before recording.',
    nextAction: 'restore_firmware'
  },
  canceled: {
    headline: 'The firmware restore was canceled',
    sentence: 'Reconnect the board if needed, then restore the WVU firmware again.',
    nextAction: 'restore_firmware'
  },
  upload_cannot_be_canceled: {
    headline: 'The board is being written',
    sentence: 'Wait for the restore to finish. Do not unplug the board.'
  },
  tool_timed_out: {
    headline: 'The Arduino tools stopped responding',
    sentence: 'Unplug the board and plug it back in, refresh the board list, then try again.',
    nextAction: 'refresh_board'
  },
  internal_error: {
    headline: 'The firmware operation failed',
    sentence: 'Open Advanced details, copy the information, and contact your instructor.'
  }
};

function lookup<T extends string>(table: Record<T, PlainMessage>, key: string | null | undefined): PlainMessage | undefined {
  return key && Object.prototype.hasOwnProperty.call(table, key) ? table[key as T] : undefined;
}

export function connectionFailureMessage(category: string | null | undefined): PlainMessage | undefined {
  return lookup(CONNECTION_FAILURE_MESSAGES, category);
}

export function faultOriginMessage(origin: string | null | undefined): PlainMessage | undefined {
  return lookup(FAULT_ORIGIN_MESSAGES, origin);
}

export function firmwareFailureMessage(category: string | null | undefined): PlainMessage | undefined {
  return lookup(FIRMWARE_FAILURE_MESSAGES, category);
}

/** Categories that only a firmware restore (or the instructor) can fix. */
const RESTORE_CATEGORIES = new Set<string>([
  'non_protocol_bytes',
  'wrong_protocol_version',
  'missing_firmware_identity',
  'incompatible_firmware_identity',
  'lab_not_supported_by_firmware'
]);

export type ProblemSeverity = 'error' | 'warning';

/** The one problem banner the message center shows. */
export type ProblemBanner = {
  /** Stable while the same problem persists; a new id is announced once. */
  id: string;
  severity: ProblemSeverity;
  headline: string;
  sentence: string;
  nextAction?: NextAction;
  /** A quieter second button, for example "Verify again" beside Restore. */
  secondary?: { action: NextAction; label: string };
};

export type FaultInput = {
  faultOrigin?: string | null;
  failureCategory?: string | null;
  /** `SessionStatus.last_error`: student text for a refused request. */
  lastError?: string | null;
  /** The faulted session ran the simulator (`SessionStatus.simulator`). */
  simulatorSession: boolean;
  /** The Source chosen for the next recording. */
  source: 'hardware' | 'simulator';
  /** The selected COM port, named in board headlines. */
  port?: string;
};

/**
 * What to do for a category whose own step is Retry connection, when the
 * banner offers Verify Firmware instead: the fault came from Verify Firmware
 * or a recording, or Source is Simulator. The sentence then names the
 * button the banner shows.
 */
export const VERIFY_INSTEAD_OF_RETRY: Partial<Record<ConnectionFailureCategory, string>> = {
  port_busy: FIRMWARE_FAILURE_MESSAGES.port_busy.sentence,
  port_open_no_bytes: 'Verify the firmware. If the board still does not answer, restore the WVU firmware.',
  protocol_crc_failure: 'Check the USB cable, then verify the firmware.',
  handshake_incomplete: 'Verify the firmware. If it fails again, restore the WVU firmware.',
  configuration_rejected: 'Verify the firmware before starting again.',
  start_not_confirmed: 'Verify the firmware before starting again.'
};

/**
 * Verify Firmware opened the board, but it did not answer as the WVU
 * firmware. `firmware_workflow` records every such result as
 * protocol_verification_failed, which the Board card shows as "No WVU
 * firmware". A port that could not be opened (busy or unplugged) was not
 * examined, and another WVU firmware version keeps its own, more specific
 * text; both offer the same step as the Board card.
 */
function verifyFoundNoWvuFirmware(category: string): boolean {
  return category !== '' && category !== 'port_busy' && category !== 'device_disconnected' && !RESTORE_CATEGORIES.has(category);
}

/** Firmware failures meaning "this board is not running the WVU firmware" (as `boardStatus`). */
export const NO_FIRMWARE_FAILURES = new Set<string>(['protocol_verification_failed', 'wrong_firmware_identity']);

/** The Board card's "No WVU firmware" state uses the same headline and actions. */
function noWvuFirmware(id: string, port: string | undefined, severity: ProblemSeverity): ProblemBanner {
  return {
    id,
    severity,
    headline: `No WVU firmware detected${port ? ` on ${port}` : ''}`,
    sentence: 'The board did not answer as the WVU firmware. Restore the WVU firmware to record from it. If another program was using the board, close it and verify again.',
    nextAction: 'restore_firmware',
    secondary: { action: 'verify_firmware', label: 'Verify again' }
  };
}

/**
 * Whether a Faulted session matters for the chosen Source. A failed board
 * check (Verify Firmware or Retry connection) says nothing about Simulator
 * practice, which owns no board and clears the fault when it starts, so the
 * Board card reports it instead of the banner. A recording, save, or
 * request fault always matters: it describes the student's own recording.
 */
export function faultConcernsSource(fault: FaultInput): boolean {
  if (fault.source === 'hardware' || fault.simulatorSession) return true;
  return fault.faultOrigin !== 'verification' && fault.faultOrigin !== 'connection';
}

/**
 * The banner for a Faulted session. The next action depends on what can
 * clear the fault: Clear connection for a save problem, a refused request,
 * or a simulator run; Restore for the wrong firmware; Refresh Board for an
 * unplugged board; otherwise the step that repeats the failed operation
 * (Verify Firmware clears a recording fault on success).
 */
export function faultBanner(input: FaultInput): ProblemBanner {
  const origin = input.faultOrigin ?? '';
  const category = input.failureCategory ?? '';
  const byCategory = connectionFailureMessage(category);
  const byOrigin = faultOriginMessage(origin);
  const id = `fault:${origin || 'unknown'}:${category || 'none'}`;

  if (origin === 'request') {
    const request = FAULT_ORIGIN_MESSAGES.request;
    const detail = input.lastError?.trim();
    return { id, severity: 'error', headline: request.headline, sentence: detail ? `${sentenceOf(detail)} ${request.sentence}` : request.sentence, nextAction: 'clear_connection' };
  }
  if (origin === 'storage' || category === 'recording_storage_error') {
    const storage = FAULT_ORIGIN_MESSAGES.storage;
    return { id, severity: 'error', headline: storage.headline, sentence: storage.sentence, nextAction: 'clear_connection' };
  }
  if (input.simulatorSession) {
    return {
      id,
      severity: 'error',
      headline: origin === 'connection' ? 'The simulator recording could not start' : 'The simulator recording stopped because of an error',
      sentence: 'Clear the connection, then start again. Open Advanced details if it happens again.',
      nextAction: 'clear_connection'
    };
  }
  // Verify Firmware opened the board but it did not answer as the WVU
  // firmware. The Board card says "No WVU firmware" too.
  if (origin === 'verification' && verifyFoundNoWvuFirmware(category)) return noWvuFirmware(id, input.port, 'error');
  const message = byCategory ?? byOrigin ?? {
    headline: 'The Arduino connection needs attention',
    sentence: 'Verify the firmware before starting a new recording. Open Advanced details if it happens again.',
    nextAction: 'verify_firmware' as const
  };
  let nextAction: NextAction | undefined;
  if (RESTORE_CATEGORIES.has(category)) nextAction = 'restore_firmware';
  else if (category === 'device_disconnected') nextAction = 'refresh_board';
  else if (origin === 'verification') nextAction = 'verify_firmware';
  else if (origin === 'connection') nextAction = input.source === 'hardware' ? 'retry_connection' : 'verify_firmware';
  else if (origin === 'recording') nextAction = 'verify_firmware';
  else nextAction = message.nextAction;
  // Retry connection needs Source = Hardware (see `connectionActions`).
  if (nextAction === 'retry_connection' && input.source !== 'hardware') nextAction = 'verify_firmware';
  // The sentence names the button shown, never a step the banner does not offer.
  const verifySentence = Object.prototype.hasOwnProperty.call(VERIFY_INSTEAD_OF_RETRY, category)
    ? VERIFY_INSTEAD_OF_RETRY[category as ConnectionFailureCategory]
    : undefined;
  const sentence = nextAction === 'verify_firmware' && message.nextAction === 'retry_connection'
    ? verifySentence ?? 'Verify the firmware before starting a new recording.'
    : message.sentence;
  return { id, severity: 'error', headline: message.headline, sentence, nextAction };
}

function sentenceOf(text: string): string {
  const trimmed = text.trim();
  return /[.!?…]$/.test(trimmed) ? trimmed : `${trimmed}.`;
}

export type ProblemInput = {
  sessionState: string;
  active: boolean;
  fault: FaultInput;
  /** `SessionStatus.storage_warning` while a recording runs. */
  storageWarning?: string | null;
  arduinoTools: { state: 'preparing' | 'ready' | 'failed'; problem: string; repairable: boolean };
  projectFolderNotice?: string;
  board: {
    source: 'hardware' | 'simulator';
    selectedPort: string;
    firmwareReady: boolean;
    compatibility: string;
    /** `FirmwareWorkflowStatus.last_failure.category`, if any. */
    lastFailureCategory?: string | null;
    firmwareJobActive: boolean;
    /** A board scan or Verify Firmware is running; its result is not known yet. */
    checking?: boolean;
  };
  /** Pre-Start projection: a timed run will not fit. */
  storageShortfall?: string;
  catalogNotice?: string;
};

/**
 * Picks the single most important problem for the current action and
 * Source, or none. Order: a Faulted session, low storage during a
 * recording, broken Arduino tools, an unavailable Project folder, a board
 * that cannot record, too little space for the planned recording, then
 * lab-catalog problems.
 *
 * The banner is for problems that block or affect what the student is
 * doing. While Source is Simulator, board and Arduino-tools problems (which
 * Simulator recordings do not need) stay in the Board card instead.
 */
export function problemBanner(input: ProblemInput): ProblemBanner | undefined {
  if (input.sessionState === 'Faulted' && faultConcernsSource(input.fault)) return faultBanner(input.fault);
  if (input.active && input.storageWarning?.trim()) {
    return {
      id: 'storage:low',
      severity: 'warning',
      headline: 'Storage is running low',
      sentence: sentenceOf(input.storageWarning)
    };
  }
  if (input.arduinoTools.state === 'failed' && input.board.source === 'hardware' && !input.active) {
    return {
      id: 'tools:failed',
      severity: 'error',
      headline: 'The Arduino tools need attention',
      sentence: sentenceOf(input.arduinoTools.problem || 'The Arduino tools could not be prepared.'),
      nextAction: input.arduinoTools.repairable ? 'repair_arduino_tools' : undefined
    };
  }
  if (!input.active && input.projectFolderNotice?.trim()) {
    return {
      id: 'folder:unavailable',
      severity: 'warning',
      headline: 'The Project folder is not available',
      sentence: sentenceOf(input.projectFolderNotice),
      nextAction: 'choose_folder'
    };
  }
  if (!input.active) {
    const board = boardProblem(input.board);
    if (board) return board;
  }
  if (!input.active && input.storageShortfall?.trim()) {
    return {
      id: 'storage:shortfall',
      severity: 'warning',
      headline: 'Not enough free space for this recording',
      sentence: sentenceOf(input.storageShortfall),
      nextAction: 'choose_folder'
    };
  }
  if (input.catalogNotice?.trim()) {
    return {
      id: 'catalog:problem',
      severity: 'warning',
      headline: 'Some labs could not be loaded',
      sentence: sentenceOf(input.catalogNotice)
    };
  }
  return undefined;
}

/**
 * Why the selected board cannot record, while Source = Hardware. (In
 * Simulator practice the Board card shows the board's state instead.)
 * Nothing is said while a scan or Verify Firmware is still finding out.
 * Recovery (Verify, Restore, Refresh) is offered regardless of any other
 * readiness.
 */
function boardProblem(board: ProblemInput['board']): ProblemBanner | undefined {
  if (board.source !== 'hardware' || board.checking || board.firmwareJobActive || board.firmwareReady) return undefined;
  if (!board.selectedPort) {
    return {
      id: 'board:none',
      severity: 'warning',
      headline: 'No Arduino is selected',
      sentence: 'Connect the UNO R4 WiFi with a USB cable, then refresh the board list.',
      nextAction: 'refresh_board'
    };
  }
  const port = board.selectedPort;
  if (board.compatibility === 'upload_in_progress') return undefined;
  const category = board.lastFailureCategory ?? '';
  // The same states, in the same order, as the Board card (`boardStatus`):
  // a busy port first, then "No WVU firmware", then the last failure.
  const noFirmware = board.compatibility === 'wvu_protocol_incompatible'
    || (board.compatibility === 'verification_failed' && (!category || NO_FIRMWARE_FAILURES.has(category)));
  if (noFirmware && category !== 'port_busy') return noWvuFirmware(`board:no-firmware:${port}`, port, 'warning');
  const lastFailure = board.compatibility !== 'wvu_protocol_compatible' || category === 'port_busy'
    ? firmwareFailureMessage(board.lastFailureCategory)
    : undefined;
  if (lastFailure) {
    return {
      id: `board:failure:${board.lastFailureCategory}:${port}`,
      severity: 'warning',
      headline: lastFailure.headline,
      sentence: lastFailure.sentence,
      nextAction: lastFailure.nextAction
    };
  }
  if (board.compatibility === 'verification_failed') {
    return {
      id: `board:verification-failed:${port}`,
      severity: 'warning',
      headline: `The firmware on ${port} could not be verified`,
      sentence: `${CLOSE_OTHER_APPS}, then verify the firmware again. If it still fails, restore the WVU firmware.`,
      nextAction: 'verify_firmware'
    };
  }
  return {
    id: `board:unchecked:${port}`,
    severity: 'warning',
    headline: `The firmware on ${port} has not been checked`,
    sentence: 'Verify the firmware before recording from this board.',
    nextAction: 'verify_firmware'
  };
}
