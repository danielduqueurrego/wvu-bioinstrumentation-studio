/**
 * The compact readiness checklist shown beside Start when a recording cannot
 * begin: Board · Firmware · Lab · Folder · Source. Every failing item names
 * the control that fixes it, so the page can open its setup card, scroll to
 * it, and focus it. The checklist explains `recordingStartReadiness`; it
 * never decides on its own whether Start is allowed.
 */
import { NEXT_ACTION_LABELS, type NextAction } from './messages';
import type { SetupCardId } from './setup-cards';

export type ChecklistStatus = 'ok' | 'fail' | 'skip';

/** A control a failing item can move focus to. */
export type ChecklistTarget =
  | 'board-select'
  | 'verify-firmware'
  | 'lab-select'
  | 'bench-acknowledgement'
  | 'duration'
  | 'adc-reference'
  | 'mpxv-supply'
  | 'project-folder'
  | 'output-folder'
  | 'message-action'
  | 'clear-connection';

export type ChecklistItem = {
  id: 'board' | 'firmware' | 'lab' | 'folder' | 'source';
  label: string;
  status: ChecklistStatus;
  detail: string;
  target?: ChecklistTarget;
};

/**
 * Where each target lives: the element ids to try in order (the first one
 * present wins) and the setup card that must be open for it to be focusable.
 */
export const CHECKLIST_TARGETS: Record<ChecklistTarget, { elementIds: string[]; card?: SetupCardId }> = {
  'board-select': { elementIds: ['board-select'], card: 'board' },
  // The Board panel's fitting action (Restore when no WVU firmware) comes
  // first; Verify Firmware is always present otherwise.
  'verify-firmware': { elementIds: ['board-primary-action', 'verify-firmware-button', 'board-select'], card: 'board' },
  'lab-select': { elementIds: ['lab-select'], card: 'lab' },
  'bench-acknowledgement': { elementIds: ['bench-acknowledgement'], card: 'lab' },
  duration: { elementIds: ['duration-custom-input', 'duration-preset-select', 'duration-mode-timed', 'duration-mode-until-stopped'], card: 'duration' },
  'adc-reference': { elementIds: ['adc-reference-input'], card: 'calibration' },
  'mpxv-supply': { elementIds: ['mpxv-supply-input'], card: 'calibration' },
  'project-folder': { elementIds: ['project-folder-browse'], card: 'project' },
  'output-folder': { elementIds: ['output-folder-input'], card: 'session' },
  'message-action': { elementIds: ['message-action-button'] },
  'clear-connection': { elementIds: ['clear-connection-button'] }
};

export type ChecklistInput = {
  source: 'hardware' | 'simulator';
  selectedPort: string;
  firmwareReady: boolean;
  /**
   * The board answered with another WVU firmware build (the Board card's
   * Older/Newer WVU firmware states). The Firmware item then names the fix.
   */
  firmwareMismatch?: 'older' | 'newer';
  /** A Faulted session that must be cleared before a hardware Start. */
  blockingFault: boolean;
  /**
   * The message center's button for that fault, if it offers one. The item
   * names that button and moves focus to it; without one it names the
   * recording bar's Clear connection.
   */
  faultAction?: NextAction;
  labName?: string;
  acknowledgementSatisfied: boolean;
  durationError?: string;
  adcReferenceError?: string;
  mpxvSupplyError?: string;
  projectFolder: string;
  outputFolderError?: string;
};

export function readinessChecklist(input: ChecklistInput): ChecklistItem[] {
  const hardware = input.source === 'hardware';
  const board: ChecklistItem = !hardware
    ? { id: 'board', label: 'Board', status: 'skip', detail: 'not needed for Simulator' }
    : input.selectedPort
      ? { id: 'board', label: 'Board', status: 'ok', detail: input.selectedPort }
      : { id: 'board', label: 'Board', status: 'fail', detail: 'select a board', target: 'board-select' };

  const firmware: ChecklistItem = !hardware
    ? { id: 'firmware', label: 'Firmware', status: 'skip', detail: 'not needed for Simulator' }
    : !input.selectedPort
      ? { id: 'firmware', label: 'Firmware', status: 'fail', detail: 'select a board first', target: 'board-select' }
      : input.firmwareReady
        ? { id: 'firmware', label: 'Firmware', status: 'ok', detail: 'ready' }
        : input.firmwareMismatch === 'older'
          ? { id: 'firmware', label: 'Firmware', status: 'fail', detail: 'restore the WVU firmware', target: 'verify-firmware' }
          : input.firmwareMismatch === 'newer'
            ? { id: 'firmware', label: 'Firmware', status: 'fail', detail: 'update this app or restore', target: 'verify-firmware' }
            : { id: 'firmware', label: 'Firmware', status: 'fail', detail: 'verify or restore', target: 'verify-firmware' };

  let lab: ChecklistItem;
  if (!input.labName) lab = { id: 'lab', label: 'Lab', status: 'fail', detail: 'choose a lab', target: 'lab-select' };
  else if (input.durationError) lab = { id: 'lab', label: 'Lab', status: 'fail', detail: 'choose a valid duration', target: 'duration' };
  else if (input.adcReferenceError) lab = { id: 'lab', label: 'Lab', status: 'fail', detail: 'fix the ADC reference voltage', target: 'adc-reference' };
  else if (input.mpxvSupplyError) lab = { id: 'lab', label: 'Lab', status: 'fail', detail: 'fix the MPXV sensor supply', target: 'mpxv-supply' };
  else if (!input.acknowledgementSatisfied) lab = { id: 'lab', label: 'Lab', status: 'fail', detail: 'confirm the course notice', target: 'bench-acknowledgement' };
  else lab = { id: 'lab', label: 'Lab', status: 'ok', detail: input.labName };

  const folder: ChecklistItem = !input.projectFolder
    ? { id: 'folder', label: 'Folder', status: 'fail', detail: 'choose a Project folder', target: 'project-folder' }
    : input.outputFolderError
      ? { id: 'folder', label: 'Folder', status: 'fail', detail: 'fix the Output folder name', target: 'output-folder' }
      : { id: 'folder', label: 'Folder', status: 'ok', detail: 'ready' };

  const source: ChecklistItem = hardware && input.blockingFault
    ? input.faultAction
      ? { id: 'source', label: 'Source', status: 'fail', detail: `select ${NEXT_ACTION_LABELS[input.faultAction]} first`, target: 'message-action' }
      : { id: 'source', label: 'Source', status: 'fail', detail: 'select Clear connection first', target: 'clear-connection' }
    : { id: 'source', label: 'Source', status: 'ok', detail: hardware ? `Hardware${input.selectedPort ? ` · ${input.selectedPort}` : ''}` : 'Simulator (practice)' };

  return [board, firmware, lab, folder, source];
}

export function firstFailingItem(items: ChecklistItem[]): ChecklistItem | undefined {
  return items.find((item) => item.status === 'fail');
}
