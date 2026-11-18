<script lang="ts">
  import { NEXT_ACTION_LABELS, type NextAction, type ProblemBanner } from '$lib/messages';

  /**
   * At most one problem banner with one next-action button (and, for some
   * problems, a quieter second one), plus a short information line for
   * action feedback ("Marker added at 12.3 s.").
   *
   * The information line is the page's only aria-live region (polite). A
   * banner is announced once, as an alert, when it first appears; it then
   * stays on screen without repeating itself.
   */
  type Props = {
    banner?: ProblemBanner;
    /** The banner's action cannot run right now (for example, busy). */
    actionDisabled?: boolean;
    /** The banner's secondary action cannot run right now. */
    secondaryDisabled?: boolean;
    onAction: (action: NextAction) => void;
    info?: string;
    /** A button after the information line, for example "Show Last recording". */
    infoAction?: { label: string; onClick: () => void };
  };

  let { banner, actionDisabled = false, secondaryDisabled = false, onAction, info = '', infoAction }: Props = $props();

  // A new problem is announced once; the alert role is then dropped so later
  // re-renders of the same banner are not read out again.
  let announcedId = '';
  let alerting = $state(false);
  let alertTimer: number | undefined;

  // A long banner shows two lines (it sits in the sticky stack above the
  // plots); More shows the rest. The whole text is always in the page, so
  // the alert and screen readers read all of it.
  let expanded = $state(false);
  let clamped = $state(false);
  let bannerText = $state<HTMLElement | null>(null);

  // Before the DOM update, so a new banner is inserted with role=alert.
  $effect.pre(() => {
    const id = banner?.id ?? '';
    if (id && id !== announcedId) {
      alerting = true;
      expanded = false;
      if (alertTimer !== undefined) window.clearTimeout(alertTimer);
      alertTimer = window.setTimeout(() => { alerting = false; }, 5000);
    }
    announcedId = id;
  });

  $effect(() => {
    const text = bannerText;
    // Measured again when the words change under the same banner.
    void banner?.headline;
    void banner?.sentence;
    if (!text) {
      clamped = false;
      return;
    }
    const measure = () => {
      if (!expanded) clamped = text.scrollHeight > text.clientHeight + 1;
    };
    measure();
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : undefined;
    observer?.observe(text);
    return () => observer?.disconnect();
  });

  $effect(() => () => {
    if (alertTimer !== undefined) window.clearTimeout(alertTimer);
  });
</script>

<div class="message-center">
  {#if banner}
    {#key banner.id}
      <div class="banner {banner.severity}" role={alerting ? 'alert' : undefined} data-problem={banner.id}>
        <span class="banner-icon" aria-hidden="true">{banner.severity === 'error' ? '!' : 'i'}</span>
        <p class="banner-text" class:clamp={!expanded} id="banner-text" bind:this={bannerText}>
          <strong>{banner.severity === 'error' ? 'Problem:' : 'Attention:'} {banner.headline}.</strong>
          <span>{banner.sentence}</span>
        </p>
        {#if clamped || expanded}
          <button type="button" class="banner-more" aria-expanded={expanded} aria-controls="banner-text" onclick={() => { expanded = !expanded; }}>
            {expanded ? 'Less' : 'More'}
          </button>
        {/if}
        {#if banner.nextAction || banner.secondary}
          <span class="banner-actions">
            {#if banner.nextAction}
              {@const action = banner.nextAction}
              <button type="button" id="message-action-button" class="banner-action" disabled={actionDisabled} onclick={() => onAction(action)}>
                {NEXT_ACTION_LABELS[action]}
              </button>
            {/if}
            {#if banner.secondary}
              {@const secondary = banner.secondary}
              <button type="button" id="message-secondary-button" class="banner-secondary" disabled={secondaryDisabled} onclick={() => onAction(secondary.action)}>
                {secondary.label}
              </button>
            {/if}
          </span>
        {/if}
      </div>
    {/key}
  {/if}
  <div class="info-row" class:empty={!info}>
    <p class="info-line" aria-live="polite" aria-atomic="true">{info}</p>
    {#if info && infoAction}
      <button type="button" id="message-info-button" class="info-action" onclick={infoAction.onClick}>{infoAction.label}</button>
    {/if}
  </div>
</div>

<style>
  .message-center { display: grid; gap: .3rem; min-width: 0; }
  .banner {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: .35rem .7rem;
    min-width: 0;
    padding: .3rem .6rem;
    border: 1px solid;
    border-left-width: 6px;
    border-radius: .3rem;
  }
  .banner.error { border-color: var(--danger, #9d2424); background: var(--danger-bg, #fdf0f0); }
  .banner.warning { border-color: var(--warn, #9b6700); background: var(--warn-bg, #fff8e8); }
  .banner-icon {
    flex: 0 0 auto;
    display: grid;
    place-items: center;
    width: 1.4rem;
    height: 1.4rem;
    border-radius: 50%;
    color: #fff;
    font-weight: 800;
  }
  .error .banner-icon { background: var(--danger, #9d2424); }
  .warning .banner-icon { background: var(--warn, #9b6700); }
  .banner-text { flex: 1 1 20rem; min-width: 0; margin: 0; color: var(--ink, #17222e); line-height: 1.35; overflow-wrap: anywhere; }
  .banner-text.clamp { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow: hidden; }
  .banner-more {
    flex: 0 0 auto;
    min-height: 2rem;
    padding: .2rem .4rem;
    border: 0;
    background: transparent;
    color: var(--wvu-navy, #002855);
    font: inherit;
    font-weight: 700;
    text-decoration: underline;
    cursor: pointer;
  }
  .banner-actions { display: inline-flex; flex-wrap: wrap; gap: .4rem; }
  .banner-action, .banner-secondary, .info-action {
    flex: 0 0 auto;
    min-height: 2.25rem;
    padding: .35rem .75rem;
    border: 1px solid var(--wvu-navy, #002855);
    border-radius: .3rem;
    font: inherit;
    font-weight: 700;
    cursor: pointer;
  }
  .banner-action { background: var(--wvu-navy, #002855); color: #fff; }
  .banner-action:not(:disabled):hover { background: #001b3a; }
  .banner-secondary, .info-action { background: var(--surface, #fff); color: var(--wvu-navy, #002855); }
  .banner-secondary:not(:disabled):hover, .info-action:hover { background: var(--wvu-navy, #002855); color: #fff; }
  .banner-action:disabled, .banner-secondary:disabled { cursor: not-allowed; opacity: .55; }
  .info-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: .25rem .75rem;
    min-width: 0;
    padding: .15rem .6rem;
    border-left: 4px solid var(--wvu-navy, #002855);
    background: var(--surface, #fff);
  }
  .info-line { flex: 1 1 20rem; min-width: 0; margin: 0; color: var(--ink, #17222e); overflow-wrap: anywhere; }
  .info-action { min-height: 2rem; padding: .2rem .65rem; }
  /* Kept in the accessibility tree while empty so later messages are announced. */
  .info-row.empty { padding: 0; border: 0; }
</style>
