<script lang="ts">
  import type { Snippet } from 'svelte';

  /**
   * A collapsible setup card. The heading is a disclosure button; the
   * collapsed card shows a one-line summary. The body stays in the page
   * while collapsed (only hidden), so bound values and component state are
   * kept and nothing is re-created when it opens again.
   */
  type Props = {
    id: string;
    title: string;
    /** Shown while collapsed. */
    summary?: string;
    open?: boolean;
    /** Read-only while a recording runs (the controls are disabled). */
    locked?: boolean;
    onToggle?: (open: boolean) => void;
    children?: Snippet;
  };

  let { id, title, summary = '', open = $bindable(true), locked = false, onToggle, children }: Props = $props();

  function toggle() {
    open = !open;
    onToggle?.(open);
  }
</script>

<section class="setup-card" class:collapsed={!open} id="{id}-card" aria-labelledby="{id}-title">
  <h3 class="card-heading">
    <button type="button" id="{id}-toggle" class="card-toggle" aria-expanded={open} aria-controls="{id}-body" onclick={toggle}>
      <span class="chevron" aria-hidden="true"></span>
      <span class="card-title" id="{id}-title">{title}</span>
      {#if !open && summary}<span class="card-summary">{summary}</span>{/if}
      {#if locked && open}<span class="card-lock">Read-only while recording</span>{/if}
    </button>
  </h3>
  <div class="card-body" id="{id}-body" hidden={!open}>
    {@render children?.()}
  </div>
</section>

<style>
  .setup-card {
    min-width: 0;
    margin: .6rem 0;
    background: var(--surface, #fff);
    border: 1px solid var(--border, #d7dde2);
    border-radius: .35rem;
  }
  .card-heading { margin: 0; font-size: 1rem; }
  .card-toggle {
    width: 100%;
    min-height: 2.75rem;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: .35rem .75rem;
    padding: .6rem clamp(.75rem, 2vw, 1rem);
    border: 0;
    border-radius: .35rem;
    background: transparent;
    color: var(--wvu-navy, #002855);
    font: inherit;
    font-weight: 750;
    text-align: left;
    cursor: pointer;
  }
  .card-toggle:hover { background: var(--surface-muted, #f2f5f8); }
  .chevron {
    flex: 0 0 auto;
    width: .55rem;
    height: .55rem;
    border-right: 2px solid currentColor;
    border-bottom: 2px solid currentColor;
    transform: rotate(45deg);
    margin: 0 .2rem .2rem;
    transition: transform .15s ease;
  }
  .collapsed .chevron { transform: rotate(-45deg); margin-bottom: 0; }
  .card-title { font-size: 1.05rem; }
  .card-summary {
    flex: 1 1 16rem;
    min-width: 0;
    overflow: hidden;
    color: var(--muted, #4b5965);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card-lock {
    margin-left: auto;
    padding: .15rem .5rem;
    border: 1px solid var(--border-strong, #8493a0);
    border-radius: 999px;
    color: var(--muted, #4b5965);
    font-size: .8rem;
    font-weight: 700;
  }
  .card-body { min-width: 0; padding: 0 clamp(.75rem, 2vw, 1rem) clamp(.75rem, 2vw, 1rem); }
  @media (prefers-reduced-motion: reduce) { .chevron { transition: none; } }
</style>
