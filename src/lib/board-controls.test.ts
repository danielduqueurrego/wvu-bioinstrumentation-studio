import { describe, expect, it } from 'vitest';
import { boardControls } from './board-controls';

const supportedBoard = {
  selectedBoard: true,
  recordingActive: false,
  boardOperationBusy: false,
  arduinoToolsReady: true
};

describe('board recovery controls', () => {
  it.each([
    'wvu_protocol_compatible',
    'wrong_firmware_identity',
    'non_wvu_sketch',
    'verification_failed',
    'unknown'
  ])('keeps recovery available for %s firmware', (firmwareStatus) => {
    const controls = boardControls({ ...supportedBoard, firmwareStatus });
    expect(controls.canSelectBoard).toBe(true);
    expect(controls.canRefreshBoards).toBe(true);
    expect(controls.canVerifyFirmware).toBe(true);
    expect(controls.canRestoreFirmware).toBe(true);
    expect(controls.canStartHardwareAcquisition).toBe(
      firmwareStatus === 'wvu_protocol_compatible'
    );
  });

  it('keeps Refresh available without a board but blocks board-specific actions', () => {
    const controls = boardControls({
      selectedBoard: false,
      recordingActive: false,
      boardOperationBusy: false,
      arduinoToolsReady: true,
      firmwareStatus: 'unknown'
    });
    expect(controls.canRefreshBoards).toBe(true);
    expect(controls.canVerifyFirmware).toBe(false);
    expect(controls.canRestoreFirmware).toBe(false);
    expect(controls.canStartHardwareAcquisition).toBe(false);
  });

  it('requires bundled tools only for Restore', () => {
    const controls = boardControls({
      ...supportedBoard,
      arduinoToolsReady: false,
      firmwareStatus: 'verification_failed'
    });
    expect(controls.canSelectBoard).toBe(true);
    expect(controls.canRefreshBoards).toBe(true);
    expect(controls.canVerifyFirmware).toBe(true);
    expect(controls.canRestoreFirmware).toBe(false);
  });

  it.each(['recording active', 'board operation active'])('blocks conflicting actions while %s', (condition) => {
    const controls = boardControls({
      ...supportedBoard,
      firmwareStatus: 'wvu_protocol_compatible',
      recordingActive: condition === 'recording active',
      boardOperationBusy: condition === 'board operation active'
    });
    expect(controls.canSelectBoard).toBe(false);
    expect(controls.canRefreshBoards).toBe(false);
    expect(controls.canVerifyFirmware).toBe(false);
    expect(controls.canRestoreFirmware).toBe(false);
    expect(controls.canStartHardwareAcquisition).toBe(false);
  });
});
