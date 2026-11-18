/**
 * The finished-recording card: which files were saved (✓/✗ from the paths
 * and `csv_error` / `finalize_error`), how the recording ended in plain
 * language, and the counts a student checks before leaving the bench.
 * Text only; it reads `SessionSummary` and never changes it.
 */
import { connectionFailureMessage } from './messages';
import { formatClock, formatCount, formatRate } from './recording-status';
import type { Integrity, Summary } from './session-types';

export type ResultFileId = 'bmeg' | 'csv' | 'metadata';

export type ResultFile = {
  id: ResultFileId;
  label: string;
  path: string;
  saved: boolean;
  /** Why the file is missing or incomplete. */
  note?: string;
};

export type RecordingResultView = {
  complete: boolean;
  headline: string;
  detail: string;
  files: ResultFile[];
  duration: string;
  samples: string;
  rate: string;
  markers: number;
  quality: string;
  qualityOk: boolean;
};

export type RecordingResultInput = {
  /** Markers added during this recording (the page keeps the list). */
  markerCount: number;
  /** `connection_diagnostics.failure_category` when the recording faulted. */
  failureCategory?: string | null;
};

function lowerFirst(text: string): string {
  // Keep acronyms such as "USB" as written.
  return /^[A-Z][a-z]/.test(text) ? text[0].toLowerCase() + text.slice(1) : text;
}

function sentence(text: string): string {
  const trimmed = text.trim();
  return /[.!?]$/.test(trimmed) ? trimmed : `${trimmed}.`;
}

function completion(summary: Summary, failureCategory: string | null | undefined): { complete: boolean; headline: string; detail: string } {
  const reason = summary.stop_reason;
  if (summary.completion_status === 'complete') {
    const detail = reason === 'timed_complete'
      ? 'The planned duration finished.'
      : reason === 'application_close'
        ? 'It was saved when the window was closed.'
        : 'Stopped by you.';
    return { complete: true, headline: 'Recording complete', detail };
  }
  switch (reason) {
    case 'fault': {
      const cause = connectionFailureMessage(failureCategory)?.headline;
      return {
        complete: false,
        headline: `Stopped early: ${cause ? lowerFirst(cause) : 'an error stopped the recording'}`,
        detail: 'The samples recorded before it stopped were saved.'
      };
    }
    case 'storage_guard':
      return {
        complete: false,
        headline: 'Stopped early: the disk was almost full',
        detail: 'Recording stopped to keep enough free space. The samples up to that point were saved.'
      };
    case 'disconnect':
      return { complete: false, headline: 'Stopped early: the connection was closed', detail: 'The samples recorded before it closed were saved.' };
    default:
      return { complete: false, headline: 'Recording ended with problems', detail: 'Check the files below before you leave the bench.' };
  }
}

function files(summary: Summary): ResultFile[] {
  const bmeg = summary.bmeg_path ?? '';
  const partial = bmeg.endsWith('.part');
  const csvError = summary.csv_error?.trim();
  const finalizeError = summary.finalize_error?.trim();
  return [
    {
      id: 'bmeg',
      label: 'Raw .bmeg',
      path: bmeg,
      saved: Boolean(bmeg) && !partial,
      note: !bmeg
        ? sentence(finalizeError || 'The raw file was not saved')
        : partial
          ? `Kept as a partial file (.bmeg.part) because it could not be finished.${finalizeError ? ` ${sentence(finalizeError)}` : ''}`
          : undefined
    },
    {
      id: 'csv',
      label: 'CSV',
      path: summary.csv_path ?? '',
      saved: Boolean(summary.csv_path) && !csvError,
      note: csvError ? sentence(csvError) : !summary.csv_path ? 'No CSV was written.' : undefined
    },
    {
      id: 'metadata',
      label: 'Metadata',
      path: summary.metadata_path ?? '',
      saved: Boolean(summary.metadata_path),
      note: summary.metadata_path ? undefined : sentence(finalizeError || 'The metadata file was not saved')
    }
  ];
}

const STOP_REASON_TEXT: Record<string, string> = {
  user: 'Stopped by you',
  timed_complete: 'The planned duration finished',
  application_close: 'Saved when the window was closed',
  disconnect: 'The connection was closed',
  storage_guard: 'Stopped to keep enough free disk space',
  fault: 'Stopped by an error'
};

/** How the recording ended, in plain words; the raw code stays in Advanced details. */
export function stopReasonText(reason: string | null | undefined): string {
  return (reason && Object.prototype.hasOwnProperty.call(STOP_REASON_TEXT, reason) ? STOP_REASON_TEXT[reason] : undefined)
    ?? 'Ended for a reason this version does not describe (see Advanced details)';
}

const plural = (count: number, one: string, many: string) => `${formatCount(count)} ${count === 1 ? one : many}`;

/** One line from the integrity counters: what a student should know about gaps. */
export function qualityLine(integrity: Integrity | undefined): { text: string; ok: boolean } {
  if (!integrity) return { text: 'Data quality was not reported.', ok: true };
  const damaged = integrity.crc_failures + integrity.invalid_frames;
  const missing = integrity.missing_sample_sequences;
  const disordered = integrity.duplicate_sample_sequences + integrity.out_of_order_sample_sequences;
  const overflows = integrity.firmware_overflows + integrity.host_channel_overflows;
  const disconnects = integrity.disconnect_events;
  const problems = [
    missing ? plural(missing, 'missing sample', 'missing samples') : '',
    damaged ? plural(damaged, 'damaged packet', 'damaged packets') : '',
    disordered ? plural(disordered, 'repeated or out-of-order sample', 'repeated or out-of-order samples') : '',
    overflows ? plural(overflows, 'buffer overflow', 'buffer overflows') : '',
    disconnects ? plural(disconnects, 'disconnection', 'disconnections') : ''
  ].filter(Boolean);
  return problems.length
    ? { text: `Check data quality: ${problems.join(' · ')}.`, ok: false }
    : { text: 'No missing, damaged, or out-of-order samples were detected.', ok: true };
}

function durationText(summary: Summary): string {
  const elapsed = summary.board_elapsed_seconds > 0 ? summary.board_elapsed_seconds : summary.host_elapsed_seconds;
  const recorded = formatClock(Number.isFinite(elapsed) ? Math.round(elapsed) : undefined);
  const planned = summary.duration?.mode === 'timed' ? summary.duration.seconds : undefined;
  return planned && planned > 0 ? `${recorded} of ${formatClock(planned)} planned` : recorded;
}

export function recordingResultView(summary: Summary, input: RecordingResultInput): RecordingResultView {
  const quality = qualityLine(summary.integrity);
  return {
    ...completion(summary, input.failureCategory),
    files: files(summary),
    duration: durationText(summary),
    samples: formatCount(summary.samples),
    rate: formatRate(summary.measured_rate_hz),
    markers: input.markerCount,
    quality: quality.text,
    qualityOk: quality.ok
  };
}
