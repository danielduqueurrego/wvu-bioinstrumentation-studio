<script lang="ts">
  import { onMount } from 'svelte';
  import uPlot from 'uplot';
  import { visiblePlotSeries, shouldShowPlotLegend, type PlotSeries } from '$lib/multichannel';
  import { displayedValue, displayUnitLabel, type DisplayUnit, type RecordingCalibration } from '$lib/calibration';
  import { markersInRange, type PlacedMarker } from '$lib/markers';
  import {
    formatLiveDisplayValue,
    formatElapsedSeconds,
    filterStartupDisplayTransient,
    layoutEndpointLabels,
    plotAreaCssBounds,
    MAX_RENDERED_DISPLAY_POINTS
  } from '$lib/live-display';
  import 'uplot/dist/uPlot.min.css';

  type PlotChannel = { id: string; label: string; csv_name: string };
  export let samples: Array<{ sequence: number; timestamp_us: number; values: number[] }> = [];
  export let channels: PlotChannel[] = [];
  export let visibleChannelIds: string[] = [];
  export let channelUnits: Record<string, DisplayUnit> = {};
  export let calibration: RecordingCalibration = { adc_reference_v: 5, mpxv_sensor_supply_v: 5, channel_units: {}, active_calibrations: [] };
  export let adcBits = 12;
  // Rendering is intentionally independent from acquisition.  If uPlot rejects a
  // transient resize or series rebuild, the parent records the diagnostic but the
  // active serial session and writer continue uninterrupted.
  export let onPlotError: (stage: string, detail: string) => void = () => {};
  // The Acquisition page advances this once per bounded display snapshot.  Multiple
  // stacked plots therefore consume one shared update rather than each polling or
  // scheduling its own sample refresh loop.
  export let displayRevision = 0;
  export let timeOriginUs: number | undefined = undefined;
  export let hardwareSource = false;
  // Markers of the current recording on the elapsed-time axis (display-only).
  export let markers: PlacedMarker[] = [];
  // Display-only pause: the plot keeps the frozen frame and never calls
  // setData; it is rebuilt only when its series must change.
  export let paused = false;

  type EndpointLabel = { id: string; label: string; color: string; value: string; top: number };
  type MarkerLine = { id: string; text: string; left: number; top: number; height: number };

  // The backend returns no more than this many timestamp-windowed records. Keep
  // a matching defensive cap here in case a future backend response is larger.
  const maximum = MAX_RENDERED_DISPLAY_POINTS;
  let element: HTMLDivElement;
  let plot: uPlot | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let resizeFrame: number | undefined;
  let recoveryFrame: number | undefined;
  let plotSignature = '';
  let failedSignature = '';
  let recoveryAttemptedSignature = '';
  let activeSeries: PlotSeries[] = [];
  let endpointLabels: EndpointLabel[] = [];
  let markerLines: MarkerLine[] = [];
  // The elapsed-time range of the data last given to uPlot.
  let dataRange: { min: number; max: number } | undefined;

  function displayChannels(): PlotSeries[] {
    return activeSeries;
  }

  function chartData(): uPlot.AlignedData {
    const active = displayChannels();
    const channelIndices = active
      .map((channel) => channels.findIndex((candidate) => candidate.id === channel.id))
      .filter((index) => index >= 0);
    const filtered = filterStartupDisplayTransient(samples, channelIndices, adcBits, hardwareSource, timeOriginUs);
    const bounded = filtered.slice(-maximum);
    const origin = timeOriginUs ?? bounded[0]?.timestamp_us ?? 0;
    const x = bounded.map((point) => Math.max(0, (point.timestamp_us - origin) / 1_000_000));
    dataRange = x.length ? { min: x[0], max: x[x.length - 1] } : undefined;
    const values = active.map((channel) => {
      const index = channels.findIndex((candidate) => candidate.id === channel.id);
      return bounded.map((point) => {
        const value = point.values[index] ?? 0;
        return displayedValue(value, channel.id, channelUnits[channel.id] ?? 'counts', adcBits, calibration);
      });
    });
    return [x, ...values];
  }

  function currentSignature() {
    return `${channels.map((channel) => channel.id).join('|')}/${visibleChannelIds.join('|')}/${JSON.stringify(channelUnits)}/${JSON.stringify(calibration)}/${adcBits}`;
  }

  function errorDetail(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
  }

  function destroyPlot() {
    const previous = plot;
    plot = undefined;
    plotSignature = '';
    endpointLabels = [];
    markerLines = [];
    if (!previous) return;
    try {
      previous.destroy();
    } catch (error) {
      console.warn('Live plot cleanup failed', error);
    }
  }

  function reportPlotError(stage: string, error: unknown) {
    const signature = currentSignature();
    failedSignature = signature;
    const detail = errorDetail(error);
    console.error(`Live plot ${stage} failed`, error);
    // Do not let a diagnostic callback rethrow into the component lifecycle.
    try {
      onPlotError(stage, detail);
    } catch (callbackError) {
      console.warn('Live plot error reporting failed', callbackError);
    }
  }

  function queueOneRecovery() {
    const signature = currentSignature();
    if (recoveryAttemptedSignature === signature || recoveryFrame !== undefined) return;
    recoveryAttemptedSignature = signature;
    recoveryFrame = window.requestAnimationFrame(() => {
      recoveryFrame = undefined;
      if (!plot) createPlot(false);
    });
  }

  function handlePlotFailure(stage: string, error: unknown, allowRecovery = true) {
    destroyPlot();
    reportPlotError(stage, error);
    if (allowRecovery) queueOneRecovery();
  }

  function createPlot(allowRecovery = true) {
    if (!element) return;
    try {
      const width = Math.max(220, Math.floor(element.clientWidth));
      const height = Math.max(220, Math.floor(element.clientHeight));
      destroyPlot();
      const active = displayChannels();
      if (!active.length) return;
      plot = new uPlot(
        {
          width,
          height,
          scales: { x: { time: false } },
          series: [
            {},
            ...active.map((channel) => ({ label: channel.label, stroke: channel.color, width: 2 }))
          ],
          // uPlot passes the tick spacing it chose as the 5th argument; the
          // label decimals follow it (0.25 s ticks need two decimals).
          axes: [{ values: (_u, splits, _axisIdx, _space, increment) => splits.map((value) => formatElapsedSeconds(value, increment)) }, {
            label: new Set(active.map((channel) => displayUnitLabel(channelUnits[channel.id] ?? 'counts', calibration, channel.id))).size === 1
              ? displayUnitLabel(channelUnits[active[0]?.id] ?? 'counts', calibration, active[0]?.id)
              : 'Mixed units'
          }],
          // The native uPlot legend is not consistently usable in the bundled
          // WebView.  A compact Svelte legend below uses the identical series
          // order and color mapping, only when a plot has multiple traces.
          legend: { show: false }
        },
        chartData(),
        element
      );
      plotSignature = currentSignature();
      failedSignature = '';
      recoveryAttemptedSignature = '';
      refreshEndpointLabels();
      refreshMarkerLines();
    } catch (error) {
      handlePlotFailure('create', error, allowRecovery);
    }
  }

  function update() {
    try {
      if (plotSignature !== currentSignature()) createPlot();
      else if (paused) refreshMarkerLines();
      else {
        plot?.setData(chartData());
        refreshEndpointLabels();
        refreshMarkerLines();
      }
    } catch (error) {
      handlePlotFailure('update', error);
    }
  }

  /**
   * Vertical marker lines at each marker's elapsed recording time, for the
   * markers inside the plotted range. Positioned in CSS pixels over the
   * plotting area; display-only.
   */
  function refreshMarkerLines() {
    if (!plot || !markers.length) {
      markerLines = [];
      return;
    }
    const currentPlot = plot;
    const ratio = Number.isFinite(uPlot.pxRatio) && uPlot.pxRatio > 0 ? uPlot.pxRatio : 1;
    const bounds = plotAreaCssBounds(currentPlot.bbox, ratio);
    const left = currentPlot.bbox.left / ratio;
    const width = currentPlot.bbox.width / ratio;
    const scale = currentPlot.scales?.x;
    const min = Number.isFinite(scale?.min) ? scale!.min! : dataRange?.min;
    const max = Number.isFinite(scale?.max) ? scale!.max! : dataRange?.max;
    markerLines = markersInRange(markers, min, max).flatMap((marker) => {
      const offset = currentPlot.valToPos(marker.seconds, 'x');
      if (!Number.isFinite(offset) || offset < 0 || (width > 0 && offset > width)) return [];
      return [{ id: marker.id, text: marker.text, left: left + offset, top: bounds.top, height: Math.max(0, bounds.bottom - bounds.top) }];
    });
  }

  function resizePlot() {
    resizeFrame = undefined;
    if (!plot || !element) return;
    try {
      const width = Math.floor(element.clientWidth);
      const height = Math.floor(element.clientHeight);
      if (width >= 220 && height >= 180) {
        plot.setSize({ width, height });
        refreshEndpointLabels();
        refreshMarkerLines();
      }
    } catch (error) {
      handlePlotFailure('resize', error);
    }
  }

  function queueResize() {
    if (resizeFrame === undefined) resizeFrame = window.requestAnimationFrame(resizePlot);
  }

  /**
   * Endpoint values always use the newest exact frame returned by the bounded
   * display query. They are converted solely for display; no endpoint overlay
   * can alter acquisition, the serial transport, or the raw recording.
   */
  function refreshEndpointLabels() {
    if (!plot || !activeSeries.length || !samples.length) {
      endpointLabels = [];
      return;
    }
    const currentPlot = plot;
    const newest = samples[samples.length - 1];
    // Badges are placed in CSS pixels; uPlot's bbox is in canvas pixels.
    const bounds = plotAreaCssBounds(currentPlot.bbox, uPlot.pxRatio);
    const pending = activeSeries.map((channel) => {
      const channelIndex = channels.findIndex((candidate) => candidate.id === channel.id);
      const unit = channelUnits[channel.id] ?? 'counts';
      const value = displayedValue(newest.values[channelIndex] ?? 0, channel.id, unit, adcBits, calibration);
      return {
        id: channel.id,
        label: channel.label,
        color: channel.color,
        value: formatLiveDisplayValue(value, unit, calibration, channel.id),
        // CSS pixels from the top of the plotting area, which starts at bounds.top.
        naturalTop: bounds.top + currentPlot.valToPos(value, 'y')
      };
    });
    const positions = new Map(
      layoutEndpointLabels(pending, bounds.top, bounds.bottom)
        .map((label) => [label.id, label.top ?? bounds.top])
    );
    endpointLabels = pending.map((label) => ({
      id: label.id,
      label: label.label,
      color: label.color,
      value: label.value,
      top: positions.get(label.id) ?? bounds.top
    }));
  }

  // Mention every display input in this reactive block so a live trace toggle or
  // layout change updates immediately, while new samples arrive via displayRevision.
  $: {
    const _revision = displayRevision;
    const _samples = samples;
    const _channels = channels;
    const _visible = visibleChannelIds;
    const _units = channelUnits;
    const _calibration = calibration;
    const _adcBits = adcBits;
    const _timeOriginUs = timeOriginUs;
    const _hardwareSource = hardwareSource;
    const _markers = markers;
    const _paused = paused;
    activeSeries = visiblePlotSeries(channels, visibleChannelIds);
    void _revision;
    void _samples;
    void _channels;
    void _visible;
    void _units;
    void _calibration;
    void _adcBits;
    void _timeOriginUs;
    void _hardwareSource;
    void _markers;
    void _paused;
    if (plot) update();
    else if (element && failedSignature !== currentSignature()) createPlot();
  }

  onMount(() => {
    createPlot();
    resizeObserver = new ResizeObserver(queueResize);
    resizeObserver.observe(element);
    return () => {
      if (resizeFrame !== undefined) window.cancelAnimationFrame(resizeFrame);
      if (recoveryFrame !== undefined) window.cancelAnimationFrame(recoveryFrame);
      resizeObserver?.disconnect();
      destroyPlot();
    };
  });
</script>

{#if shouldShowPlotLegend(activeSeries)}
  <div class="plot-legend" aria-label="Plot legend" role="list">
    {#each activeSeries as channel (channel.id)}
      <span class="plot-legend-item" role="listitem"><span class="plot-legend-swatch" style:background-color={channel.color} aria-hidden="true"></span>{channel.label}</span>
    {/each}
  </div>
{/if}
<div class="plot-shell">
  <div class="plot" bind:this={element} role="img" aria-label={`Live signal plot${markers.length ? `, ${markers.length} marker${markers.length === 1 ? '' : 's'}` : ''}${paused ? ', paused' : ''}`}></div>
  <!-- Marker lines repeat the markers named in the plot's label; hidden from assistive technology. -->
  <div class="marker-lines" aria-hidden="true">
    {#each markerLines as line (line.id)}
      <div class="marker-line" style:left={`${line.left}px`} style:top={`${line.top}px`} style:height={`${line.height}px`}>
        <span class="marker-text">{line.text}</span>
      </div>
    {/each}
  </div>
  <div class="endpoint-labels" role="list" aria-label="Newest visible signal values">
    {#each endpointLabels as endpoint (endpoint.id)}
      <span
        class="endpoint-label"
        role="listitem"
        style:top={`${endpoint.top}px`}
        style:--series-color={endpoint.color}
        aria-label={`${endpoint.label}: ${endpoint.value}`}
      >{endpoint.value}</span>
    {/each}
  </div>
</div>

<style>
  .plot-legend { display: flex; flex-wrap: wrap; gap: .35rem .8rem; align-items: center; margin: 0 0 .45rem; color: #42515d; font-size: .87rem; }
  .plot-legend-item { display: inline-flex; min-width: 0; align-items: center; gap: .35rem; overflow-wrap: anywhere; }
  .plot-legend-swatch { display: inline-block; flex: 0 0 .85rem; width: .85rem; height: .26rem; border-radius: .15rem; }
  .plot-shell { position: relative; min-width: 0; }
  .plot { width: 100%; height: clamp(260px, 42vh, 500px); min-width: 0; min-height: 240px; overflow: hidden; }
  .endpoint-labels { position: absolute; inset: 0; pointer-events: none; overflow: hidden; z-index: 2; }
  .marker-lines { position: absolute; inset: 0; pointer-events: none; overflow: hidden; z-index: 1; }
  .marker-line { position: absolute; width: 0; border-left: 2px dashed var(--wvu-navy, #002855); }
  .marker-text {
    position: absolute;
    top: 0;
    left: 3px;
    max-width: 10rem;
    overflow: hidden;
    padding: .05rem .3rem;
    border-radius: .2rem;
    background: var(--wvu-navy, #002855);
    color: #fff;
    font-size: .72rem;
    font-weight: 700;
    line-height: 1.2;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .endpoint-label { position: absolute; right: .45rem; max-width: calc(100% - 1rem); transform: translateY(-50%); padding: .1rem .34rem; border: 1px solid var(--series-color); border-radius: .25rem; background: rgb(255 255 255 / 88%); color: #172b3a; box-shadow: 0 1px 2px rgb(0 0 0 / 12%); font-size: .76rem; font-weight: 700; line-height: 1.15; white-space: nowrap; }
</style>
