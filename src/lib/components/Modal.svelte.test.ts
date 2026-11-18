// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest';
import { createRawSnippet, flushSync, mount, unmount } from 'svelte';
import Modal from './Modal.svelte';

const content = createRawSnippet(() => ({
  render: () => '<div><h2 id="test-title">Test dialog</h2><button id="first">First</button><button id="last">Last</button></div>'
}));

type Props = { open: boolean; closable?: boolean; onClose?: () => void };

function setup(initial: Props) {
  const opener = document.createElement('button');
  opener.id = 'opener';
  opener.textContent = 'Open';
  const host = document.createElement('div');
  document.body.append(opener, host);
  opener.focus();
  let props = $state({ ...initial });
  const component = mount(Modal, {
    target: host,
    props: {
      get open() { return props.open; },
      get closable() { return props.closable; },
      get onClose() { return props.onClose; },
      labelledBy: 'test-title',
      children: content
    }
  });
  flushSync();
  return {
    component,
    opener,
    host,
    set: (next: Partial<Props>) => { props = { ...props, ...next }; flushSync(); },
    dialog: () => document.querySelector<HTMLElement>('[role="dialog"]')
  };
}

function key(key: string, shiftKey = false) {
  (document.activeElement ?? document.body).dispatchEvent(new KeyboardEvent('keydown', { key, shiftKey, bubbles: true, cancelable: true }));
  flushSync();
}

const wait = () => new Promise((done) => setTimeout(done, 5));

afterEach(() => {
  document.body.replaceChildren();
});

describe('Modal', () => {
  it('is a labelled modal dialog that takes focus when it opens', () => {
    const modal = setup({ open: true });
    const dialog = modal.dialog();
    expect(dialog?.getAttribute('aria-modal')).toBe('true');
    expect(dialog?.getAttribute('aria-labelledby')).toBe('test-title');
    expect(document.activeElement).toBe(dialog);
    unmount(modal.component);
  });

  it('is moved to <body>, so a hidden (collapsed) ancestor cannot hide it', () => {
    const modal = setup({ open: false });
    modal.host.hidden = true;
    modal.set({ open: true });
    expect(modal.dialog()?.closest('[hidden]')).toBeNull();
    expect(modal.dialog()?.parentElement?.parentElement).toBe(document.body);
    unmount(modal.component);
  });

  it('keeps Tab inside the dialog', () => {
    const modal = setup({ open: true });
    key('Tab', true);
    expect(document.activeElement?.id).toBe('last');
    key('Tab');
    expect(document.activeElement?.id).toBe('first');
    key('Tab', true);
    expect(document.activeElement?.id).toBe('last');
    unmount(modal.component);
  });

  it('closes on Escape when closable and returns focus to the opener', async () => {
    let closed = 0;
    const modal = setup({ open: true, onClose: () => { closed += 1; } });
    key('Escape');
    expect(closed).toBe(1);
    modal.set({ open: false });
    expect(modal.dialog()).toBeNull();
    await wait();
    expect(document.activeElement).toBe(modal.opener);
    unmount(modal.component);
  });

  it('ignores Escape when it must not close (a firmware upload)', () => {
    let closed = 0;
    const modal = setup({ open: true, closable: false, onClose: () => { closed += 1; } });
    key('Escape');
    expect(closed).toBe(0);
    expect(modal.dialog()).not.toBeNull();
    unmount(modal.component);
  });

  it('removes the dialog when its owner is destroyed while it is open', () => {
    const modal = setup({ open: true });
    unmount(modal.component);
    flushSync();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });

  it('lets only the newest of two open dialogs handle Escape', () => {
    let firstClosed = 0;
    let secondClosed = 0;
    const first = setup({ open: true, onClose: () => { firstClosed += 1; } });
    const second = setup({ open: true, onClose: () => { secondClosed += 1; } });
    key('Escape');
    expect([firstClosed, secondClosed]).toEqual([0, 1]);
    unmount(second.component);
    flushSync();
    key('Escape');
    expect([firstClosed, secondClosed]).toEqual([1, 1]);
    unmount(first.component);
  });
});
