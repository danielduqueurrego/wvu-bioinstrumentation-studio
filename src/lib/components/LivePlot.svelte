<script lang="ts">
  import { onMount } from 'svelte';
  import uPlot from 'uplot';
  import 'uplot/dist/uPlot.min.css';
  import { countsToVolts } from '$lib/conversion';

  export let samples: Array<{ timestamp_us: number; counts: number }> = [];
  export let volts = false;
  let element: HTMLDivElement;
  let plot: uPlot | undefined;
  const maximum = 1_500;

  function update() {
    if (!plot) return;
    const bounded = samples.slice(-maximum);
    const x = bounded.map((point) => point.timestamp_us / 1_000_000);
    const y = bounded.map((point) => volts ? countsToVolts(point.counts) : point.counts);
    plot.setData([x, y]);
  }

  onMount(() => {
    plot = new uPlot({ width: 760, height: 310, series: [{}, { label: volts ? 'Volts' : 'Counts', stroke: '#002855', width: 2 }], axes: [{}, { label: volts ? 'V' : 'ADC counts' }] }, [[], []], element);
    const interval = window.setInterval(update, 40); // 25 Hz; never an update per ADC sample.
    return () => { window.clearInterval(interval); plot?.destroy(); };
  });

</script>

<div class="plot" bind:this={element} aria-label="Bounded live analog signal plot"></div>
