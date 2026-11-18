/** The values of the Rust `FirmwareCompatibility` enum (snake_case). */
export type FirmwareCompatibility =
  | 'unknown'
  | 'wvu_protocol_compatible'
  | 'wvu_protocol_incompatible'
  | 'upload_in_progress'
  | 'verification_failed';

export type BoardControlState = {
  selectedBoard: boolean;
  recordingActive: boolean;
  boardOperationBusy: boolean;
  arduinoToolsReady: boolean;
};

/**
 * Separates board-recovery controls from the stricter acquisition gate. A board
 * with unknown, silent, or incompatible firmware is exactly when Restore WVU
 * Firmware must remain available, so firmware status is not an input here.
 */
export function boardControls(state: BoardControlState) {
  const idle = !state.recordingActive && !state.boardOperationBusy;
  return {
    canSelectBoard: idle,
    canRefreshBoards: idle,
    canVerifyFirmware: idle && state.selectedBoard,
    canRestoreFirmware: idle && state.selectedBoard && state.arduinoToolsReady
  };
}

/**
 * Hardware Start is allowed only on the port where compatible firmware was
 * proven, matching the backend's `is_acquisition_allowed` (`verified_port`).
 * Windows COM names compare case-insensitively.
 */
export function firmwareReadyOnPort(
  compatibility: FirmwareCompatibility | string,
  selectedPort: string,
  verifiedPort: string | null | undefined
): boolean {
  return compatibility === 'wvu_protocol_compatible'
    && Boolean(selectedPort)
    && (verifiedPort ?? '').toUpperCase() === selectedPort.toUpperCase();
}
