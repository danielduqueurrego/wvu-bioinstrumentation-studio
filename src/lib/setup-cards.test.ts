import { describe, expect, it } from 'vitest';
import { collapseAll, initialCardState, openCard, recordingDisclosure, withModeDefaults } from './setup-cards';

describe('setup card disclosure', () => {
  it('starts with Calibration & Units collapsed in Student mode only', () => {
    expect(initialCardState(false)).toEqual({ board: true, project: true, lab: true, session: true, duration: true, calibration: false });
    expect(initialCardState(true).calibration).toBe(true);
  });

  it('applies a mode default without touching the other cards', () => {
    const state = { ...initialCardState(false), board: false };
    expect(withModeDefaults(state, true)).toEqual({ ...state, calibration: true });
    expect(withModeDefaults(state, false)).toBe(state);
  });

  it('collapses every card while recording and restores the layout afterwards', () => {
    const before = { ...initialCardState(true), duration: false };
    const recording = recordingDisclosure({ open: before }, true);
    expect(Object.values(recording.open).every((open) => !open)).toBe(true);
    expect(recording.saved).toBe(before);
    // Every status poll repeats the same value; nothing changes.
    expect(recordingDisclosure(recording, true)).toBe(recording);
    const after = recordingDisclosure(recording, false);
    expect(after).toEqual({ open: before });
    expect(recordingDisclosure(after, false)).toBe(after);
  });

  it('lets a student open a card during a recording without losing the saved layout', () => {
    const before = initialCardState(false);
    const recording = recordingDisclosure({ open: before }, true);
    const peeked = { ...recording, open: openCard(recording.open, 'lab') };
    expect(peeked.open.lab).toBe(true);
    expect(recordingDisclosure(peeked, false).open).toEqual(before);
  });

  it('opens a card for a readiness target and keeps the same object when nothing changes', () => {
    const state = collapseAll(initialCardState(false));
    expect(openCard(state, 'duration').duration).toBe(true);
    const open = initialCardState(false);
    expect(openCard(open, 'board')).toBe(open);
    expect(collapseAll(state)).toBe(state);
  });
});
