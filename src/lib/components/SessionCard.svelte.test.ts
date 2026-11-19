// @vitest-environment happy-dom
/**
 * The Session setup card's read-only lab fields. The rate field holds the
 * rate and its unit only, so it fits its column; an LED-sequenced lab's
 * "(one per LED state)" is in its tooltip (and the card's summary).
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import { labRateLabel, labRateText } from '$lib/lab-authoring';
import type { LabProfile } from '$lib/labs';
import SessionCard from './SessionCard.svelte';

const lab = (file: string) => JSON.parse(readFileSync(resolve(process.cwd(), 'profiles', file), 'utf8')) as LabProfile;

let component: ReturnType<typeof mount> | undefined;

afterEach(() => {
  if (component) unmount(component);
  component = undefined;
  document.body.innerHTML = '';
});

function render(profile: LabProfile) {
  if (component) unmount(component);
  document.body.innerHTML = '';
  component = mount(SessionCard, {
    target: document.body,
    props: {
      outputDirectory: '',
      note: '',
      profile,
      channels: profile.acquisition.channels ?? [],
      rateLabel: labRateLabel(profile.acquisition),
      rateText: labRateText(profile.acquisition),
      effectiveOutputFolder: 'C:\\Project',
      locked: false
    }
  });
  flushSync();
}

function field(label: string): HTMLInputElement | undefined {
  return [...document.querySelectorAll('label')]
    .find((candidate) => candidate.textContent?.trim().startsWith(label))
    ?.querySelector('input') ?? undefined;
}

describe('Session setup card', () => {
  it('shows an LED-sequenced lab’s record rate briefly, with the full text as its tooltip', () => {
    render(lab('pulseox_tx_rx_course_capture.profile.json'));
    expect(field('Analog pins')?.value).toBe('A0, A1');
    expect(field('Record rate')?.value).toBe('1000 records/s');
    expect(field('Record rate')?.title).toBe('1000 records/s (one per LED state). Protected by the selected profile');
  });

  it('keeps other labs’ rate text and tooltip', () => {
    render(lab('op_amp_course_capture.profile.json'));
    expect(field('Sample rate')?.value).toBe('100 frames/s');
    expect(field('Sample rate')?.title).toBe('Protected by the selected profile');
    render(lab('legacy/pulseox_tx_rx_course_capture_1.0.0.profile.json'));
    expect(field('Analog pins')?.value).toBe('TX A0; RX A1');
    expect(field('Cycle rate')?.value).toBe('250 cycles/s');
  });
});
