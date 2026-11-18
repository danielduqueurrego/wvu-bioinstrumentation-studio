<script lang="ts">
  import { untrack } from 'svelte';
  import LivePlot from './LivePlot.svelte';
  import PlotToolbar from './PlotToolbar.svelte';
  import { unitsForGroup, type DisplayUnit, type RecordingCalibration } from '$lib/calibration';
  import { MAX_PLOT_TIME_WINDOW_SECONDS, normalizePlotTimeWindow, previewPlotTimeWindow } from '$lib/live-display';
  import type { LabDigitalOutput } from '$lib/labs';
  import type { PlacedMarker } from '$lib/markers';
  import {
    assignChannelToPlot,
    onePlotPerSignal,
    overlayAll,
    setPlotGroupCount,
    setTraceVisibility,
    type PlotChannel,
    type PlotGroup,
    type VisibleChannelMap
  } from '$lib/multichannel';
  import type { Point } from '$lib/session-types';

  /**
   * Live signals: the bounded display of the recording, its plot controls,
   * and the plot arrangement. Display-only throughout: the time window,
   * Pause, trace visibility, units, and grouping change only what is drawn
   * and never send an acquisition command or touch the raw recording.
   */
  type Channel = PlotChannel;
  type RenderedGroup = { id: string; slot: number; channelIds: string[] };

  type Props = {
    pulseox: boolean;
    /** "Teaching use only" shown with the plots (while they lead the page). */
    teachingNotice?: boolean;
    arduino: string;
    storage: string;
    digitalOutputs?: LabDigitalOutput[];
    outputMask?: number | null;
    /** Display-only pause: the plots keep their frame; recording continues. */
    paused: boolean;
    recording: boolean;
    /** One display time extent shared by every plot (a bounded display query only). */
    windowSeconds: number;
    channels: Channel[];
    traceVisibility: VisibleChannelMap;
    plotGroups: PlotGroup[];
    renderedGroups: RenderedGroup[];
    visibleCount: number;
    simulated: boolean;
    samples: Point[];
    channelUnits: Record<string, DisplayUnit>;
    calibration: RecordingCalibration;
    adcBits: number;
    displayRevision: number;
    timeOriginUs?: number;
    hardwareSource: boolean;
    markers: PlacedMarker[];
    displayOptionsOpen: boolean;
    arrangementSummary: string;
    /** The time window changed or Pause was released: fetch the newest display data. */
    onDisplayRefresh: () => void;
    onStatus: (message: string) => void;
    onPlotError: (stage: string, detail: string) => void;
  };

  let {
    pulseox,
    teachingNotice = false,
    arduino,
    storage,
    digitalOutputs = [],
    outputMask,
    paused = $bindable(),
    recording,
    windowSeconds = $bindable(),
    channels,
    traceVisibility = $bindable(),
    plotGroups = $bindable(),
    renderedGroups,
    visibleCount,
    simulated,
    samples,
    channelUnits,
    calibration,
    adcBits,
    displayRevision,
    timeOriginUs,
    hardwareSource,
    markers,
    displayOptionsOpen = $bindable(),
    arrangementSummary,
    onDisplayRefresh,
    onStatus,
    onPlotError
  }: Props = $props();

  const PRESETS = [1, 2, 5, 10, MAX_PLOT_TIME_WINDOW_SECONDS];
  // The custom field as typed; it may be briefly invalid while editing.
  let windowInput = $state(String(untrack(() => windowSeconds)));

  function changeWindow(seconds: number) {
    if (seconds === windowSeconds) return;
    windowSeconds = seconds;
    onDisplayRefresh();
  }

  /** Keeps the last valid extent while the field is briefly cleared; valid edits apply at once. */
  function editWindow(value: string) {
    windowInput = value;
    changeWindow(previewPlotTimeWindow(value, windowSeconds));
  }

  function commitWindow() {
    const next = normalizePlotTimeWindow(windowInput, windowSeconds);
    windowInput = String(next);
    changeWindow(next);
  }

  function choosePreset(seconds: number) {
    windowInput = String(seconds);
    changeWindow(seconds);
  }

  /** Pause freezes what the plots show; recording and polling continue. Resume jumps to the newest frames. */
  function togglePause() {
    paused = !paused;
    if (paused) {
      onStatus(recording ? 'Plots paused. Recording continues; select Resume display to see the newest data.' : 'Plots paused.');
      return;
    }
    onStatus('Plots resumed at the newest data.');
    onDisplayRefresh();
  }

  const BIT_FOR_PIN: Record<string, number> = { D4: 1, D5: 2, D6: 4 };

  const outputsText = $derived(digitalOutputs
    .map((output) => `${output.pin} ${output.label} — ${output.behavior === 'acquisition_sequenced' ? 'Sequenced' : (outputMask ?? 0) & BIT_FOR_PIN[output.pin] ? 'HIGH' : 'LOW'}`)
    .join('; '));

  // Samples at either ADC limit, per channel (display-only clipping hint).
  const railCounts = $derived.by(() => {
    const fullScale = Math.pow(2, adcBits) - 1;
    const counts: Record<string, number> = {};
    channels.forEach((channel, index) => {
      counts[channel.id] = samples.reduce((count, sample) => {
        const value = sample.values[index];
        return count + (value === 0 || value === fullScale ? 1 : 0);
      }, 0);
    });
    return counts;
  });

  const labelOf = (id: string) => channels.find((channel) => channel.id === id)?.label ?? id;

  function railText(channelIds: string[]): string {
    const total = channelIds.reduce((count, id) => count + (railCounts[id] ?? 0), 0);
    return total ? `; ${total} samples at ADC limit (clipping)` : '';
  }
</script>

<!-- `studio-form` only where its form defaults are wanted: PlotToolbar and LivePlot keep their own styles. -->
<section class="panel plot-panel" id="live-panel" aria-labelledby="live-title">
  <div class="plot-heading studio-form">
    <h2 id="live-title">{pulseox ? 'Live signals — raw pulse-ox' : 'Live signals'}</h2>
    <section class="metric-grid" aria-label="Recording details">
      <span><strong>Arduino</strong>{arduino}</span>
      <span><strong>Storage</strong>{storage}</span>
      {#if digitalOutputs.length}<span><strong>Lab outputs</strong>{outputsText}</span>{/if}
    </section>
  </div>
  {#if teachingNotice}
    <p class="teaching-line"><strong>Teaching use only — not a medical device.</strong> Follow BMEG 420L lab instructions and instructor safety procedures. Raw counts and Arduino-input volts are preserved; this app makes no diagnostic or clinical decision.</p>
  {/if}
  <!-- Display-only controls: none of them sends an acquisition command. -->
  <PlotToolbar {paused} {recording} {windowSeconds} {windowInput} presets={PRESETS} onTogglePause={togglePause} onChoosePreset={choosePreset} onWindowInput={editWindow} onWindowCommit={commitWindow} />
  <fieldset class="trace-controls">
    <legend>Visible signals</legend>
    {#each channels as channel}<label class="choice"><input type="checkbox" checked={traceVisibility[channel.id] !== false} onchange={(event) => traceVisibility = setTraceVisibility(traceVisibility, channel.id, event.currentTarget.checked)} /> {channel.label}</label>{/each}
    <span class="display-note">Display only: each plot autoscales to its unit; recording is unaffected.</span>
  </fieldset>
  {#if pulseox}<p class="help">TX and RX plots show each raw RED, DARK 1, IR, and DARK 2 measurement. The eight raw values are recorded unchanged.</p>{/if}
  {#if !visibleCount}
    <p class="help">No traces are selected for display. Recording and export continue for every profile field.</p>
  {:else}
    <!-- The plot area. The SIMULATED DATA watermark covers every plot while the data are simulated. -->
    <div class="stacked-plots" class:simulated role="group" aria-label={simulated ? 'Synchronized plot groups, simulated data' : 'Synchronized plot groups'}>
      {#each renderedGroups as group (group.id)}
        <div class="stacked-plot" role="group" aria-label={`Plot ${group.slot}: ${group.channelIds.map(labelOf).join(', ')}`}>
          {#if simulated}<div class="sim-watermark" aria-hidden="true"><span>SIMULATED DATA</span></div>{/if}
          <div class="stacked-plot-heading"><strong>Plot {group.slot}: {group.channelIds.map(labelOf).join(' + ')}</strong><span>{unitsForGroup(group.channelIds, channelUnits, calibration)}{railText(group.channelIds)}</span></div>
          <LivePlot {samples} {channels} visibleChannelIds={group.channelIds} {channelUnits} {calibration} {adcBits} {displayRevision} {timeOriginUs} {hardwareSource} {markers} {paused} {onPlotError} />
        </div>
      {/each}
    </div>
  {/if}
  <details class="display-options studio-form" bind:open={displayOptionsOpen}>
    <summary>Display options <span class="summary-text">{arrangementSummary}</span></summary>
    <section class="plot-arrangement" aria-labelledby="plot-arrangement-title">
      <div class="plot-arrangement-heading">
        <div><h3 id="plot-arrangement-title">Plot arrangement</h3><p class="help">Display-only grouping. Every captured signal remains in the raw BMEG, CSV, and metadata even when hidden.</p></div>
        <div class="plot-count-control" role="group" aria-label="Number of plots"><span>Number of plots</span><button type="button" aria-label="Use one fewer plot" onclick={() => plotGroups = setPlotGroupCount(channels, plotGroups, plotGroups.length - 1)} disabled={plotGroups.length <= 1 || !channels.length}>−</button><strong>{plotGroups.length}</strong><button type="button" aria-label="Use one more plot" onclick={() => plotGroups = setPlotGroupCount(channels, plotGroups, plotGroups.length + 1)} disabled={plotGroups.length >= channels.length || !channels.length}>+</button></div>
      </div>
      <div class="button-pair"><button type="button" onclick={() => plotGroups = overlayAll(channels)} disabled={!channels.length}>Overlay all</button><button type="button" onclick={() => plotGroups = onePlotPerSignal(channels)} disabled={!channels.length}>One plot per signal</button></div>
      <div class="plot-assignment-grid" role="group" aria-label="Signal plot assignments">
        {#each channels as channel}
          <label>{channel.label}<select value={String(Math.max(0, plotGroups.findIndex((group) => group.channelIds.includes(channel.id))) + 1)} onchange={(event) => plotGroups = assignChannelToPlot(channels, plotGroups, channel.id, Number(event.currentTarget.value) - 1)} disabled={!plotGroups.length}>{#each plotGroups as _group, index}<option value={String(index + 1)}>Plot {index + 1}</option>{/each}</select></label>
        {/each}
      </div>
    </section>
  </details>
</section>

<style>
  .panel { min-width: 0; margin: 1rem 0; padding: clamp(.6rem, 1.5vw, .9rem); background: var(--surface); border: 1px solid var(--border); border-radius: .35rem; }
  .plot-heading { display: flex; min-width: 0; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: .4rem 1rem; }
  .plot-heading h2 { margin: 0; font-size: 1.3rem; }
  /* Compact, one line: the details beside the heading, not a row of tiles. */
  .plot-heading .metric-grid { display: flex; flex-wrap: wrap; gap: .35rem; margin: 0; }
  .plot-heading .metric-grid span { padding: .15rem .55rem; border-radius: 999px; font-size: .9rem; }
  .plot-heading .metric-grid strong { display: inline; margin-right: .35rem; font-size: .72rem; }
  .teaching-line { margin: .4rem 0 0; padding: .25rem .6rem; border-left: 4px solid var(--wvu-gold); background: var(--warn-bg); font-size: .85rem; line-height: 1.35; }
  .help { margin: .5rem 0 0; color: var(--muted); }
  .trace-controls { min-width: 0; border: 0; padding: 0; display: flex; flex-wrap: wrap; align-items: center; gap: .35rem .8rem; margin: .1rem 0 .5rem; }
  .trace-controls legend { float: left; margin: 0 .4rem 0 0; padding: 0; font-weight: 650; }
  .trace-controls .choice { display: flex; align-items: center; gap: .4rem; font-size: .9rem; font-weight: 600; }
  .display-note { margin-left: auto; color: var(--muted); font-size: .85rem; }
  .stacked-plots { display: grid; gap: .7rem; min-width: 0; }
  .stacked-plot { position: relative; min-width: 0; min-height: 0; padding: .5rem .6rem; border: 1px solid var(--border); border-radius: .3rem; background: #f9fbfc; }
  .stacked-plot-heading { display: flex; flex-wrap: wrap; justify-content: space-between; gap: .5rem; margin-bottom: .35rem; color: #42515d; }
  /* Over the traces, under the newest-value badges and marker labels. */
  .sim-watermark { position: absolute; inset: 0; z-index: 0; display: grid; place-items: center; overflow: hidden; pointer-events: none; }
  .sim-watermark span { padding: .15em .5em; border: .35rem solid rgb(0 40 85 / 14%); border-radius: .5rem; color: rgb(0 40 85 / 16%); font-size: clamp(2rem, 6vw, 4.5rem); font-weight: 900; letter-spacing: .12em; white-space: nowrap; transform: rotate(-14deg); }
  .display-options { min-width: 0; margin-top: .85rem; }
  .display-options > summary { cursor: pointer; color: var(--wvu-navy); font-weight: 700; }
  .display-options .summary-text { margin-left: .5rem; color: var(--muted); font-weight: 600; }
  .plot-arrangement { min-width: 0; margin-top: .75rem; padding: .75rem; border: 1px solid var(--border); border-radius: .3rem; background: #f9fbfc; }
  .plot-arrangement-heading { display: flex; min-width: 0; flex-wrap: wrap; justify-content: space-between; gap: .75rem; align-items: start; }
  .plot-arrangement h3 { margin: 0; font-size: 1rem; }
  .plot-arrangement .help { max-width: 72ch; }
  .plot-count-control { display: flex; flex-wrap: wrap; align-items: center; gap: .45rem; font-weight: 700; }
  .plot-count-control button { min-height: 2.1rem; min-width: 2.1rem; padding: .2rem .55rem; text-align: center; }
  .plot-count-control strong { min-width: 1.5rem; text-align: center; }
  .plot-assignment-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 15rem), 1fr)); gap: .65rem; margin-top: .8rem; }
</style>
