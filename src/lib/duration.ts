export type RecordingDurationRequest =
  | { mode: 'timed'; seconds: number }
  | { mode: 'until_stopped' };

export const MIN_TIMED_SECONDS = 10;

export function isTimedDurationValid(seconds: number): boolean {
  return Number.isFinite(seconds) && Number.isInteger(seconds) && seconds >= MIN_TIMED_SECONDS;
}

export function durationRequest(
  mode: 'timed' | 'until_stopped',
  seconds: number
): RecordingDurationRequest {
  return mode === 'until_stopped'
    ? { mode: 'until_stopped' }
    : { mode: 'timed', seconds };
}

export function remainingTimeVisible(duration: RecordingDurationRequest): boolean {
  return duration.mode === 'timed';
}
