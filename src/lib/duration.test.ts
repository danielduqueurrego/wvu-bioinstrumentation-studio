import { describe, expect, it } from 'vitest';
import {
  durationRequest,
  isTimedDurationValid,
  MIN_TIMED_SECONDS,
  remainingTimeVisible
} from './duration';

describe('recording duration controls', () => {
  it('uses an explicit until-stopped request without a numeric sentinel', () => {
    const duration = durationRequest('until_stopped', 0);
    expect(duration).toEqual({ mode: 'until_stopped' });
    expect(remainingTimeVisible(duration)).toBe(false);
  });

  it('validates timed duration and preserves remaining-time behavior', () => {
    expect(isTimedDurationValid(MIN_TIMED_SECONDS - 1)).toBe(false);
    expect(isTimedDurationValid(MIN_TIMED_SECONDS)).toBe(true);
    expect(isTimedDurationValid(60.5)).toBe(false);
    expect(remainingTimeVisible(durationRequest('timed', 60))).toBe(true);
  });
});
