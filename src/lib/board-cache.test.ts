import { describe, expect, it } from 'vitest';
import { reconcileBoardCache } from './board-cache';

const uno = { port: 'COM12', name: 'Arduino UNO R4 WiFi', fqbn: 'arduino:renesas_uno:unor4wifi' };

describe('root board cache reconciliation', () => {
  it('auto-selects and schedules exactly one verified board when no selection exists', () => {
    expect(reconcileBoardCache('', [uno])).toMatchObject({
      selectedPort: 'COM12', verificationPort: 'COM12', selectionCleared: false
    });
  });

  it('does not guess when multiple supported boards are cached', () => {
    const result = reconcileBoardCache('', [uno, { ...uno, port: 'COM13' }]);
    expect(result).toMatchObject({ selectedPort: '', selectionCleared: false });
    expect(result.verificationPort).toBeUndefined();
  });

  it('preserves the selected board across an explicit refresh and clears a disappeared one', () => {
    const preserved = reconcileBoardCache('COM12', [uno]);
    expect(preserved).toMatchObject({ selectedPort: 'COM12', selectionCleared: false });
    expect(preserved.verificationPort).toBeUndefined();
    const cleared = reconcileBoardCache('COM12', []);
    expect(cleared).toMatchObject({ selectedPort: '', selectionCleared: true });
    expect(cleared.verificationPort).toBeUndefined();
  });

  it('selects the board that appeared after a failed restore without verifying it', () => {
    // COM34 went into upload mode and came back as COM9: the selection
    // follows it, but no Verify replaces the restore's failure.
    const result = reconcileBoardCache('COM34', [{ ...uno, port: 'COM9' }], { verifyNewSelection: false });
    expect(result).toMatchObject({ selectedPort: 'COM9', selectionCleared: true });
    expect(result.verificationPort).toBeUndefined();
    expect(reconcileBoardCache('COM34', [{ ...uno, port: 'COM35' }]).verificationPort).toBe('COM35');
  });
});
