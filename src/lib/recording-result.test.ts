import { describe, expect, it } from 'vitest';
import { qualityLine, recordingResultView, stopReasonText } from './recording-result';
import { emptyIntegrity, type Summary } from './session-types';

const summary = (overrides: Partial<Summary> = {}): Summary => ({
  state: 'Disconnected', samples: 6000, packets: 600, measured_rate_hz: 99.96,
  board_elapsed_seconds: 59.98, host_elapsed_seconds: 60.2,
  bmeg_path: 'C:\\BMEG 420L\\Trial1\\run.bmeg',
  csv_path: 'C:\\BMEG 420L\\Trial1\\run.csv',
  metadata_path: 'C:\\BMEG 420L\\Trial1\\run.metadata.json',
  recording_status: 'complete', duration: { mode: 'timed', seconds: 60 }, stop_reason: 'timed_complete',
  completion_status: 'complete', integrity: emptyIntegrity,
  ...overrides
});

describe('Finished-recording card', () => {
  it('reports a complete timed recording with every file saved', () => {
    const view = recordingResultView(summary(), { markerCount: 2 });
    expect(view).toMatchObject({ complete: true, headline: 'Recording complete', detail: 'The planned duration finished.' });
    expect(view.files.map((file) => [file.label, file.saved])).toEqual([['Raw .bmeg', true], ['CSV', true], ['Metadata', true]]);
    // 59.98 s is shown as the whole minute it was, not 0:59.
    expect(view.duration).toBe('1:00 of 1:00 planned');
    expect(view).toMatchObject({ samples: '6,000', rate: '100.0 Hz', markers: 2, qualityOk: true });
  });

  it('says in plain words why a recording stopped early', () => {
    const view = recordingResultView(summary({ completion_status: 'incomplete', stop_reason: 'fault', board_elapsed_seconds: 42.4 }), { markerCount: 0, failureCategory: 'no_data_timeout' });
    expect(view.complete).toBe(false);
    expect(view.headline).toBe('Stopped early: the Arduino stopped sending samples');
    expect(view.duration).toBe('0:42 of 1:00 planned');
    expect(recordingResultView(summary({ completion_status: 'incomplete', stop_reason: 'fault' }), { markerCount: 0 }).headline)
      .toBe('Stopped early: an error stopped the recording');
    expect(recordingResultView(summary({ completion_status: 'incomplete', stop_reason: 'storage_guard' }), { markerCount: 0 }).headline)
      .toBe('Stopped early: the disk was almost full');
    expect(recordingResultView(summary({ stop_reason: 'user', duration: { mode: 'until_stopped' } }), { markerCount: 0 }))
      .toMatchObject({ headline: 'Recording complete', detail: 'Stopped by you.', duration: '1:00' });
  });

  it('marks a missing CSV with its reason and keeps the raw file and metadata', () => {
    const view = recordingResultView(summary({ csv_path: '', csv_error: 'could not write the CSV: disk full' }), { markerCount: 0 });
    const csv = view.files.find((file) => file.id === 'csv')!;
    expect(csv).toMatchObject({ saved: false, note: 'could not write the CSV: disk full.' });
    expect(view.files.find((file) => file.id === 'bmeg')?.saved).toBe(true);
  });

  it('marks a raw file kept as .bmeg.part and a missing metadata file', () => {
    const view = recordingResultView(summary({
      bmeg_path: 'C:\\BMEG 420L\\run.bmeg.part', metadata_path: '', csv_path: '',
      finalize_error: 'could not rename the raw file', completion_status: 'incomplete', stop_reason: 'user'
    }), { markerCount: 0 });
    const [bmeg, , metadata] = view.files;
    expect(bmeg).toMatchObject({ saved: false, path: 'C:\\BMEG 420L\\run.bmeg.part' });
    expect(bmeg.note).toContain('partial file');
    expect(metadata).toMatchObject({ saved: false, path: '', note: 'could not rename the raw file.' });
    expect(view.headline).toBe('Recording ended with problems');
  });

  it('summarizes the integrity counters in one line', () => {
    expect(qualityLine(emptyIntegrity)).toEqual({ text: 'No missing, damaged, or out-of-order samples were detected.', ok: true });
    const line = qualityLine({ ...emptyIntegrity, missing_sample_sequences: 3, crc_failures: 1, host_channel_overflows: 1 });
    expect(line.ok).toBe(false);
    expect(line.text).toBe('Check data quality: 3 missing samples · 1 damaged packet · 1 buffer overflow.');
    expect(qualityLine(undefined).ok).toBe(true);
    // Skipped sampling deadlines leave no sequence gap; the board flags them.
    expect(qualityLine({ ...emptyIntegrity, overrun_flagged_records: 10 })).toEqual({
      text: 'Check data quality: 10 samples flagged for a timing overrun.',
      ok: false
    });
  });

  it('describes every stop reason in plain words, never as a raw code', () => {
    expect(stopReasonText('user')).toBe('Stopped by you');
    expect(stopReasonText('timed_complete')).toBe('The planned duration finished');
    for (const reason of ['user', 'timed_complete', 'application_close', 'disconnect', 'storage_guard', 'fault', 'something_new', undefined]) {
      expect(stopReasonText(reason), String(reason)).not.toMatch(/[a-z]+_[a-z]+/);
    }
  });
});
