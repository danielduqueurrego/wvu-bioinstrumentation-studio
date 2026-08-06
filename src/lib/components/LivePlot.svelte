<script lang="ts">
  import { onMount } from 'svelte';
  import uPlot from 'uplot';
  import 'uplot/dist/uPlot.min.css';
  import { countsToVolts } from '$lib/conversion';

  export let samples: Array<{ timestamp_us: number; counts: number }> = [];
  export let volts = false;

  const maximum = 1_500;
  let element: HTMLDivElement;
  let plot: uPlot | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let resizeFrame: number | undefined;
  let updateTimer: number | undefined;

  function chartData(): uPlot.AlignedData {
    const bounded = samples.slice(-maximum);
    return [
      bounded.map((point) => point.timestamp_us / 1_000_000),
      bounded.map((point) => (volts ? countsToVolts(point.counts) : point.counts))
    ];
  }

  function update() {
    plot?.setData(chartData());
  }

  function resizePlot() {
    resizeFrame = undefined;
    if (!plot || !element) return;
    const width = Math.floor(element.clientWidth);
    const height = Math.floor(element.clientHeight);
    if (width < 220 || height < 180) return;
    plot.setSize({ width, height });
  }

  function queueResize() {
    if (resizeFrame !== undefined) return;
    resizeFrame = window.requestAnimationFrame(resizePlot);
  }

  $: if (plot) update();

  onMount(() => {
    const width = Math.max(220, Math.floor(element.clientWidth));
    const height = Math.max(220, Math.floor(element.clientHeight));
    plot = new uPlot(
      {
        width,
        height,
        series: [
          {},
          { label: 'Raw signal', stroke: '#002855', width: 2 }
        ],
        axes: [{}, { label: 'Counts or volts' }]
      },
      chartData(),
      element
    );
    resizeObserver = new ResizeObserver(queueResize);
    resizeObserver.observe(element);
    updateTimer = window.setInterval(update, 40); // 25 Hz batched UI update.

    return () => {
      if (resizeFrame !== undefined) window.cancelAnimationFrame(resizeFrame);
      if (updateTimer !== undefined) window.clearInterval(updateTimer);
      resizeObserver?.disconnect();
      plot?.destroy();
    };
  });
</script>

<div class="plot" bind:this={element} aria-label="Bounded live analog signal plot"></div>

<style>
  .plot {
    width: 100%;
    height: clamp(240px, 38vh, 420px);
    min-width: 0;
    min-height: 220px;
    overflow: hidden;
  }
</style>
