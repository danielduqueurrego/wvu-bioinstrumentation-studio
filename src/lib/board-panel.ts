/**
 * What the Board panel shows: one state chip (text, never colour alone),
 * the single action that fits that state, and the persistent "Last firmware
 * operation" card built from `last_restore` / `last_failure`.
 *
 * Recovery never depends on readiness: Verify and Restore stay reachable as
 * secondary buttons whatever the chip says (see `boardControls`, which does
 * not read the firmware status at all). This module only picks which action
 * is the prominent one.
 */
import { firmwareFailureMessage, NO_FIRMWARE_FAILURES, type NextAction } from './messages';
import type { FirmwareFailure, FirmwareJob, FirmwareWorkflowStatus } from './session-types';

export type BoardState =
  | 'none'
  | 'checking'
  | 'restoring'
  | 'ready'
  | 'busy'
  | 'no_firmware'
  | 'needs_attention'
  | 'not_checked';

/** The prominent board action. Verify and Restore also stay available as secondary buttons. */
export type BoardAction = 'refresh' | 'verify' | 'verify_after_close' | 'restore' | 'repair_tools';

export const BOARD_ACTION_LABELS: Record<BoardAction, string> = {
  refresh: 'Refresh Board',
  verify: 'Verify Firmware',
  verify_after_close: 'Close other serial apps, then Verify',
  restore: 'Restore WVU Firmware',
  repair_tools: 'Repair Arduino tools'
};

export type BoardChipTone = 'ready' | 'idle' | 'busy' | 'problem';

export type BoardStatus = {
  state: BoardState;
  label: string;
  tone: BoardChipTone;
  /** One plain sentence under the row. */
  hint: string;
  action?: BoardAction;
};

export type BoardStatusInput = {
  selectedPort: string;
  /** `FirmwareWorkflowStatus.compatibility`. */
  compatibility: string;
  /** Compatible firmware proven on the selected port (`verified_port`). */
  firmwareReady: boolean;
  /** A board scan or a Verify Firmware handshake is running. */
  checking: boolean;
  firmwareJobActive: boolean;
  /** `FirmwareWorkflowStatus.last_failure.category`. */
  lastFailureCategory?: string | null;
};

const CLOSE_OTHER_APPS = 'Close Arduino IDE, Serial Monitor, or Arduino Cloud Agent';


const ACTION_FOR_NEXT: Partial<Record<NextAction, BoardAction>> = {
  restore_firmware: 'restore',
  verify_firmware: 'verify',
  refresh_board: 'refresh',
  repair_arduino_tools: 'repair_tools'
};

export function boardStatus(input: BoardStatusInput): BoardStatus {
  const port = input.selectedPort;
  if (input.firmwareJobActive || input.compatibility === 'upload_in_progress') {
    return { state: 'restoring', label: 'Restoring firmware…', tone: 'busy', hint: 'The WVU firmware is being restored. Follow the steps in the restore window.' };
  }
  if (input.checking) {
    return { state: 'checking', label: 'Checking…', tone: 'busy', hint: 'Looking for the board and checking its firmware. Nothing is uploaded.' };
  }
  if (!port) {
    return {
      state: 'none',
      label: 'No board selected',
      tone: 'idle',
      hint: 'Connect the UNO R4 WiFi with a USB cable, then select Refresh Board.',
      action: 'refresh'
    };
  }
  if (input.firmwareReady) {
    return { state: 'ready', label: `Ready on ${port}`, tone: 'ready', hint: 'The WVU firmware was verified on this board. Hardware recording is available.' };
  }
  const category = input.lastFailureCategory ?? '';
  if (category === 'port_busy') {
    return {
      state: 'busy',
      label: 'Port busy',
      tone: 'problem',
      hint: `Another program is using ${port}. ${CLOSE_OTHER_APPS}, then verify again.`,
      action: 'verify_after_close'
    };
  }
  if (
    input.compatibility === 'wvu_protocol_incompatible'
    || (input.compatibility === 'verification_failed' && (!category || NO_FIRMWARE_FAILURES.has(category)))
  ) {
    return {
      state: 'no_firmware',
      label: 'No WVU firmware',
      tone: 'problem',
      hint: `The board on ${port} did not answer as the WVU firmware. Restore the WVU firmware to record from it.`,
      action: 'restore'
    };
  }
  if (input.compatibility === 'verification_failed') {
    const message = firmwareFailureMessage(category);
    return {
      state: 'needs_attention',
      label: 'Needs attention',
      tone: 'problem',
      hint: message ? `${message.headline}. ${message.sentence}` : 'The last firmware operation did not finish. Restore the WVU firmware again.',
      action: (message?.nextAction && ACTION_FOR_NEXT[message.nextAction]) || 'restore'
    };
  }
  return {
    state: 'not_checked',
    label: 'Not checked',
    tone: 'idle',
    hint: `Verify the firmware on ${port} before recording. Verifying only reads from the board; it never uploads.`,
    action: 'verify'
  };
}

export type LastFirmwareOperation = {
  kind: 'restore' | 'verify';
  outcome: 'success' | 'failure';
  title: string;
  explanation: string;
  recommendedAction?: string;
  originalPort?: string;
  finalPort?: string;
  logPath?: string;
  /** ISO time the operation finished (restores only). */
  finishedAt?: string;
  /** Plain text for Copy diagnostics, including the raw codes. */
  diagnostics: string;
};

function sameFailure(a: FirmwareFailure | null | undefined, b: FirmwareFailure | null | undefined): boolean {
  return Boolean(a && b)
    && a!.category === b!.category
    && a!.stage === b!.stage
    && a!.explanation === b!.explanation
    && a!.technical_details === b!.technical_details;
}

function failureText(failure: FirmwareFailure) {
  const plain = firmwareFailureMessage(failure.category);
  return {
    title: plain?.headline ?? failure.title,
    explanation: failure.explanation,
    recommendedAction: plain?.sentence ?? failure.recommended_action
  };
}

function line(label: string, value: string | number | null | undefined): string | undefined {
  return value === undefined || value === null || value === '' ? undefined : `${label}: ${value}`;
}

function diagnosticsText(kind: 'restore' | 'verify', job: FirmwareJob | undefined, failure: FirmwareFailure | undefined, outcome: string): string {
  return [
    'WVU Bioinstrumentation Studio: last firmware operation',
    line('Operation', kind === 'restore' ? `Restore WVU Firmware${job ? ` (job ${job.id})` : ''}` : 'Verify Firmware'),
    line('Result', outcome),
    line('Stage', failure?.stage ?? job?.stage),
    line('Category', failure?.category),
    line('Title', failure?.title),
    line('Explanation', failure?.explanation),
    line('Recommended action', failure?.recommended_action),
    line('Technical details', failure?.technical_details),
    line('Original port', job?.original_port),
    line('Upload port', job?.bootloader_port),
    line('Final port', job?.final_port),
    line('Started', job?.started_utc),
    line('Finished', job?.completed_utc),
    line('Firmware source hash', job?.source_hash),
    line('Log', job?.log_path),
    line('Backend message', job?.message)
  ].filter((entry): entry is string => entry !== undefined).join('\n');
}

/**
 * The most recent firmware operation. `last_failure` is cleared when a
 * restore begins and by a successful Verify, so a failure that differs
 * from the last restore's own failure is a later Verify.
 */
export function lastFirmwareOperation(workflow: Pick<FirmwareWorkflowStatus, 'last_restore' | 'last_failure'>): LastFirmwareOperation | undefined {
  const restore = workflow.last_restore ?? undefined;
  const failure = workflow.last_failure ?? undefined;
  const verifyFailureIsNewer = failure && (!restore || restore.stage === 'complete' || !sameFailure(restore.failure, failure));
  if (failure && verifyFailureIsNewer) {
    return {
      kind: 'verify',
      outcome: 'failure',
      ...failureText(failure),
      diagnostics: diagnosticsText('verify', undefined, failure, 'failed')
    };
  }
  if (!restore || restore.active) return undefined;
  const ports = {
    originalPort: restore.original_port ?? undefined,
    finalPort: restore.final_port ?? undefined,
    logPath: restore.log_path ?? undefined,
    finishedAt: restore.completed_utc ?? undefined
  };
  if (restore.stage === 'complete') {
    const moved = restore.original_port && restore.final_port && restore.original_port !== restore.final_port
      ? ` The board moved from ${restore.original_port} to ${restore.final_port}; that is normal after a restore.`
      : '';
    return {
      kind: 'restore',
      outcome: 'success',
      title: 'WVU Firmware restored',
      explanation: `The WVU firmware was written and verified${restore.final_port ? ` on ${restore.final_port}` : ''}.${moved}`,
      recommendedAction: 'Hardware recording is available on this board.',
      ...ports,
      diagnostics: diagnosticsText('restore', restore, undefined, 'restored and verified')
    };
  }
  const restoreFailure = restore.failure ?? undefined;
  const text = restoreFailure
    ? failureText(restoreFailure)
    : restore.stage === 'canceled'
      ? { title: 'The firmware restore was canceled', explanation: 'The restore was canceled before the board was written.', recommendedAction: 'Restore the WVU firmware again when you are ready.' }
      : { title: 'The firmware restore did not finish', explanation: restore.message, recommendedAction: 'Restore the WVU firmware again. If it fails again, copy the diagnostics for your instructor.' };
  return {
    kind: 'restore',
    outcome: 'failure',
    ...text,
    ...ports,
    diagnostics: diagnosticsText('restore', restore, restoreFailure, restore.stage === 'canceled' ? 'canceled' : 'failed')
  };
}
