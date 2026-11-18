<script lang="ts">
  import { untrack } from 'svelte';
  import ReadinessChecklist from './ReadinessChecklist.svelte';
  import { markerLabelError } from '$lib/markers';
  import type { ChecklistItem, ChecklistTarget } from '$lib/readiness';
  import type { RecordingChip, RecordingProgress } from '$lib/recording-status';

  /**
   * The sticky bar that keeps the recording in view, in one compact row:
   * state (as text), the SIMULATED tag, and then either the planned
   * duration, the Source choice, and Start; or, while recording, the clock
   * with progress, frames and measured rate, Add marker, and Stop. When
   * Start is unavailable it shows the readiness checklist instead of a
   * single blocking sentence. Display changes made elsewhere never pass
   * through here; every button calls a page handler that owns the
   * acquisition command.
   */
  type Props = {
    chip: RecordingChip;
    /** Source = Simulator (or the running session is simulated). */
    simulated: boolean;
    /** A recording is active (setup is read-only). */
    active: boolean;
    /** Markers can be added only while acquiring. */
    acquiring: boolean;
    stopping: boolean;
    progress: RecordingProgress;
    frames: string;
    rate: string;
    plannedDuration: string;
    source: 'hardware' | 'simulator';
    sourceLocked: boolean;
    /** The selected COM port, shown on the Hardware choice. */
    hardwarePort: string;
    startDisabled: boolean;
    stopDisabled: boolean;
    /** One line under the bar (Start in progress, or why Start is unavailable). */
    caption?: string;
    checklist?: ChecklistItem[];
    showClearConnection?: boolean;
    clearDisabled?: boolean;
    onStart: () => void;
    onStop: () => void;
    /** Resolves true when the marker was stored. */
    onAddMarker: (label: string) => Promise<boolean>;
    onClearConnection?: () => void;
    onChecklistSelect: (target: ChecklistTarget) => void;
    /** Called only for a choice the student made (hook for the source auto-select rule). */
    onSourceChange?: (source: 'hardware' | 'simulator') => void;
  };

  let {
    chip,
    simulated,
    active,
    acquiring,
    stopping,
    progress,
    frames,
    rate,
    plannedDuration,
    source = $bindable(),
    sourceLocked,
    hardwarePort,
    startDisabled,
    stopDisabled,
    caption = '',
    checklist,
    showClearConnection = false,
    clearDisabled = false,
    onStart,
    onStop,
    onAddMarker,
    onClearConnection = () => {},
    onChecklistSelect,
    onSourceChange
  }: Props = $props();

  let markerLabel = $state('');
  let markerPending = $state(false);
  const markerError = $derived(markerLabelError(markerLabel));

  // Start and Stop replace each other. When the one that had keyboard focus
  // disappears (focus falls back to the page), focus moves to the other, so
  // a keyboard user stays on the recording control. "Had focus" ends as soon
  // as the student's focus goes anywhere else: another control, a click
  // outside the two buttons, or a blur that leaves the button in place. A
  // button that is disabled (Saving…) or removed keeps it.
  let startButton = $state<HTMLButtonElement | null>(null);
  let stopButton = $state<HTMLButtonElement | null>(null);
  let recordingControlFocused = false;

  function isRecordingControl(target: EventTarget | null): boolean {
    return target instanceof Node && Boolean(startButton?.contains(target) || stopButton?.contains(target));
  }

  function focusIsNowhere(): boolean {
    return !document.activeElement || document.activeElement === document.body;
  }

  function trackFocus(event: FocusEvent) {
    recordingControlFocused = isRecordingControl(event.target);
  }

  function trackPointer(event: PointerEvent) {
    if (!isRecordingControl(event.target)) recordingControlFocused = false;
  }

  function trackBlur(event: FocusEvent) {
    const button = event.target;
    if (event.relatedTarget || !(button instanceof HTMLButtonElement) || !isRecordingControl(button)) return;
    // Checked once the update that may be removing or disabling the button is done.
    queueMicrotask(() => {
      if (button.isConnected && !button.disabled && focusIsNowhere()) recordingControlFocused = false;
    });
  }

  $effect(() => {
    const replacement = active ? stopButton : startButton;
    if (!replacement) return;
    untrack(() => {
      // The bar is always on screen, so moving focus within it never scrolls the page.
      if (recordingControlFocused && focusIsNowhere()) replacement.focus({ preventScroll: true });
    });
  });

  $effect(() => {
    document.addEventListener('focusin', trackFocus);
    document.addEventListener('focusout', trackBlur);
    document.addEventListener('pointerdown', trackPointer, true);
    return () => {
      document.removeEventListener('focusin', trackFocus);
      document.removeEventListener('focusout', trackBlur);
      document.removeEventListener('pointerdown', trackPointer, true);
    };
  });

  function chooseSource(next: 'hardware' | 'simulator') {
    if (sourceLocked || source === next) return;
    source = next;
    onSourceChange?.(next);
  }

  async function addMarker() {
    if (!acquiring || markerPending || markerError) return;
    markerPending = true;
    try {
      if (await onAddMarker(markerLabel.trim())) markerLabel = '';
    } finally {
      markerPending = false;
    }
  }

  /**
   * A held Enter (or Space) that pressed Start must not also press the Stop
   * that replaces it, nor may one held through Stop start a new recording.
   */
  function ignoreRepeatedKey(event: KeyboardEvent) {
    if (event.repeat && (event.key === 'Enter' || event.key === ' ')) event.preventDefault();
  }
</script>

<section class="recording-bar" class:recording={active} aria-labelledby="recording-bar-title">
  <h2 id="recording-bar-title" class="visually-hidden">Recording</h2>
  <div class="bar-row">
    <div class="status-group">
      <span class="chip {chip.tone}" data-testid="recording-state"><span class="chip-dot" aria-hidden="true"></span>{chip.label}</span>
      {#if simulated}<span class="sim-tag" title="Simulator data, not from the Arduino">SIMULATED</span>{/if}
    </div>

    {#if active}
      <div class="clock">
        <span class="readout"><span class="readout-label">Elapsed</span> <span class="readout-value">{progress.elapsedText}{progress.totalText ? ` / ${progress.totalText}` : ''}</span></span>
        {#if progress.fraction !== undefined}
          <progress max="1" value={progress.fraction} aria-label="Recording progress"></progress>
          <span class="readout"><span class="readout-value">{progress.remainingText}</span> <span class="readout-label">left</span></span>
        {/if}
      </div>
      <div class="counters">
        <span class="readout"><span class="readout-label">Frames</span> <span class="readout-value">{frames}</span></span>
        <span class="readout"><span class="readout-label">Rate</span> <span class="readout-value">{rate}</span></span>
      </div>
      <div class="marker-group">
        <label for="marker-label-input" class="visually-hidden">Marker label (optional)</label>
        <input
          id="marker-label-input"
          bind:value={markerLabel}
          maxlength="80"
          placeholder="Marker label"
          disabled={!acquiring}
          aria-invalid={Boolean(markerError)}
          aria-describedby={markerError ? 'marker-label-error' : undefined}
          onkeydown={(event) => { if (event.key === 'Enter') { event.preventDefault(); void addMarker(); } }}
        />
        <button type="button" class="secondary" onclick={() => void addMarker()} disabled={!acquiring || markerPending || Boolean(markerError)}>Add marker</button>
        {#if markerError}<span id="marker-label-error" class="marker-error">{markerError}</span>{/if}
      </div>
      <div class="actions">
        <button type="button" class="stop" bind:this={stopButton} onclick={onStop} onkeydown={ignoreRepeatedKey} disabled={stopDisabled}><span class="stop-icon" aria-hidden="true"></span>{stopping ? 'Saving…' : 'Stop recording'}</button>
      </div>
    {:else}
      <div class="clock">
        <span class="readout"><span class="readout-label">Duration</span> <span class="readout-value">{plannedDuration}</span></span>
      </div>
      <!-- Hidden while recording: the SIMULATED tag and the state chip already say which source is running. -->
      <fieldset class="source" disabled={sourceLocked}>
        <legend>Source</legend>
        <div class="segments">
          <label class:selected={source === 'hardware'}>
            <input type="radio" name="recording-source" value="hardware" checked={source === 'hardware'} onchange={() => chooseSource('hardware')} />
            Hardware{hardwarePort ? ` · ${hardwarePort}` : ''}
          </label>
          <label class:selected={source === 'simulator'}>
            <input type="radio" name="recording-source" value="simulator" checked={source === 'simulator'} onchange={() => chooseSource('simulator')} />
            Simulator (practice)
          </label>
        </div>
      </fieldset>
      <div class="actions">
        <button type="button" id="start-recording-button" class="start" bind:this={startButton} onclick={onStart} onkeydown={ignoreRepeatedKey} disabled={startDisabled}>Start recording</button>
        {#if showClearConnection}
          <button type="button" id="clear-connection-button" class="secondary" onclick={onClearConnection} disabled={clearDisabled}>Clear connection</button>
        {/if}
      </div>
    {/if}
  </div>

  {#if !active && (caption || checklist)}
    <div class="readiness-row">
      {#if caption}<p class="caption">{caption}</p>{/if}
      {#if checklist}<ReadinessChecklist items={checklist} onSelect={onChecklistSelect} />{/if}
    </div>
  {/if}
</section>

<style>
  .recording-bar {
    min-width: 0;
    padding: .4rem clamp(.6rem, 1.6vw, 1.1rem);
    border-bottom: 3px solid var(--wvu-gold, #EEAA00);
    background: var(--surface, #fff);
    box-shadow: 0 .2rem .6rem rgb(0 27 54 / 12%);
  }
  .recording-bar.recording { border-bottom-color: var(--danger, #9d2424); }
  .bar-row, .readiness-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: .4rem .7rem;
    min-width: 0;
  }
  .readiness-row { margin-top: .35rem; padding-top: .35rem; border-top: 1px solid var(--border, #d7dde2); gap: .3rem .6rem; }
  .status-group, .clock, .counters, .marker-group { display: flex; flex-wrap: nowrap; align-items: center; gap: .5rem; min-width: 0; }
  .counters { gap: .8rem; }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: .35rem;
    padding: .2rem .6rem;
    border: 2px solid;
    border-radius: 999px;
    font-weight: 800;
    white-space: nowrap;
  }
  .chip-dot { width: .6rem; height: .6rem; border-radius: 50%; background: currentColor; }
  .chip.ready { border-color: var(--ok, #176c33); color: var(--ok, #176c33); background: var(--ok-bg, #eaf6ee); }
  .chip.idle { border-color: var(--border-strong, #8493a0); color: var(--muted, #4b5965); background: var(--surface, #fff); }
  .chip.idle .chip-dot { background: transparent; border: 2px solid currentColor; }
  .chip.busy, .chip.saving { border-color: var(--wvu-navy, #002855); color: var(--wvu-navy, #002855); background: var(--surface-muted, #f2f5f8); }
  .chip.recording { border-color: var(--danger, #9d2424); color: #fff; background: var(--danger, #9d2424); }
  .chip.problem { border-color: var(--danger, #9d2424); color: var(--danger, #9d2424); background: var(--danger-bg, #fdf0f0); }
  .sim-tag {
    padding: .2rem .45rem;
    border-radius: .25rem;
    background: var(--wvu-navy, #002855);
    color: var(--wvu-gold, #EEAA00);
    font-size: .86rem;
    font-weight: 800;
    letter-spacing: .05em;
  }
  .readout { white-space: nowrap; }
  .readout-label { color: var(--muted, #4b5965); font-size: .74rem; font-weight: 700; text-transform: uppercase; letter-spacing: .03em; }
  .readout-value { font-variant-numeric: tabular-nums; font-weight: 700; }
  progress { width: clamp(3.5rem, 6vw, 7rem); height: .75rem; accent-color: var(--wvu-navy, #002855); }
  .source { display: flex; align-items: center; gap: .4rem; min-width: 0; margin: 0 0 0 auto; padding: 0; border: 0; }
  .source legend { float: left; margin-right: .4rem; padding: 0; color: var(--muted, #4b5965); font-size: .74rem; font-weight: 700; text-transform: uppercase; letter-spacing: .03em; }
  /* No overflow clipping, so the keyboard focus ring around a choice stays fully visible. */
  .segments { display: inline-flex; flex-wrap: nowrap; border: 1px solid var(--wvu-navy, #002855); border-radius: .3rem; }
  .segments label {
    position: relative;
    display: flex;
    align-items: center;
    min-height: 2.3rem;
    padding: .3rem .65rem;
    color: var(--wvu-navy, #002855);
    font-weight: 700;
    cursor: pointer;
    white-space: nowrap;
  }
  .segments label:first-child { border-radius: calc(.3rem - 1px) 0 0 calc(.3rem - 1px); }
  .segments label:last-child { border-radius: 0 calc(.3rem - 1px) calc(.3rem - 1px) 0; }
  .segments label + label { border-left: 1px solid var(--wvu-navy, #002855); }
  .segments label.selected { background: var(--wvu-navy, #002855); color: #fff; }
  .segments label.selected::before { content: '✓'; margin-right: .35rem; }
  .segments input { position: absolute; opacity: 0; inset: 0; margin: 0; cursor: pointer; }
  .segments label:has(input:focus-visible) { outline: 3px solid var(--focus-ring, #002855); outline-offset: 2px; box-shadow: 0 0 0 2px #fff; z-index: 1; }
  .source:disabled .segments label { cursor: not-allowed; opacity: .6; }
  .source:disabled .segments label.selected { opacity: .85; }
  .actions { display: flex; flex-wrap: wrap; gap: .5rem; margin-left: auto; }
  .source + .actions { margin-left: 0; }
  .actions button, .marker-group button {
    min-height: 2.4rem;
    padding: .4rem .9rem;
    border: 1px solid var(--wvu-navy, #002855);
    border-radius: .3rem;
    font: inherit;
    font-weight: 750;
    white-space: nowrap;
    cursor: pointer;
  }
  .start { background: var(--wvu-gold, #EEAA00); color: var(--ink, #17222e); font-size: 1.05rem; }
  .start:not(:disabled):hover { background: #d99a00; }
  .stop { display: inline-flex; align-items: center; gap: .5rem; border-color: #761a1a !important; background: var(--danger, #9d2424); color: #fff; font-size: 1.05rem; }
  .stop:not(:disabled):hover { background: #7f1b1b; }
  .stop-icon { width: .8rem; height: .8rem; background: currentColor; }
  .secondary { background: var(--surface, #fff); color: var(--wvu-navy, #002855); }
  .secondary:not(:disabled):hover { background: var(--wvu-navy, #002855); color: #fff; }
  button:disabled { cursor: not-allowed; opacity: .55; }
  .marker-group { flex: 1 1 14rem; max-width: 26rem; }
  .marker-group input {
    flex: 1 1 7rem;
    min-width: 7rem;
    min-height: 2.4rem;
    padding: .3rem .5rem;
    border: 1px solid var(--border-strong, #8493a0);
    border-radius: .25rem;
    font: inherit;
  }
  .marker-error { color: var(--danger, #9d2424); font-weight: 700; white-space: normal; }
  .caption { flex: 1 1 100%; margin: 0; color: var(--ink, #17222e); font-weight: 600; }
  .visually-hidden { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
  @media (max-width: 1100px) { .source { margin-left: 0; } }
</style>
