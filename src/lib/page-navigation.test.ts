// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { revealIfHidden, scrollToAndFocus, visibleHeight } from './page-navigation';

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe('page navigation', () => {
  it('measures how much of a box is on screen below the sticky stack', () => {
    expect(visibleHeight({ top: 356, bottom: 691 }, 146, 690)).toBe(334);
    expect(visibleHeight({ top: 100, bottom: 400 }, 146, 690)).toBe(254);
    expect(visibleHeight({ top: 900, bottom: 1200 }, 146, 690)).toBe(0);
  });

  it('scrolls a container up only when too little of its target is visible', () => {
    const container = document.createElement('section');
    const target = document.createElement('div');
    container.append(target);
    document.body.append(container);
    const scroll = vi.fn();
    container.scrollIntoView = scroll;
    vi.spyOn(target, 'getBoundingClientRect').mockReturnValue({ top: 300, bottom: 600, height: 300 } as DOMRect);
    vi.stubGlobal('innerHeight', 680);
    expect(revealIfHidden(container, target, 60, 280)).toBe(false);
    vi.spyOn(target, 'getBoundingClientRect').mockReturnValue({ top: 560, bottom: 860, height: 300 } as DOMRect);
    expect(revealIfHidden(container, target, 60, 280)).toBe(true);
    expect(scroll).toHaveBeenCalledWith(expect.objectContaining({ block: 'start' }));
    vi.unstubAllGlobals();
  });

  it('focuses the destination, or the fallback when the destination cannot take focus', () => {
    const disabled = document.createElement('button');
    disabled.disabled = true;
    const fallback = document.createElement('button');
    const enabled = document.createElement('button');
    document.body.append(disabled, fallback, enabled);
    scrollToAndFocus(enabled, { fallback });
    expect(document.activeElement).toBe(enabled);
    scrollToAndFocus(disabled, { fallback });
    expect(document.activeElement).toBe(fallback);
  });

  it('focuses a control in the sticky stack without scrolling the page', () => {
    const stack = document.createElement('div');
    stack.style.position = 'sticky';
    const inStack = document.createElement('button');
    stack.append(inStack);
    const inPage = document.createElement('button');
    document.body.append(stack, inPage);
    const stackScroll = vi.fn();
    const pageScroll = vi.fn();
    inStack.scrollIntoView = stackScroll;
    inPage.scrollIntoView = pageScroll;
    const focus = vi.spyOn(inStack, 'focus');
    scrollToAndFocus(inStack);
    expect(stackScroll).not.toHaveBeenCalled();
    expect(focus).toHaveBeenCalledWith({ preventScroll: true });
    expect(document.activeElement).toBe(inStack);
    scrollToAndFocus(inPage);
    expect(pageScroll).toHaveBeenCalledWith(expect.objectContaining({ block: 'center' }));
    expect(document.activeElement).toBe(inPage);
  });
});
