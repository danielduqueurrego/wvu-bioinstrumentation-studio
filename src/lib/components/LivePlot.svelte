<script lang="ts">
  import { onMount } from 'svelte';
  import uPlot from 'uplot';
  import { visiblePlotSeries, shouldShowPlotLegend, type PlotSeries } from '$lib/multichannel';
  import { displayedValue, displayUnitLabel, type DisplayUnit, type RecordingCalibration } from '$lib/calibration';
  import 'uplot/dist/uPlot.min.css';

  type PlotChannel = { id: string; label: string; csv_name: string };
  export let samples: Array<{ timestamp_us: number; values: number[] }> = [];
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

  const maximum = 1_500;
  let element: HTMLDivElement;
  let plot: uPlot | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let resizeFrame: number | undefined;
  let recoveryFrame: number | undefined;
  let plotSignature = '';
  let failedSignature = '';
  let recoveryAttemptedSignature = '';
  let activeSeries: PlotSeries[] = [];

  function displayChannels(): PlotSeries[] {
    return activeSeries;
  }

  function chartData(): uPlot.AlignedData {
    const bounded = samples.slice(-maximum);
    const active = displayChannels();
    const x = bounded.map((point) => point.timestamp_us / 1_000_000);
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
          series: [
            {},
            ...active.map((channel) => ({ label: channel.label, stroke: channel.color, width: 2 }))
          ],
          axes: [{}, {
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
    } catch (error) {
      handlePlotFailure('create', error, allowRecovery);
    }
  }

  function update() {
    try {
      if (plotSignature !== currentSignature()) createPlot();
      else plot?.setData(chartData());
    } catch (error) {
      handlePlotFailure('update', error);
    }
  }

  function resizePlot() {
    resizeFrame = undefined;
    if (!plot || !element) return;
    try {
      const width = Math.floor(element.clientWidth);
      const height = Math.floor(element.clientHeight);
      if (width >= 220 && height >= 180) plot.setSize({ width, height });
    } catch (error) {
      handlePlotFailure('resize', error);
    }
  }

  function queueResize() {
    if (resizeFrame === undefined) resizeFrame = window.requestAnimationFrame(resizePlot);
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
    activeSeries = visiblePlotSeries(channels, visibleChannelIds);
    void _revision;
    void _samples;
    void _channels;
    void _visible;
    void _units;
    void _calibration;
    void _adcBits;
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
<div class="plot" bind:this={element} aria-label="Bounded synchronized live acquisition plot"></div>

<style>
  .plot-legend { display: flex; flex-wrap: wrap; gap: .35rem .8rem; align-items: center; margin: 0 0 .45rem; color: #42515d; font-size: .87rem; }
  .plot-legend-item { display: inline-flex; min-width: 0; align-items: center; gap: .35rem; overflow-wrap: anywhere; }
  .plot-legend-swatch { display: inline-block; flex: 0 0 .85rem; width: .85rem; height: .26rem; border-radius: .15rem; }
  .plot { width: 100%; height: clamp(260px, 42vh, 500px); min-width: 0; min-height: 240px; overflow: hidden; }
</style>
