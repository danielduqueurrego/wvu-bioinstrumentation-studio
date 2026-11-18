// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import RecordingBar from './RecordingBar.svelte';
import { readinessChecklist, type ChecklistTarget } from '$lib/readiness';
import { recordingChip, recordingProgress } from '$lib/recording-status';

type BarState = {
  active: boolean;
  acquiring: boolean;
  source: 'hardware' | 'simulator';
  checklist?: ReturnType<typeof readinessChecklist>;
};

function render(initial: Partial<BarState> = {}) {
  const calls = { start: 0, stop: 0, markers: [] as string[], targets: [] as ChecklistTarget[], sources: [] as string[] };
  let state = $state<BarState>({ active: false, acquiring: false, source: 'simulator', ...initial });
  const component = mount(RecordingBar, {
    target: document.body,
    props: {
      get chip() {
        return recordingChip({ sessionState: state.acquiring ? 'Acquiring' : 'Disconnected', startInFlight: false, canStart: !state.checklist, source: state.source, boardSelected: false });
      },
      get simulated() { return state.source === 'simulator'; },
      get active() { return state.active; },
      get acquiring() { return state.acquiring; },
      stopping: false,
      get progress() { return recordingProgress({ mode: 'timed', seconds: 60 }, 42, 18); },
      frames: '4,200',
      rate: '100.0 Hz',
      plannedDuration: '1:00 timed',
      get source() { return state.source; },
      set source(value: 'hardware' | 'simulator') { state.source = value; },
      get sourceLocked() { return state.active; },
      hardwarePort: 'COM5',
      startDisabled: false,
      stopDisabled: false,
      get checklist() { return state.checklist; },
      onStart: () => { calls.start += 1; },
      onStop: () => { calls.stop += 1; },
      onAddMarker: async (label: string) => { calls.markers.push(label); return true; },
      onChecklistSelect: (target: ChecklistTarget) => { calls.targets.push(target); },
      onSourceChange: (source: string) => { calls.sources.push(source); }
    }
  });
  flushSync();
  return { component, calls, state };
}

const button = (text: string) => [...document.querySelectorAll('button')].find((candidate) => candidate.textContent?.trim() === text);

afterEach(() => {
  document.body.replaceChildren();
});

describe('RecordingBar', () => {
  it('shows Start, the planned duration, and a SIMULATED tag before a simulator recording', () => {
    const bar = render();
    expect(button('Start recording')).toBeDefined();
    expect(document.body.textContent).toContain('1:00 timed');
    expect(document.body.textContent).toContain('SIMULATED');
    expect(document.body.textContent).not.toContain('left');
    button('Start recording')?.click();
    expect(bar.calls.start).toBe(1);
    unmount(bar.component);
  });

  it('lets the student choose the Source, reports it as their choice, and hides it while recording', () => {
    const bar = render();
    const hardware = document.querySelector<HTMLInputElement>('input[value="hardware"]')!;
    expect(hardware.closest('label')?.textContent).toContain('Hardware · COM5');
    hardware.click();
    flushSync();
    expect(bar.state.source).toBe('hardware');
    expect(bar.calls.sources).toEqual(['hardware']);
    expect(document.body.textContent).not.toContain('SIMULATED');
    bar.state.active = true;
    flushSync();
    // The state chip and SIMULATED tag say which source is running.
    expect(document.querySelector('fieldset.source')).toBeNull();
    unmount(bar.component);
  });

  it('shows the readiness checklist; a failing item asks to move to its control', () => {
    const checklist = readinessChecklist({ source: 'hardware', selectedPort: '', firmwareReady: false, blockingFault: false, labName: 'ECG', acknowledgementSatisfied: true, projectFolder: '' });
    const bar = render({ source: 'hardware', checklist });
    const items = [...document.querySelectorAll('.readiness li')].map((item) => item.textContent?.replace(/\s+/g, ' ').trim());
    expect(items[0]).toContain('Board (needs attention): select a board');
    expect(items[2]).toContain('Lab (ready): ECG');
    // A narrow window folds the passing items into one chip; it is not read out twice.
    const readyCount = document.querySelector('.readiness .ready-count');
    expect(readyCount?.getAttribute('aria-hidden')).toBe('true');
    expect(readyCount?.textContent?.replace(/\s+/g, ' ').trim()).toBe(`✓ ${checklist.filter((item) => item.status !== 'fail').length} ready`);
    const folder = [...document.querySelectorAll<HTMLButtonElement>('.readiness button')].find((candidate) => candidate.textContent?.includes('Folder'));
    folder?.click();
    expect(bar.calls.targets).toEqual(['project-folder']);
    expect(document.querySelector('[data-testid="recording-state"]')?.textContent).toBe('Disconnected');
    unmount(bar.component);
  });

  it('shows Stop, the clock with progress, and Add marker with a label while recording', async () => {
    const bar = render({ active: true, acquiring: true, source: 'hardware' });
    expect(button('Start recording')).toBeUndefined();
    expect(button('Stop recording')).toBeDefined();
    expect(document.body.textContent).toContain('0:42 / 1:00');
    expect(document.body.textContent).toContain('0:18 left');
    expect(document.querySelector('progress')?.getAttribute('aria-label')).toBe('Recording progress');
    const label = document.querySelector<HTMLInputElement>('#marker-label-input')!;
    label.value = '  baseline ';
    label.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    button('Add marker')?.click();
    await new Promise((done) => setTimeout(done, 0));
    flushSync();
    expect(bar.calls.markers).toEqual(['baseline']);
    expect(label.value).toBe('');
    button('Stop recording')?.click();
    expect(bar.calls.stop).toBe(1);
    unmount(bar.component);
  });

  it('moves keyboard focus between Start and Stop when one replaces the other', () => {
    const bar = render();
    const start = button('Start recording')!;
    start.focus();
    expect(document.activeElement).toBe(start);
    bar.state.active = true;
    bar.state.acquiring = true;
    flushSync();
    expect(document.activeElement).toBe(button('Stop recording'));
    bar.state.active = false;
    bar.state.acquiring = false;
    flushSync();
    expect(document.activeElement).toBe(button('Start recording'));
    unmount(bar.component);
  });

  it('moves focus between Start and Stop without scrolling the page', () => {
    const bar = render();
    const start = button('Start recording')!;
    start.focus();
    const focus = vi.spyOn(HTMLElement.prototype, 'focus');
    bar.state.active = true;
    flushSync();
    expect(focus).toHaveBeenCalledTimes(1);
    expect(focus.mock.contexts[0]).toBe(button('Stop recording'));
    expect(focus).toHaveBeenCalledWith({ preventScroll: true });
    focus.mockRestore();
    unmount(bar.component);
  });

  it('keeps Stop disabled while saving and still hands focus to Start when it goes', async () => {
    const bar = render({ active: true, acquiring: true });
    const stop = button('Stop recording')!;
    stop.focus();
    // Saving: the focused Stop is disabled, and the page may drop its focus.
    stop.disabled = true;
    stop.blur();
    await Promise.resolve();
    bar.state.active = false;
    bar.state.acquiring = false;
    flushSync();
    expect(document.activeElement).toBe(button('Start recording'));
    unmount(bar.component);
  });

  it('does not move focus once the student has left the button (a blur or a click elsewhere)', async () => {
    const bar = render({ active: true, acquiring: true });
    button('Stop recording')!.focus();
    button('Stop recording')!.blur();
    await Promise.resolve();
    // The timed recording ends by itself later.
    bar.state.active = false;
    bar.state.acquiring = false;
    flushSync();
    expect(document.activeElement).toBe(document.body);

    // Start → Stop again, then a click on a part of the page that takes no focus.
    button('Start recording')!.focus();
    bar.state.active = true;
    bar.state.acquiring = true;
    flushSync();
    expect(document.activeElement).toBe(button('Stop recording'));
    const plot = document.createElement('div');
    document.body.append(plot);
    plot.dispatchEvent(new Event('pointerdown', { bubbles: true }));
    button('Stop recording')!.blur();
    bar.state.active = false;
    bar.state.acquiring = false;
    flushSync();
    expect(document.activeElement).toBe(document.body);
    unmount(bar.component);
  });

  it('leaves focus alone when it is elsewhere as the recording starts', () => {
    const bar = render();
    const outside = document.createElement('input');
    document.body.append(outside);
    outside.focus();
    bar.state.active = true;
    flushSync();
    expect(document.activeElement).toBe(outside);
    unmount(bar.component);
  });

  it('ignores a held Enter key on Stop, so one long press cannot start and stop a recording', () => {
    const bar = render({ active: true, acquiring: true });
    const stop = button('Stop recording')!;
    const held = new KeyboardEvent('keydown', { key: 'Enter', repeat: true, bubbles: true, cancelable: true });
    stop.dispatchEvent(held);
    expect(held.defaultPrevented).toBe(true);
    const fresh = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
    stop.dispatchEvent(fresh);
    expect(fresh.defaultPrevented).toBe(false);
    unmount(bar.component);
  });

  it('ignores a held Enter key on Start, so one held through Stop cannot start a new recording', () => {
    const bar = render();
    const start = button('Start recording')!;
    for (const key of ['Enter', ' ']) {
      const held = new KeyboardEvent('keydown', { key, repeat: true, bubbles: true, cancelable: true });
      start.dispatchEvent(held);
      expect(held.defaultPrevented, key).toBe(true);
    }
    const fresh = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
    start.dispatchEvent(fresh);
    expect(fresh.defaultPrevented).toBe(false);
    unmount(bar.component);
  });

  it('refuses a marker label the backend would refuse', () => {
    const bar = render({ active: true, acquiring: true });
    const label = document.querySelector<HTMLInputElement>('#marker-label-input')!;
    label.value = 'é'.repeat(41);
    label.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    expect(label.getAttribute('aria-invalid')).toBe('true');
    expect(button('Add marker')?.disabled).toBe(true);
    expect(document.getElementById('marker-label-error')?.textContent).toMatch(/80 characters/);
    unmount(bar.component);
  });
});
