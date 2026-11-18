/**
 * What the sticky recording bar shows: the state chip (always text, never
 * colour alone), the elapsed/remaining clock with a progress fraction for a
 * timed recording, and the frame and rate readouts. The backend counts
 * `elapsed_seconds` from START, so the progress reaches 100 % when the
 * recording actually ends.
 */
import type { RecordingDurationRequest } from './duration';

export type ChipTone = 'ready' | 'idle' | 'busy' | 'recording' | 'saving' | 'problem';

export type RecordingChip = { label: string; tone: ChipTone };

/** Session states in which a recording or a handshake check owns the board. */
const SESSION_BUSY_STATES = new Set(['Connecting', 'Connected', 'Configured', 'Acquiring', 'Stopping']);

/**
 * Whether a recording owns the session. Verify Firmware, Retry connection,
 * and a restore's verification pass through Connecting/Connected as well;
 * the backend's `recording_active` tells them apart. (A status without the
 * field falls back to the state.)
 */
export function recordingActive(status: { state: string; recording_active?: boolean | null }): boolean {
  return typeof status.recording_active === 'boolean' ? status.recording_active : SESSION_BUSY_STATES.has(status.state);
}

export function recordingChip(input: {
  sessionState: string;
  /** A recording owns the session (`recordingActive`); defaults to the state. */
  sessionActive?: boolean;
  /** A fault that concerns the chosen Source; defaults to `Faulted`. */
  faulted?: boolean;
  startInFlight: boolean;
  canStart: boolean;
  source: 'hardware' | 'simulator';
  boardSelected: boolean;
}): RecordingChip {
  const state = input.sessionState;
  const active = input.sessionActive ?? SESSION_BUSY_STATES.has(state);
  if (state === 'Acquiring') return { label: 'Recording', tone: 'recording' };
  if (state === 'Stopping') return { label: 'Saving…', tone: 'saving' };
  if (input.startInFlight || (active && SESSION_BUSY_STATES.has(state))) return { label: 'Connecting…', tone: 'busy' };
  // Verify Firmware or Retry connection is talking to the board.
  if (SESSION_BUSY_STATES.has(state)) return { label: 'Checking board…', tone: 'busy' };
  if (input.faulted ?? state === 'Faulted') return { label: 'Needs attention', tone: 'problem' };
  if (input.canStart) return { label: 'Ready', tone: 'ready' };
  if (input.source === 'hardware' && !input.boardSelected) return { label: 'Disconnected', tone: 'idle' };
  return { label: 'Not ready', tone: 'idle' };
}

/** "0:42", "12:05", "1:02:03"; "—" when unknown. */
export function formatClock(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds)) return '—';
  const whole = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(whole / 3600);
  const minutes = Math.floor((whole % 3600) / 60);
  const remainder = whole % 60;
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, '0')}:${String(remainder).padStart(2, '0')}`
    : `${minutes}:${String(remainder).padStart(2, '0')}`;
}

export type RecordingProgress = {
  elapsedText: string;
  /** Timed recordings only. */
  totalText?: string;
  remainingText?: string;
  /** 0–1 for a timed recording. */
  fraction?: number;
};

export function recordingProgress(
  duration: RecordingDurationRequest | null | undefined,
  elapsedSeconds: number,
  remainingSeconds: number | null | undefined
): RecordingProgress {
  const elapsed = Number.isFinite(elapsedSeconds) ? Math.max(0, elapsedSeconds) : 0;
  if (duration?.mode !== 'timed' || !(duration.seconds > 0)) return { elapsedText: formatClock(elapsed) };
  const total = duration.seconds;
  const remaining = remainingSeconds != null && Number.isFinite(remainingSeconds)
    ? Math.max(0, remainingSeconds)
    : Math.max(0, total - elapsed);
  return {
    elapsedText: formatClock(elapsed),
    totalText: formatClock(total),
    remainingText: formatClock(Math.ceil(remaining)),
    fraction: Math.min(1, Math.max(0, elapsed / total))
  };
}

/** Measured rate with one decimal, e.g. "99.9 Hz". */
export function formatRate(hz: number | null | undefined): string {
  return hz != null && Number.isFinite(hz) ? `${hz.toFixed(1)} Hz` : '—';
}

export function formatCount(value: number | null | undefined): string {
  return value != null && Number.isFinite(value) ? Math.round(value).toLocaleString('en-US') : '—';
}

/** The planned duration shown before Start, e.g. "60 s timed". */
export function plannedDurationText(mode: 'timed' | 'until_stopped', seconds: number): string {
  if (mode === 'until_stopped') return 'Until stopped';
  return Number.isFinite(seconds) && seconds > 0 ? `${formatClock(seconds)} timed` : 'Timed';
}
