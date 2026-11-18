// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import FirmwareJobProgress from './FirmwareJobProgress.svelte';
import type { FirmwareJob } from '$lib/session-types';

function render(initial: FirmwareJob) {
  const calls = { cancel: 0 };
  let job = $state<FirmwareJob>(initial);
  const component = mount(FirmwareJobProgress, {
    target: document.body,
    props: {
      get job() { return job; },
      onCancel: () => { calls.cancel += 1; }
    }
  });
  flushSync();
  return {
    component,
    calls,
    setJob(next: FirmwareJob) {
      job = next;
      flushSync();
    }
  };
}

const restoreJob = (overrides: Partial<FirmwareJob> = {}): FirmwareJob => ({
  id: 7, kind: 'restore_reference', stage: 'compiling', active: true,
  message: 'Compiling the controlled WVU reference firmware.', original_port: 'COM5',
  ...overrides
});

const stepTexts = () => [...document.querySelectorAll('.stepper li')].map((item) => item.textContent?.replace(/\s+/g, ' ').trim());
const button = (text: string) => [...document.querySelectorAll('button')].find((candidate) => candidate.textContent?.trim() === text);

afterEach(() => {
  document.body.replaceChildren();
});

describe('FirmwareJobProgress', () => {
  it('shows the six steps with the current one marked, and Cancel before the upload', () => {
    const view = render(restoreJob());
    expect(document.getElementById('operation-title')?.textContent).toBe('Restoring WVU Firmware…');
    expect(stepTexts()).toHaveLength(6);
    expect(document.querySelector('[aria-current="step"]')?.textContent).toContain('Compile');
    expect(stepTexts()[0]).toContain('done');
    expect(document.getElementById('operation-stage')?.textContent).toBe('Compiling the controlled WVU reference firmware.');
    button('Cancel restore')?.click();
    expect(view.calls.cancel).toBe(1);
    expect(document.body.textContent).not.toContain('Press the RESET button twice quickly');
    unmount(view.component);
  });

  it('asks for the double reset with a countdown while the board waits for it', () => {
    const view = render(restoreJob({
      stage: 'waiting_for_bootloader',
      message: 'Press the board Reset button twice.',
      manual_reset_deadline_utc: new Date(Date.now() + 90_000).toISOString()
    }));
    const card = document.querySelector('[aria-labelledby="double-reset-title"]');
    expect(card?.querySelector('h3')?.textContent).toBe('Press the RESET button twice quickly');
    expect(card?.textContent).toMatch(/Time left: 1:(30|29)/);
    expect(document.querySelector('[aria-current="step"]')?.textContent).toContain('Switch board to upload mode');
    // Waiting for the reset can still be canceled: nothing has been written.
    expect(button('Cancel restore')).toBeDefined();

    // The CLI's own transition after the reset has no deadline: no card.
    view.setJob(restoreJob({ stage: 'waiting_for_bootloader', manual_reset_deadline_utc: null }));
    expect(document.querySelector('[aria-labelledby="double-reset-title"]')).toBeNull();
    unmount(view.component);
  });

  it('removes Cancel and warns not to unplug while the firmware is written', () => {
    const view = render(restoreJob({ stage: 'uploading', message: 'Writing the WVU firmware to the board. Do not unplug the board.' }));
    expect(button('Cancel restore')).toBeUndefined();
    expect(document.querySelector('.write-notice')?.textContent).toContain('Writing firmware — do not unplug the board');
    expect(document.querySelector('[aria-current="step"]')?.textContent).toContain('Upload');
    view.setJob(restoreJob({ stage: 'verifying_firmware' }));
    expect(button('Cancel restore')).toBeUndefined();
    expect(stepTexts().slice(0, 5).every((text) => text?.includes('done'))).toBe(true);
    unmount(view.component);
  });
});
