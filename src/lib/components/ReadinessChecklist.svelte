<script lang="ts">
  import type { ChecklistItem, ChecklistTarget } from '$lib/readiness';

  /**
   * Board · Firmware · Lab · Folder · Source, shown beside Start when a
   * recording cannot begin. A failing item is a button that takes the
   * student to the control that fixes it. Status is given by a symbol and
   * words, never by colour alone.
   */
  type Props = {
    items: ChecklistItem[];
    onSelect: (target: ChecklistTarget) => void;
  };

  let { items, onSelect }: Props = $props();

  const STATUS_WORD = { ok: 'ready', fail: 'needs attention', skip: 'not needed' } as const;
  const SYMBOL = { ok: '✓', fail: '✗', skip: '–' } as const;

  // In a narrow window the passing items fold into one "✓ 3 ready" chip so
  // the list stays on one row above the plots. They stay in the list for
  // screen readers, and the chip is not read twice.
  const hasFailure = $derived(items.some((item) => item.status === 'fail'));
  const readyCount = $derived(items.filter((item) => item.status !== 'fail').length);
</script>

<ul class="readiness" class:has-failure={hasFailure} aria-label="Before you can record">
  {#each items as item (item.id)}
    <li class="item {item.status}">
      {#if item.status === 'fail' && item.target}
        {@const target = item.target}
        <button type="button" onclick={() => onSelect(target)}>
          <span class="symbol" aria-hidden="true">{SYMBOL[item.status]}</span>
          <span class="label">{item.label}<span class="visually-hidden">{` (${STATUS_WORD[item.status]})`}</span>:</span>
          <span class="detail">{item.detail}</span>
        </button>
      {:else}
        <span class="symbol" aria-hidden="true">{SYMBOL[item.status]}</span>
        <span class="label">{item.label}<span class="visually-hidden">{` (${STATUS_WORD[item.status]})`}</span>:</span>
        <span class="detail">{item.detail}</span>
      {/if}
    </li>
  {/each}
  {#if hasFailure && readyCount}
    <li class="item ready-count" aria-hidden="true"><span class="symbol">✓</span> {readyCount} ready</li>
  {/if}
</ul>

<style>
  .readiness {
    display: flex;
    flex-wrap: wrap;
    gap: .35rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .item {
    display: flex;
    align-items: center;
    gap: .3rem;
    min-width: 0;
    padding: .2rem .55rem;
    border: 1px solid var(--border, #d7dde2);
    border-radius: 999px;
    background: var(--surface, #fff);
    color: var(--ink, #17222e);
    font-size: .88rem;
  }
  .item.ok .symbol { color: var(--ok, #176c33); }
  .item.skip { color: var(--muted, #4b5965); }
  .item.fail { padding: 0; border-color: var(--warn, #9b6700); background: var(--warn-bg, #fff8e8); }
  .item.fail button {
    display: flex;
    align-items: center;
    gap: .3rem;
    min-height: 2rem;
    padding: .2rem .6rem;
    border: 0;
    border-radius: 999px;
    background: transparent;
    color: var(--ink, #17222e);
    font: inherit;
    cursor: pointer;
    text-align: left;
  }
  .item.fail button:hover .detail { text-decoration: underline; }
  .item.fail .symbol { color: var(--danger, #9d2424); }
  .symbol { font-weight: 800; }
  .label { font-weight: 700; }
  .detail { overflow-wrap: anywhere; }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  .ready-count { display: none; color: var(--muted, #4b5965); }
  .ready-count .symbol { color: var(--ok, #176c33); }
  @media (max-width: 1100px) {
    .has-failure .item.ok, .has-failure .item.skip {
      position: absolute;
      width: 1px;
      height: 1px;
      padding: 0;
      overflow: hidden;
      clip: rect(0 0 0 0);
      white-space: nowrap;
      border: 0;
    }
    .has-failure .ready-count { display: flex; }
  }
</style>
