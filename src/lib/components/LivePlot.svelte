<script lang="ts">
  import { onMount } from 'svelte';
  import uPlot from 'uplot';
  import { pulseoxAmbientSubtractedPreview, visibleChannels } from '$lib/multichannel';
  import { displayedValue, displayUnitLabel, type DisplayUnit, type RecordingCalibration } from '$lib/calibration';
  import 'uplot/dist/uPlot.min.css';

  type PlotChannel = { id: string; label: string; csv_name: string };
  export let samples: Array<{ timestamp_us: number; values: number[] }> = [];
  export let channels: PlotChannel[] = [];
  export let visibleChannelIds: string[] = [];
  export let channelUnits: Record<string, DisplayUnit> = {};
  export let calibration: RecordingCalibration = { adc_reference_v: 5, mpxv_sensor_supply_v: 5, channel_units: {}, active_calibrations: [] };
  export let adcBits = 12;
  export let pulseoxPreview = false;
  // The Acquisition page advances this once per bounded display snapshot.  Multiple
  // stacked plots therefore consume one shared update rather than each polling or
  // scheduling its own sample refresh loop.
  export let displayRevision = 0;

  const maximum = 1_500;
  const colors = ['#002855', '#EEAA00', '#007A78', '#9D2235', '#5D4E99', '#C65E00'];
  let element: HTMLDivElement;
  let plot: uPlot | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let resizeFrame: number | undefined;
  let plotSignature = '';

  function displayChannels(): PlotChannel[] {
    return visibleChannels(channels, visibleChannelIds);
  }

  function previewValues(values: number[]): number[] {
    if (!pulseoxPreview || values.length < 8) return values;
    return pulseoxAmbientSubtractedPreview(values);
  }

  function chartData(): uPlot.AlignedData {
    const bounded = samples.slice(-maximum);
    const active = displayChannels();
    const x = bounded.map((point) => point.timestamp_us / 1_000_000);
    const values = active.map((channel) => {
      const index = channels.findIndex((candidate) => candidate.id === channel.id);
      return bounded.map((point) => {
        const value = previewValues(point.values)[index] ?? 0;
        return displayedValue(value, channel.id, channelUnits[channel.id] ?? 'counts', adcBits, calibration);
      });
    });
    return [x, ...values];
  }

  function currentSignature() {
    return `${channels.map((channel) => channel.id).join('|')}/${visibleChannelIds.join('|')}/${pulseoxPreview}/${JSON.stringify(channelUnits)}/${JSON.stringify(calibration)}/${adcBits}`;
  }

  function createPlot() {
    if (!element) return;
    const width = Math.max(220, Math.floor(element.clientWidth));
    const height = Math.max(220, Math.floor(element.clientHeight));
    plot?.destroy();
    const active = displayChannels();
    plot = new uPlot(
      {
        width,
        height,
        series: [
          {},
          ...active.map((channel, index) => ({ label: channel.label, stroke: colors[index % colors.length], width: 2 }))
        ],
        axes: [{}, {
          label: new Set(active.map((channel) => displayUnitLabel(channelUnits[channel.id] ?? 'counts', calibration, channel.id))).size === 1
            ? displayUnitLabel(channelUnits[active[0]?.id] ?? 'counts', calibration, active[0]?.id)
            : 'Mixed units'
        }],
        legend: { show: true }
      },
      chartData(),
      element
    );
    plotSignature = currentSignature();
  }

  function update() {
    if (plotSignature !== currentSignature()) createPlot();
    else plot?.setData(chartData());
  }

  function resizePlot() {
    resizeFrame = undefined;
    if (!plot || !element) return;
    const width = Math.floor(element.clientWidth);
    const height = Math.floor(element.clientHeight);
    if (width >= 220 && height >= 180) plot.setSize({ width, height });
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
    const _pulseoxPreview = pulseoxPreview;
    void _revision;
    void _samples;
    void _channels;
    void _visible;
    void _units;
    void _calibration;
    void _adcBits;
    void _pulseoxPreview;
    if (plot) update();
  }

  onMount(() => {
    createPlot();
    resizeObserver = new ResizeObserver(queueResize);
    resizeObserver.observe(element);
    return () => {
      if (resizeFrame !== undefined) window.cancelAnimationFrame(resizeFrame);
      resizeObserver?.disconnect();
      plot?.destroy();
    };
  });
</script>

<div class="plot" bind:this={element} aria-label="Bounded synchronized live acquisition plot"></div>

<style>
  .plot { width: 100%; height: clamp(260px, 42vh, 500px); min-width: 0; min-height: 240px; overflow: hidden; }
</style>
