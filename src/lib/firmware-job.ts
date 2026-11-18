/**
 * Plain-language labels for the firmware job shown in the busy modal.
 *
 * The backend reports `kind` and `stage` as snake_case identifiers
 * (`restore_reference`, `waiting_for_bootloader`, …). Students never see
 * those names: every title, step, and Cancel decision comes from here.
 */
export type FirmwareJobSummary = {
  kind?: string | null;
  stage?: string | null;
  message?: string | null;
};

/** Shown instead of Cancel once the board is being written. */
export const FIRMWARE_WRITE_NOTICE = 'Writing firmware — do not unplug the board';

const JOB_TITLES: Record<string, string> = {
  restore_reference: 'Restoring WVU Firmware…'
};

const STAGE_LABELS: Record<string, string> = {
  idle: 'Getting ready',
  preparing: 'Getting ready',
  closing_serial_session: 'Closing the board connection',
  compiling: 'Preparing the WVU firmware',
  touch_reset: 'Getting the board ready for the upload',
  waiting_for_bootloader: 'Waiting for the board to switch to upload mode',
  uploading: 'Writing the firmware to the board',
  waiting_for_application_port: 'Waiting for the board to restart',
  verifying_firmware: 'Checking the restored firmware',
  complete: 'Finished',
  failed: 'Stopped with a problem',
  canceled: 'Canceled'
};

/**
 * From the upload onward the backend refuses Cancel (a flash stopped part-way
 * helps no one), so the modal hides the button for these stages.
 */
const NOT_CANCELABLE_STAGES = new Set(['uploading', 'waiting_for_application_port', 'verifying_firmware']);

export function firmwareJobTitle(job: FirmwareJobSummary | null | undefined): string {
  return (job?.kind && JOB_TITLES[job.kind]) || 'Firmware operation in progress…';
}

export function firmwareJobStageLabel(stage: string | null | undefined): string {
  return (stage && STAGE_LABELS[stage]) || 'Working';
}

export function firmwareJobCancelable(stage: string | null | undefined): boolean {
  return !NOT_CANCELABLE_STAGES.has(stage ?? '');
}

export type FirmwareJobView = {
  title: string;
  step: string;
  detail: string;
  cancelable: boolean;
  cancelLabel: string;
};

/** The six steps of a restore as students see them, in order. */
export type FirmwareStepId = 'prepare' | 'compile' | 'switch' | 'upload' | 'reconnect' | 'verify';

export const FIRMWARE_STEPS: ReadonlyArray<{ id: FirmwareStepId; label: string }> = [
  { id: 'prepare', label: 'Prepare' },
  { id: 'compile', label: 'Compile' },
  { id: 'switch', label: 'Switch board to upload mode' },
  { id: 'upload', label: 'Upload' },
  { id: 'reconnect', label: 'Reconnect' },
  { id: 'verify', label: 'Verify' }
];

/** Rust `FirmwareJobStage` → step index (the terminal stages are handled separately). */
const STAGE_STEP: Record<string, number> = {
  idle: 0,
  preparing: 0,
  closing_serial_session: 0,
  compiling: 1,
  touch_reset: 2,
  waiting_for_bootloader: 2,
  uploading: 3,
  waiting_for_application_port: 4,
  verifying_firmware: 5
};

export type FirmwareStepStatus = 'done' | 'current' | 'pending' | 'failed';

export type FirmwareStep = { id: FirmwareStepId; label: string; status: FirmwareStepStatus };

export type FirmwareStepperJob = FirmwareJobSummary & { failure?: { stage?: string | null } | null };

/** The step a stage belongs to; `complete` is past the last step. */
export function firmwareStepIndex(stage: string | null | undefined): number | undefined {
  if (stage === 'complete') return FIRMWARE_STEPS.length;
  return stage != null && Object.prototype.hasOwnProperty.call(STAGE_STEP, stage) ? STAGE_STEP[stage] : undefined;
}

/**
 * Done / current / pending for each step. A failed or canceled job marks the
 * step its failure happened in (`failure.stage`) as failed.
 */
export function firmwareStepper(job: FirmwareStepperJob | null | undefined): FirmwareStep[] {
  const stage = job?.stage ?? '';
  const terminalFailure = stage === 'failed' || stage === 'canceled';
  const index = terminalFailure ? firmwareStepIndex(job?.failure?.stage) : firmwareStepIndex(stage);
  return FIRMWARE_STEPS.map((step, position) => {
    let status: FirmwareStepStatus = 'pending';
    if (index !== undefined) {
      if (position < index) status = 'done';
      else if (position === index) status = terminalFailure ? 'failed' : 'current';
    }
    return { ...step, status };
  });
}

/** "Step 3 of 6: Switch board to upload mode", announced once per step. */
export function firmwareStepAnnouncement(job: FirmwareStepperJob | null | undefined): string {
  const index = firmwareStepIndex(job?.stage);
  if (index === undefined || index >= FIRMWARE_STEPS.length) return '';
  return `Restoring WVU Firmware, step ${index + 1} of ${FIRMWARE_STEPS.length}: ${FIRMWARE_STEPS[index].label}.`;
}

/** The backend's double-reset wait: `NATIVE_USB_MANUAL_RESET_TIMEOUT`. */
export const DOUBLE_RESET_SECONDS = 90;

/**
 * Seconds left to press RESET twice, while a native-USB restore waits for
 * it (`manual_reset_deadline_utc` is set only then); otherwise undefined.
 */
export function doubleResetSecondsLeft(
  job: { active?: boolean; stage?: string | null; manual_reset_deadline_utc?: string | null } | null | undefined,
  nowMs: number
): number | undefined {
  if (!job?.active || job.stage !== 'waiting_for_bootloader' || !job.manual_reset_deadline_utc) return undefined;
  const deadline = Date.parse(job.manual_reset_deadline_utc);
  if (!Number.isFinite(deadline)) return undefined;
  return Math.min(DOUBLE_RESET_SECONDS, Math.max(0, Math.ceil((deadline - nowMs) / 1000)));
}

export function firmwareJobView(job: FirmwareJobSummary | null | undefined): FirmwareJobView {
  const step = firmwareJobStageLabel(job?.stage);
  const message = job?.message?.trim();
  return {
    title: firmwareJobTitle(job),
    step,
    detail: message || `${step}…`,
    cancelable: firmwareJobCancelable(job?.stage),
    cancelLabel: 'Cancel restore'
  };
}
