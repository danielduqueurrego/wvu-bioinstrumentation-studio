import { describe, expect, it } from 'vitest';
import { formatClock, formatCount, formatRate, plannedDurationText, recordingActive, recordingChip, recordingProgress } from './recording-status';

const idle = { sessionState: 'Disconnected', startInFlight: false, canStart: true, source: 'hardware' as const, boardSelected: true };

describe('recording state chip', () => {
  it.each([
    [{ sessionState: 'Acquiring' }, 'Recording', 'recording'],
    [{ sessionState: 'Stopping' }, 'Saving…', 'saving'],
    [{ sessionState: 'Connecting' }, 'Connecting…', 'busy'],
    [{ sessionState: 'Configured' }, 'Connecting…', 'busy'],
    [{ startInFlight: true }, 'Connecting…', 'busy'],
    [{ sessionState: 'Faulted' }, 'Needs attention', 'problem'],
    [{}, 'Ready', 'ready'],
    [{ canStart: false, boardSelected: false }, 'Disconnected', 'idle'],
    [{ canStart: false }, 'Not ready', 'idle'],
    [{ canStart: false, source: 'simulator' as const, boardSelected: false }, 'Not ready', 'idle']
  ])('%o → %s', (overrides, label, tone) => {
    expect(recordingChip({ ...idle, ...overrides })).toEqual({ label, tone });
  });

  it('shows a Verify or Retry handshake as a board check, never as a recording connecting', () => {
    for (const sessionState of ['Connecting', 'Connected']) {
      expect(recordingChip({ ...idle, sessionState, sessionActive: false, canStart: false })).toEqual({ label: 'Checking board…', tone: 'busy' });
      expect(recordingChip({ ...idle, sessionState, sessionActive: true })).toEqual({ label: 'Connecting…', tone: 'busy' });
    }
  });

  it('shows a fault that does not concern the chosen Source as ready', () => {
    expect(recordingChip({ ...idle, sessionState: 'Faulted', faulted: false, source: 'simulator' })).toEqual({ label: 'Ready', tone: 'ready' });
    expect(recordingChip({ ...idle, sessionState: 'Faulted', faulted: true })).toEqual({ label: 'Needs attention', tone: 'problem' });
  });
});

describe('recording ownership of the session', () => {
  it('trusts the backend flag and falls back to the state without it', () => {
    expect(recordingActive({ state: 'Connecting', recording_active: false })).toBe(false);
    expect(recordingActive({ state: 'Connected', recording_active: false })).toBe(false);
    expect(recordingActive({ state: 'Acquiring', recording_active: true })).toBe(true);
    expect(recordingActive({ state: 'Acquiring' })).toBe(true);
    expect(recordingActive({ state: 'Faulted' })).toBe(false);
    expect(recordingActive({ state: 'Disconnected', recording_active: null })).toBe(false);
  });
});

describe('recording clock and progress', () => {
  it('formats clocks', () => {
    expect(formatClock(0)).toBe('0:00');
    expect(formatClock(42.9)).toBe('0:42');
    expect(formatClock(725)).toBe('12:05');
    expect(formatClock(3723)).toBe('1:02:03');
    expect(formatClock(null)).toBe('—');
    expect(formatClock(-3)).toBe('0:00');
  });

  it('counts a timed recording from START to its end', () => {
    expect(recordingProgress({ mode: 'timed', seconds: 60 }, 42, 18)).toEqual({ elapsedText: '0:42', totalText: '1:00', remainingText: '0:18', fraction: 0.7 });
    expect(recordingProgress({ mode: 'timed', seconds: 60 }, 59.4, 0.6).remainingText).toBe('0:01');
    expect(recordingProgress({ mode: 'timed', seconds: 60 }, 61, null)).toMatchObject({ remainingText: '0:00', fraction: 1 });
  });

  it('shows only elapsed time until stopped', () => {
    expect(recordingProgress({ mode: 'until_stopped' }, 75, null)).toEqual({ elapsedText: '1:15' });
    expect(recordingProgress(null, Number.NaN, null)).toEqual({ elapsedText: '0:00' });
  });

  it('formats rates with one decimal and counts with separators', () => {
    expect(formatRate(99.94)).toBe('99.9 Hz');
    expect(formatRate(undefined)).toBe('—');
    expect(formatCount(12345)).toBe('12,345');
  });

  it('describes the planned duration before Start', () => {
    expect(plannedDurationText('timed', 60)).toBe('1:00 timed');
    expect(plannedDurationText('until_stopped', 60)).toBe('Until stopped');
    expect(plannedDurationText('timed', Number.NaN)).toBe('Timed');
  });
});
