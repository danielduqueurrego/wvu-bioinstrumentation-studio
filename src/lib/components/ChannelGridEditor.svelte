<script lang="ts">
  import { assignDraftPlotNumber, MAX_ANALOG_CHANNELS, newChannel, normalizeDraftPlotGroups } from '$lib/lab-authoring';
  import type { LabChannel, LabPlotGroup } from '$lib/labs';

  /**
   * Lab Manager's analog channel grid, shared by simultaneous and
   * LED-sequenced labs: pin, ID, label, CSV field, calibration capability,
   * default visibility, the default plot, and (LED-sequenced labs only) the
   * display-only "By LED state" default. The parent's draft holds the only
   * state; every edit is reported through `onChange` with the default plots
   * kept consistent with the channels.
   */
  type Props = {
    channels: LabChannel[];
    /** The draft's default plots (`plot_defaults.groups`). */
    groups: LabPlotGroup[];
    /** Adds the "By LED state" column (LED-sequenced labs; display-only). */
    showSplit?: boolean;
    maxChannels?: number;
    title?: string;
    help: string;
    onChange: (channels: LabChannel[], groups: LabPlotGroup[]) => void;
  };

  let { channels, groups, showSplit = false, maxChannels = MAX_ANALOG_CHANNELS, title = 'Analog channel editor', help, onChange }: Props = $props();

  const analogPins = ['A0', 'A1', 'A2', 'A3', 'A4', 'A5'];

  function update(next: LabChannel[]) {
    onChange(next, normalizeDraftPlotGroups(next, groups));
  }

  function addChannel() {
    if (channels.length >= maxChannels) return;
    const pin = analogPins.find((candidate) => !channels.some((channel) => channel.pin === candidate)) ?? 'A0';
    // The lowest unused number, so a removal never leads to a duplicate ID or CSV field.
    update([...channels, newChannel(pin, channels)]);
  }

  function removeChannel(index: number) {
    if (channels.length <= 1) return;
    update(channels.filter((_, candidate) => candidate !== index));
  }

  function updateChannel(index: number, change: Partial<LabChannel>) {
    update(channels.map((channel, candidate) => candidate === index ? { ...channel, ...change } : channel));
  }

  function setPlotNumber(channelId: string, value: string) {
    onChange(channels, assignDraftPlotNumber(channels, groups, channelId, Number(value)));
  }

  function plotNumberOf(channelId: string): string {
    return String(Math.max(0, groups.findIndex((group) => group.channel_ids.includes(channelId))) + 1);
  }

  function capabilityOf(channel: LabChannel): string {
    if (channel.allowed_conversions?.includes('mpxv_pressure')) return 'mpxv_pressure';
    if (channel.allowed_conversions?.includes('linear_calibration')) return 'linear_calibration';
    return 'counts_volts';
  }
</script>

<section class="subpanel channel-editor" aria-label={title}>
  <div class="manager-heading"><h3>{title}</h3><button type="button" onclick={addChannel} disabled={channels.length >= maxChannels}>Add channel</button></div>
  <p class="help">{help}</p>
  <div class="channel-grid header" class:with-split={showSplit} aria-hidden="true"><span>Pin</span><span>Channel ID</span><span>Label</span><span>CSV field</span><span>Calibration capability</span><span>Visible</span>{#if showSplit}<span>By LED state</span>{/if}<span>Plot</span><span>Remove</span></div>
  {#each channels as channel, index}
    <div class="channel-grid" class:with-split={showSplit} role="group" aria-label={`Channel ${index + 1}`}>
      <select aria-label={`Channel ${index + 1} pin`} value={channel.pin} onchange={(event) => updateChannel(index, { pin: event.currentTarget.value })}>{#each analogPins as pin}<option value={pin}>{pin}</option>{/each}</select>
      <input aria-label={`Channel ${index + 1} ID`} value={channel.id} oninput={(event) => updateChannel(index, { id: event.currentTarget.value })} />
      <input aria-label={`Channel ${index + 1} label`} value={channel.label} oninput={(event) => updateChannel(index, { label: event.currentTarget.value })} />
      <input aria-label={`Channel ${index + 1} CSV field`} value={channel.csv_name} oninput={(event) => updateChannel(index, { csv_name: event.currentTarget.value })} />
      <select aria-label={`Channel ${index + 1} calibration capability`} value={capabilityOf(channel)} onchange={(event) => updateChannel(index, { allowed_conversions: [event.currentTarget.value] })}><option value="counts_volts">Counts / Volts only</option><option value="mpxv_pressure">MPXV pressure</option><option value="linear_calibration">Generic linear calibration</option></select>
      <input type="checkbox" aria-label={`Channel ${index + 1} visible by default`} checked={channel.default_visible !== false} onchange={(event) => updateChannel(index, { default_visible: event.currentTarget.checked })} />
      {#if showSplit}
        <!-- Display-only default: the live plot draws this channel as RED, DARK 1, IR, and DARK 2 traces. -->
        <input type="checkbox" aria-label={`Channel ${index + 1} shown by LED state`} checked={channel.split_by_led_state === true} onchange={(event) => updateChannel(index, { split_by_led_state: event.currentTarget.checked || undefined })} />
      {/if}
      <select aria-label={`Channel ${index + 1} plot`} value={plotNumberOf(channel.id)} onchange={(event) => setPlotNumber(channel.id, event.currentTarget.value)}>{#each channels as _, groupIndex}<option value={String(groupIndex + 1)}>Plot {groupIndex + 1}</option>{/each}</select>
      <button type="button" aria-label={`Remove channel ${index + 1}`} onclick={() => removeChannel(index)} disabled={channels.length <= 1}>Remove</button>
    </div>
  {/each}
</section>

<style>
  .subpanel { min-width: 0; margin-top: 1rem; padding: .8rem; border: 1px solid #d7dde2; border-radius: .3rem; background: #f9fbfc; overflow-x: auto; }
  .subpanel h3 { margin-top: 0; }
  .manager-heading { display: flex; flex-wrap: wrap; gap: .7rem; align-items: end; justify-content: space-between; }
  .manager-heading h3 { margin: 0; }
  .help { color: #4b5965; }
  input, select { min-width: 0; width: 100%; min-height: 2.35rem; border: 1px solid #8493a0; border-radius: .25rem; padding: .35rem .5rem; font: inherit; }
  button { min-height: 2.4rem; border: 1px solid #002855; border-radius: .3rem; background: #fff; color: #002855; padding: .45rem .7rem; font: inherit; font-weight: 650; cursor: pointer; }
  button:disabled { cursor: not-allowed; opacity: .55; }
  button:focus-visible, input:focus-visible, select:focus-visible { outline: 3px solid var(--focus-ring, #002855); outline-offset: 2px; box-shadow: 0 0 0 2px #fff; }
  /* The minimum widths leave the last column room for a whole "Remove" button
     (the modal's text may wrap anywhere); a narrow modal scrolls the grid instead. */
  .channel-grid { display: grid; grid-template-columns: 5.5rem minmax(8rem, 1fr) minmax(10rem, 1.2fr) minmax(10rem, 1.2fr) minmax(12rem, 1.2fr) 4.5rem 5rem auto; gap: .45rem; align-items: center; margin-top: .45rem; min-width: 64rem; }
  .channel-grid.with-split { grid-template-columns: 5.5rem minmax(8rem, 1fr) minmax(10rem, 1.2fr) minmax(10rem, 1.2fr) minmax(12rem, 1.2fr) 4.5rem 6rem 5rem auto; min-width: 70rem; }
  .channel-grid button { white-space: nowrap; }
  .channel-grid.header { font-weight: 800; font-size: .84rem; }
  .channel-grid input[type='checkbox'] { width: auto; min-height: auto; justify-self: center; }
  @media (max-width: 700px) { .manager-heading { align-items: stretch; } .manager-heading > button { flex: 1 1 12rem; } }
</style>
