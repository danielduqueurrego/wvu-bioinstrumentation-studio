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
      return 'WVU protocol compatible — Acquisition may be used after a compatible handshake.';
    case 'non_wvu_sketch':
      return 'Non-WVU sketch uploaded — upload succeeded, but Acquisition remains unavailable until the reference firmware is restored.';
    case 'wvu_protocol_incompatible':
      return 'WVU protocol firmware responded but its identity is incompatible.';
    case 'upload_in_progress':
      return 'Upload in progress — Acquisition is disabled until verification finishes.';
    case 'verification_failed':
      return 'Upload completed but WVU protocol verification failed. Restore the controlled reference firmware.';
    default:
      return 'Firmware compatibility has not been verified. Editing and compiling remain available.';
  }
}
