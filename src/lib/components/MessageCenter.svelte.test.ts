// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import MessageCenter from './MessageCenter.svelte';
import { faultBanner, type NextAction, type ProblemBanner } from '$lib/messages';

function render(initial: { banner?: ProblemBanner; info?: string; actionDisabled?: boolean } = {}) {
  const actions: NextAction[] = [];
  let state = $state({ ...initial });
  const component = mount(MessageCenter, {
    target: document.body,
    props: {
      get banner() { return state.banner; },
      get info() { return state.info; },
      get actionDisabled() { return state.actionDisabled; },
      onAction: (action: NextAction) => { actions.push(action); }
    }
  });
  flushSync();
  return { component, actions, state };
}

const noData = faultBanner({ faultOrigin: 'recording', failureCategory: 'no_data_timeout', simulatorSession: false, recordingRan: true, source: 'hardware' });

afterEach(() => {
  document.body.replaceChildren();
});

describe('MessageCenter', () => {
  it('has exactly one polite live region, present even while empty', () => {
    const center = render();
    const regions = document.querySelectorAll('[aria-live]');
    expect(regions).toHaveLength(1);
    expect(regions[0].getAttribute('aria-live')).toBe('polite');
    expect(regions[0].textContent).toBe('');
    center.state.info = 'Marker added at 12.3 s.';
    flushSync();
    expect(regions[0].textContent).toBe('Marker added at 12.3 s.');
    unmount(center.component);
  });

  it('shows one banner in plain language with its one next action', () => {
    const center = render({ banner: noData });
    const banner = document.querySelector('[data-problem]');
    expect(banner?.textContent).toContain('The Arduino stopped sending samples');
    expect(banner?.textContent).not.toMatch(/no_data_timeout/);
    const action = document.querySelector<HTMLButtonElement>('#message-action-button');
    expect(action?.textContent?.trim()).toBe('Verify Firmware');
    action?.click();
    expect(center.actions).toEqual(['verify_firmware']);
    unmount(center.component);
  });

  it('announces a banner as an alert only when it first appears', () => {
    const center = render();
    expect(document.querySelector('[role="alert"]')).toBeNull();
    center.state.banner = noData;
    flushSync();
    expect(document.querySelector('[role="alert"]')?.getAttribute('data-problem')).toBe(noData.id);
    unmount(center.component);
  });

  it('disables the action while it cannot run', () => {
    const center = render({ banner: noData, actionDisabled: true });
    expect(document.querySelector<HTMLButtonElement>('#message-action-button')?.disabled).toBe(true);
    unmount(center.component);
  });

  it('offers a quieter second action beside the first one', () => {
    const silent = faultBanner({ faultOrigin: 'verification', failureCategory: 'port_open_no_bytes', simulatorSession: false, recordingRan: false, source: 'hardware', port: 'COM5' });
    const center = render({ banner: silent });
    expect(document.querySelector('#message-action-button')?.textContent?.trim()).toBe('Restore WVU Firmware');
    const secondary = document.querySelector<HTMLButtonElement>('#message-secondary-button');
    expect(secondary?.textContent?.trim()).toBe('Verify again');
    secondary?.click();
    expect(center.actions).toEqual(['verify_firmware']);
    unmount(center.component);
  });

  it('shows a button after the information line only while there is a message', () => {
    let clicked = 0;
    const component = mount(MessageCenter, {
      target: document.body,
      props: { info: 'Recording saved.', infoAction: { label: 'Show Last recording', onClick: () => { clicked += 1; } }, onAction: () => {} }
    });
    flushSync();
    const button = document.querySelector<HTMLButtonElement>('#message-info-button');
    expect(button?.textContent).toBe('Show Last recording');
    // The button is not part of the announced text.
    expect(document.querySelector('[aria-live]')?.textContent).toBe('Recording saved.');
    button?.click();
    expect(clicked).toBe(1);
    unmount(component);
  });

  it('shows two lines of a long banner, with More for the rest, and keeps the whole text in the page', () => {
    // happy-dom has no layout: the text reports three lines in a two-line box.
    const scrollHeight = vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockImplementation(function (this: HTMLElement) {
      return this.classList.contains('banner-text') ? 66 : 0;
    });
    const clientHeight = vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockImplementation(function (this: HTMLElement) {
      return this.classList.contains('banner-text') ? 44 : 0;
    });
    try {
      const long = { id: 'tools:failed', severity: 'error' as const, headline: 'The Arduino tools need attention', sentence: 'The Arduino tools included with the application were not found. Reinstall the application or contact your instructor.' };
      const center = render({ banner: long });
      const text = document.querySelector('.banner-text')!;
      expect(text.classList.contains('clamp')).toBe(true);
      expect(text.textContent).toContain('contact your instructor.');
      const more = document.querySelector<HTMLButtonElement>('.banner-more')!;
      expect(more.textContent?.trim()).toBe('More');
      expect(more.getAttribute('aria-expanded')).toBe('false');
      expect(more.getAttribute('aria-controls')).toBe(text.id);
      more.click();
      flushSync();
      expect(text.classList.contains('clamp')).toBe(false);
      expect(more.textContent?.trim()).toBe('Less');
      expect(more.getAttribute('aria-expanded')).toBe('true');
      // A new problem starts collapsed again.
      center.state.banner = { ...long, id: 'folder:unavailable' };
      flushSync();
      expect(document.querySelector('.banner-text')?.classList.contains('clamp')).toBe(true);
      expect(document.querySelector('.banner-more')?.textContent?.trim()).toBe('More');
      unmount(center.component);
    } finally {
      scrollHeight.mockRestore();
      clientHeight.mockRestore();
    }
  });

  it('offers no More for a banner that fits', () => {
    const center = render({ banner: noData });
    expect(document.querySelector('.banner-more')).toBeNull();
    unmount(center.component);
  });

  it('shows a banner without an action when none applies', () => {
    const center = render({ banner: { id: 'catalog:problem', severity: 'warning', headline: 'Some labs could not be loaded', sentence: 'Ask your instructor.' } });
    expect(document.querySelector('#message-action-button')).toBeNull();
    expect(document.body.textContent).toContain('Attention: Some labs could not be loaded.');
    unmount(center.component);
  });
});
