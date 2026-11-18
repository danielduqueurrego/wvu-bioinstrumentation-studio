import { afterEach, describe, expect, it, vi } from 'vitest';
import { copyText } from './clipboard';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('copyText', () => {
  it('writes to the clipboard when it is available', async () => {
    const writeText = vi.fn(async () => undefined);
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    await expect(copyText('C:\\BMEG 420L\\run.bmeg')).resolves.toBe(true);
    expect(writeText).toHaveBeenCalledWith('C:\\BMEG 420L\\run.bmeg');
  });

  it('reports failure when the clipboard is missing or refuses, so the text can be shown instead', async () => {
    vi.stubGlobal('navigator', {});
    await expect(copyText('x')).resolves.toBe(false);
    vi.stubGlobal('navigator', { clipboard: { writeText: async () => { throw new Error('denied'); } } });
    await expect(copyText('x')).resolves.toBe(false);
  });
});
