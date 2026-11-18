// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import RecordingResultCard from './RecordingResultCard.svelte';
import { recordingResultView } from '$lib/recording-result';
import { emptyIntegrity, type Summary } from '$lib/session-types';

const summary: Summary = {
  state: 'Disconnected', samples: 4200, packets: 420, measured_rate_hz: 100, board_elapsed_seconds: 42.2, host_elapsed_seconds: 42.5,
  bmeg_path: 'C:\\BMEG 420L\\run.bmeg', csv_path: '', metadata_path: 'C:\\BMEG 420L\\run.metadata.json',
  recording_status: 'faulted', duration: { mode: 'timed', seconds: 60 }, stop_reason: 'fault', completion_status: 'incomplete',
  integrity: { ...emptyIntegrity, missing_sample_sequences: 2 }, csv_error: 'could not write the CSV'
};

function render() {
  const calls = { reveal: [] as string[], feedback: [] as string[] };
  const component = mount(RecordingResultCard, {
    target: document.body,
    props: {
      view: recordingResultView(summary, { markerCount: 3, failureCategory: 'no_data_timeout' }),
      stopReason: 'fault',
      onReveal: (path: string) => { calls.reveal.push(path); },
      onFeedback: (message: string) => { calls.feedback.push(message); }
    }
  });
  flushSync();
  return { component, calls };
}

const rows = () => [...document.querySelectorAll('.result-file')];

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

describe('RecordingResultCard', () => {
  it('lists each file as saved or not saved in words and explains the early stop', () => {
    const card = render();
    expect(document.body.textContent).toContain('Stopped early: the Arduino stopped sending samples.');
    expect(rows().map((row) => row.querySelector('.file-status')?.textContent?.replace(/\s+/g, ' ').trim())).toEqual([
      '✓ Raw .bmeg Saved', '✗ CSV Not saved', '✓ Metadata Saved'
    ]);
    expect(rows()[1].textContent).toContain('could not write the CSV.');
    // A file that was not written has nothing to copy or show.
    expect(rows()[1].querySelector('button')).toBeNull();
    const facts = document.querySelector('.result-facts')?.textContent?.replace(/\s+/g, ' ');
    expect(facts).toContain('Duration0:42 of 1:00 planned');
    expect(facts).toContain('Markers3');
    expect(document.querySelector('.quality')?.textContent).toContain('2 missing samples');
    // The stop reason in plain words; the raw code is only in Advanced details.
    const details = document.querySelector('.result-details')?.textContent ?? '';
    expect(details).toContain('How it ended: Stopped by an error.');
    expect(details).not.toMatch(/fault/);
    unmount(card.component);
  });

  it('copies a path and asks the page to show the file in its folder', async () => {
    const writeText = vi.fn(async () => undefined);
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const card = render();
    const [copy, reveal] = [...rows()[0].querySelectorAll('button')];
    expect(copy.getAttribute('aria-label')).toBe('Copy path: Raw .bmeg');
    expect(reveal.getAttribute('aria-label')).toBe('Show in folder: Raw .bmeg');
    copy.click();
    await new Promise((done) => setTimeout(done, 0));
    expect(writeText).toHaveBeenCalledWith('C:\\BMEG 420L\\run.bmeg');
    expect(card.calls.feedback).toEqual(['Raw .bmeg path copied.']);
    reveal.click();
    expect(card.calls.reveal).toEqual(['C:\\BMEG 420L\\run.bmeg']);
    unmount(card.component);
  });

  it('falls back to selectable text when the clipboard is unavailable', async () => {
    vi.stubGlobal('navigator', {});
    const card = render();
    rows()[2].querySelector('button')?.click();
    await new Promise((done) => setTimeout(done, 0));
    flushSync();
    await new Promise((done) => setTimeout(done, 0));
    const field = rows()[2].querySelector<HTMLInputElement>('.copy-fallback input');
    expect(field?.value).toBe('C:\\BMEG 420L\\run.metadata.json');
    expect(field?.readOnly).toBe(true);
    expect(document.activeElement).toBe(field);
    unmount(card.component);
  });
});
