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
