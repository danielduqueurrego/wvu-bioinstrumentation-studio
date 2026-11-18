/**
 * Moving the student to a place on the page: scrolling (smoothly unless
 * reduced motion is preferred) and focusing. The page's scroll-padding-top
 * (the measured sticky-stack height) keeps the destination from ending up
 * under the sticky recording bar. Display-only; nothing here touches the
 * recording.
 */

export function prefersReducedMotion(): boolean {
  try {
    return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  } catch {
    return true;
  }
}

function scrollBehavior(): ScrollBehavior {
  return prefersReducedMotion() ? 'auto' : 'smooth';
}

/**
 * Whether `element` is inside a sticky or fixed container (the recording
 * bar and message center), which keeps it on screen at any scroll position.
 */
export function inPinnedContainer(element: HTMLElement): boolean {
  for (let node: HTMLElement | null = element; node; node = node.parentElement) {
    const position = window.getComputedStyle(node).position;
    if (position === 'sticky' || position === 'fixed') return true;
  }
  return false;
}

/**
 * Scrolls `destination` into view and focuses it. When it cannot take focus
 * (for example a disabled button), `fallback` takes it instead. A
 * destination in the sticky stack is already on screen: scrolling the page
 * would only move the content under it, so it is focused where it is.
 */
export function scrollToAndFocus(destination: HTMLElement, options: { block?: ScrollLogicalPosition; fallback?: HTMLElement | null } = {}) {
  if (!inPinnedContainer(destination)) destination.scrollIntoView?.({ block: options.block ?? 'center', behavior: scrollBehavior() });
  destination.focus({ preventScroll: true });
  if (document.activeElement !== destination && options.fallback) options.fallback.focus({ preventScroll: true });
}

/** How many pixels of a box are on screen between `topInset` (the sticky stack's bottom) and the viewport's bottom. */
export function visibleHeight(box: { top: number; bottom: number }, topInset: number, viewportHeight: number): number {
  return Math.max(0, Math.min(box.bottom, viewportHeight) - Math.max(box.top, topInset));
}

/**
 * Scrolls `container` to the top of the viewport (below the sticky stack)
 * when less than `minimum` pixels of `target` are on screen. Returns
 * whether it scrolled. Focus is not moved.
 */
export function revealIfHidden(container: HTMLElement, target: HTMLElement, topInset: number, minimum: number): boolean {
  const box = target.getBoundingClientRect();
  if (visibleHeight(box, topInset, window.innerHeight) >= Math.min(minimum, box.height)) return false;
  container.scrollIntoView?.({ block: 'start', behavior: scrollBehavior() });
  return true;
}
