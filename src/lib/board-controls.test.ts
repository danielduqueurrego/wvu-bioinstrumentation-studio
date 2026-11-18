import { describe, expect, it } from 'vitest';
import { boardControls, firmwareReadyOnPort, type FirmwareCompatibility } from './board-controls';

const supportedBoard = {
  selectedBoard: true,
  recordingActive: false,
  boardOperationBusy: false,
  arduinoToolsReady: true
};

describe('board recovery controls', () => {
  it('keeps recovery available whatever the firmware status is', () => {
    // Firmware status is deliberately not an input: recovery must never
    // depend on firmware readiness.
    const controls = boardControls(supportedBoard);
    expect(controls.canSelectBoard).toBe(true);
    expect(controls.canRefreshBoards).toBe(true);
    expect(controls.canVerifyFirmware).toBe(true);
    expect(controls.canRestoreFirmware).toBe(true);
  });

  it('keeps Refresh available without a board but blocks board-specific actions', () => {
    const controls = boardControls({ ...supportedBoard, selectedBoard: false });
    expect(controls.canRefreshBoards).toBe(true);
    expect(controls.canVerifyFirmware).toBe(false);
    expect(controls.canRestoreFirmware).toBe(false);
  });

  it('requires bundled tools only for Restore', () => {
    const controls = boardControls({ ...supportedBoard, arduinoToolsReady: false });
    expect(controls.canSelectBoard).toBe(true);
    expect(controls.canRefreshBoards).toBe(true);
    expect(controls.canVerifyFirmware).toBe(true);
    expect(controls.canRestoreFirmware).toBe(false);
  });

  it.each(['recording active', 'board operation active'])('blocks conflicting actions while %s', (condition) => {
    const controls = boardControls({
      ...supportedBoard,
      recordingActive: condition === 'recording active',
      boardOperationBusy: condition === 'board operation active'
    });
    expect(controls.canSelectBoard).toBe(false);
    expect(controls.canRefreshBoards).toBe(false);
    expect(controls.canVerifyFirmware).toBe(false);
    expect(controls.canRestoreFirmware).toBe(false);
  });
});

describe('hardware Start firmware gate', () => {
  it('allows hardware Start only on the port where compatible firmware was verified', () => {
    expect(firmwareReadyOnPort('wvu_protocol_compatible', 'COM5', 'COM5')).toBe(true);
    expect(firmwareReadyOnPort('wvu_protocol_compatible', 'com5', 'COM5')).toBe(true);
    // Verified on another board (for example after the COM number changed).
    expect(firmwareReadyOnPort('wvu_protocol_compatible', 'COM6', 'COM5')).toBe(false);
    expect(firmwareReadyOnPort('wvu_protocol_compatible', 'COM5', null)).toBe(false);
    expect(firmwareReadyOnPort('wvu_protocol_compatible', '', '')).toBe(false);
  });

  it.each<FirmwareCompatibility>([
    'unknown',
    'wvu_protocol_incompatible',
    'upload_in_progress',
    'verification_failed'
  ])('refuses hardware Start for %s firmware', (compatibility) => {
    expect(firmwareReadyOnPort(compatibility, 'COM5', 'COM5')).toBe(false);
  });

  it('refuses a status string Rust never sends', () => {
    expect(firmwareReadyOnPort('non_wvu_sketch', 'COM5', 'COM5')).toBe(false);
  });
});
