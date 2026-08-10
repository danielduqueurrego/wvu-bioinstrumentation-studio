export type FirmwareCompatibility =
  | 'unknown'
  | 'wvu_protocol_compatible'
  | 'wvu_protocol_incompatible'
  | 'non_wvu_sketch'
  | 'upload_in_progress'
  | 'verification_failed';

export type FirmwareControlsInput = {
  hasProject: boolean;
  unsavedChanges: boolean;
  activeJob: boolean;
  hasCurrentCompile: boolean;
  selectedPort: string;
};

export function firmwareControls(input: FirmwareControlsInput) {
  const saveEnabled = input.hasProject && input.unsavedChanges;
  const compileEnabled = input.hasProject && !input.unsavedChanges && !input.activeJob;
  const uploadEnabled = compileEnabled && input.hasCurrentCompile && Boolean(input.selectedPort);
  return {
    saveEnabled,
    compileEnabled,
    uploadEnabled,
    restoreEnabled: Boolean(input.selectedPort) && !input.activeJob
  };
}

export function firmwareCompatibilityMessage(value: FirmwareCompatibility) {
  switch (value) {
    case 'wvu_protocol_compatible':
      return 'Firmware ready';
    case 'non_wvu_sketch':
      return 'Upload complete — Restore WVU Firmware before using Acquisition.';
    case 'wvu_protocol_incompatible':
      return 'Firmware update required — restore WVU Firmware before using Acquisition.';
    case 'upload_in_progress':
      return 'Upload in progress — Acquisition is disabled until verification finishes.';
    case 'verification_failed':
      return 'Firmware needs attention — restore WVU Firmware before using Acquisition.';
    default:
      return 'Firmware has not been checked yet.';
  }
}
