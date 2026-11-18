<script lang="ts">
  /**
   * Display-only plot controls: Pause/Resume, the time-window chips, and the
   * custom time-window field. Every control changes only what the plots
   * show; none sends an acquisition command or touches the raw recording.
   */
  type Props = {
    paused: boolean;
    /** A recording is running (the paused badge says it continues). */
    recording: boolean;
    windowSeconds: number;
    /** The custom field as typed (it may be temporarily invalid). */
    windowInput: string;
    presets: number[];
    onTogglePause: () => void;
    onChoosePreset: (seconds: number) => void;
    onWindowInput: (value: string) => void;
    onWindowCommit: () => void;
  };

  let { paused, recording, windowSeconds, windowInput, presets, onTogglePause, onChoosePreset, onWindowInput, onWindowCommit }: Props = $props();
</script>

<div class="plot-toolbar">
  <button type="button" id="plot-pause-button" class="pause-button" class:paused onclick={onTogglePause}>
    <span class="pause-icon" aria-hidden="true">{paused ? '▶' : '❚❚'}</span>{paused ? 'Resume display' : 'Pause display'}
  </button>
  {#if paused}<span class="paused-badge">{recording ? 'Paused — recording continues' : 'Display paused'}</span>{/if}
  <div class="plot-time-window" role="group" aria-labelledby="plot-time-window-label">
    <span id="plot-time-window-label" class="window-label">Plot time window</span>
    <span class="window-chips">
      {#each presets as preset (preset)}
        <button type="button" class="window-chip" aria-pressed={windowSeconds === preset} onclick={() => onChoosePreset(preset)}>{preset} s</button>
      {/each}
    </span>
    <label for="plot-time-window">Custom (s)</label>
    <input
      id="plot-time-window"
      type="number"
      min="0.5"
      max="30"
      step="0.5"
      inputmode="decimal"
      aria-describedby="plot-time-window-help"
      value={windowInput}
      oninput={(event) => onWindowInput(event.currentTarget.value)}
      onblur={onWindowCommit}
      onkeydown={(event) => { if (event.key === 'Enter') { event.preventDefault(); onWindowCommit(); event.currentTarget.blur(); } }}
    />
    <span aria-hidden="true">s</span>
    <span id="plot-time-window-help" class="help">0.5–30 s. Applies to all live plots{paused ? ' when you resume' : ''}. Recording is unaffected.</span>
  </div>
</div>

<style>
  .plot-toolbar { display: flex; flex-wrap: wrap; align-items: center; gap: .5rem 1rem; margin: .5rem 0; }
  button {
    min-height: 2.5rem;
    border: 1px solid var(--wvu-navy, #002855);
    border-radius: .3rem;
    background: var(--surface, #fff);
    color: var(--wvu-navy, #002855);
    padding: .5rem .75rem;
    font: inherit;
    font-weight: 650;
    cursor: pointer;
  }
  button:hover { background: var(--wvu-navy, #002855); color: #fff; }
  .pause-button { display: inline-flex; align-items: center; gap: .45rem; }
  .pause-button.paused { background: var(--wvu-navy, #002855); color: #fff; }
  .pause-icon { font-size: .78rem; letter-spacing: -.1em; }
  .paused-badge { padding: .3rem .7rem; border: 2px solid var(--warn, #9b6700); border-radius: 999px; background: var(--warn-bg, #fff8e8); color: #6b4700; font-weight: 800; }
  /* Shrinks beside Pause and wraps inside itself (the help moves to a second line) before it drops below Pause. */
  .plot-time-window { display: flex; flex: 1 1 30rem; flex-wrap: wrap; align-items: center; gap: .45rem; min-width: 0; }
  .window-label { font-weight: 700; }
  .window-chips { display: inline-flex; flex-wrap: wrap; gap: .3rem; }
  .window-chip { min-height: 2.2rem; padding: .25rem .65rem; border-radius: 999px; text-align: center; font-variant-numeric: tabular-nums; }
  /* The chosen chip is marked with a check, not by colour alone. */
  .window-chip[aria-pressed="true"] { background: var(--wvu-navy, #002855); color: #fff; }
  .window-chip[aria-pressed="true"]::before { content: '✓ '; }
  label { font-weight: 600; }
  input {
    width: 5.5rem;
    min-height: 2.4rem;
    padding: .35rem .5rem;
    border: 1px solid var(--border-strong, #8493a0);
    border-radius: .25rem;
    background: var(--surface, #fff);
    font: inherit;
  }
  .help { flex: 1 1 16rem; margin: 0; color: var(--muted, #4b5965); }
</style>
