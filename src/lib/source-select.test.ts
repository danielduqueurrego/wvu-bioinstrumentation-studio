import { describe, expect, it } from 'vitest';
import { shouldAutoSelectHardware, type SourceAutoSelectInput } from './source-select';

const firstReady: SourceAutoSelectInput = {
  source: 'simulator',
  autoSelected: false,
  userChose: false,
  recordingSeen: false,
  sessionActive: false,
  wasReady: false,
  ready: true
};

describe('Source auto-select', () => {
  it('switches to Hardware on the first transition to firmware ready while idle', () => {
    expect(shouldAutoSelectHardware(firstReady)).toBe(true);
  });

  it('needs a transition: not while still unready, and not again while it stays ready', () => {
    expect(shouldAutoSelectHardware({ ...firstReady, ready: false })).toBe(false);
    expect(shouldAutoSelectHardware({ ...firstReady, wasReady: true })).toBe(false);
  });

  it('switches only once', () => {
    expect(shouldAutoSelectHardware({ ...firstReady, autoSelected: true })).toBe(false);
  });

  it('never overrides a Source the student chose', () => {
    expect(shouldAutoSelectHardware({ ...firstReady, userChose: true })).toBe(false);
  });

  it('never switches during or after a recording', () => {
    expect(shouldAutoSelectHardware({ ...firstReady, sessionActive: true })).toBe(false);
    expect(shouldAutoSelectHardware({ ...firstReady, recordingSeen: true })).toBe(false);
  });

  it('has nothing to do when Hardware is already selected', () => {
    expect(shouldAutoSelectHardware({ ...firstReady, source: 'hardware' })).toBe(false);
  });
});
