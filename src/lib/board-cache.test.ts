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
});
