export type BoardControlState = {
  selectedBoard: boolean;
  recordingActive: boolean;
  boardOperationBusy: boolean;
  arduinoToolsReady: boolean;
  firmwareStatus: string;
};

/**
 * Separates board-recovery controls from the stricter acquisition gate. A board
 * with unknown, silent, or incompatible firmware is exactly when Restore WVU
 * Firmware must remain available.
 */
export function boardControls(state: BoardControlState) {
  const idle = !state.recordingActive && !state.boardOperationBusy;
  return {
    canSelectBoard: idle,
    canRefreshBoards: idle,
    canVerifyFirmware: idle && state.selectedBoard,
    canRestoreFirmware: idle && state.selectedBoard && state.arduinoToolsReady,
    canStartHardwareAcquisition: idle
      && state.selectedBoard
      && state.firmwareStatus === 'wvu_protocol_compatible'
  };
}
