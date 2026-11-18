<script module lang="ts">
  /** Open dialogs, oldest first. Only the newest traps focus and handles Escape. */
  const openDialogs: HTMLElement[] = [];

  const FOCUSABLE = 'a[href], button, input, select, textarea, summary, [tabindex]:not([tabindex="-1"])';

  function focusableIn(dialog: HTMLElement): HTMLElement[] {
    return [...dialog.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((element) => {
      if (element.closest('[hidden], [inert]')) return false;
      // Content of a closed <details> cannot take focus; its summary can.
      const details = element.closest('details');
      if (details && !details.open && element.tagName !== 'SUMMARY') return false;
      try {
        return !element.matches(':disabled');
      } catch {
        return !(element as HTMLButtonElement).disabled;
      }
    });
  }

  function topDialog(): HTMLElement | undefined {
    return openDialogs[openDialogs.length - 1];
  }
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';

  /**
   * A real modal dialog: role=dialog with aria-modal, labelled by its title,
   * focus moved in on open, Tab kept inside, Escape closes it only when it
   * is closable, and focus returns to the control that opened it.
   *
   * The dialog is moved to <body> while open, so a collapsed setup card
   * around the component that renders it can never hide it.
   */
  type Props = {
    open: boolean;
    /** Id of the element that names the dialog (its heading). */
    labelledBy: string;
    describedBy?: string;
    /**
     * False for operations that must finish (for example a firmware upload):
     * Escape then does nothing and there is no implicit way to close.
     */
    closable?: boolean;
    onClose?: () => void;
    size?: 'narrow' | 'medium' | 'wide';
    children?: Snippet;
  };

  let { open, labelledBy, describedBy, closable = true, onClose = () => {}, size = 'medium', children }: Props = $props();

  function restoreFocus(opener: HTMLElement | null) {
    // After the closing update has settled, so a control re-enabled by the
    // same update can take focus again. If another dialog is still open,
    // focus goes back only to an opener inside it.
    window.setTimeout(() => {
      if (!opener?.isConnected) return;
      const top = topDialog();
      if (top && !top.contains(opener)) {
        if (!top.contains(document.activeElement)) top.focus();
        return;
      }
      opener.focus();
    }, 0);
  }

  function modal(backdrop: HTMLElement) {
    const dialog = backdrop.querySelector<HTMLElement>('[role="dialog"]');
    if (!dialog) return;
    const active = document.activeElement;
    const opener = active instanceof HTMLElement && active !== document.body ? active : null;
    document.body.appendChild(backdrop);
    openDialogs.push(dialog);
    if (!dialog.contains(document.activeElement)) dialog.focus();

    function onKeydown(event: KeyboardEvent) {
      if (topDialog() !== dialog || !dialog) return;
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        if (closable) onClose();
        return;
      }
      if (event.key !== 'Tab') return;
      const focusable = focusableIn(dialog);
      const current = document.activeElement;
      if (!focusable.length) {
        event.preventDefault();
        dialog.focus();
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (!dialog.contains(current)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      } else if (event.shiftKey && (current === first || current === dialog)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && current === last) {
        event.preventDefault();
        first.focus();
      }
    }

    function onFocusIn(event: FocusEvent) {
      if (topDialog() !== dialog || !dialog) return;
      const target = event.target;
      if (target instanceof Node && !dialog.contains(target)) dialog.focus();
    }

    /** Clicking the backdrop must not move focus out of the dialog. */
    function onBackdropMousedown(event: MouseEvent) {
      if (event.target === backdrop) event.preventDefault();
    }

    document.addEventListener('keydown', onKeydown, true);
    document.addEventListener('focusin', onFocusIn, true);
    backdrop.addEventListener('mousedown', onBackdropMousedown);
    return () => {
      document.removeEventListener('keydown', onKeydown, true);
      document.removeEventListener('focusin', onFocusIn, true);
      backdrop.removeEventListener('mousedown', onBackdropMousedown);
      const index = openDialogs.indexOf(dialog);
      if (index >= 0) openDialogs.splice(index, 1);
      // Also covers the owner being destroyed while the dialog is open.
      backdrop.remove();
      restoreFocus(opener);
    };
  }

</script>

{#if open}
  <div class="modal-backdrop" {@attach modal}>
    <div class="modal modal-{size}" role="dialog" aria-modal="true" aria-labelledby={labelledBy} aria-describedby={describedBy} tabindex="-1">
      {@render children?.()}
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: grid;
    place-items: center;
    padding: 1rem;
    background: rgb(12 27 42 / 52%);
  }
  .modal {
    max-height: min(92vh, 70rem);
    overflow: auto;
    padding: clamp(1rem, 2.5vw, 1.5rem);
    border: 2px solid var(--wvu-navy, #002855);
    border-radius: .45rem;
    background: var(--surface, #fff);
    color: var(--ink, #17222e);
    box-shadow: 0 1rem 3rem rgb(0 0 0 / 25%);
    overflow-wrap: anywhere;
  }
  .modal:focus { outline: none; }
  .modal-narrow { width: min(100%, 36rem); }
  .modal-medium { width: min(100%, 46rem); }
  .modal-wide { width: min(100%, 96rem); }
</style>
