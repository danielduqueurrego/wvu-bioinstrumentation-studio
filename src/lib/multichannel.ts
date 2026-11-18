/** Small pure helpers shared by the profile-aware Acquisition page and plot. */
import type { LabChannel, LabProfile } from './labs';

export type DisplayChannel = { id: string; label: string; csv_name: string; default_visible?: boolean };
export type VisibleChannelMap = Record<string, boolean>;
export type PlotGroup = { id: string; channelIds: string[] };
export type PlotSeries = DisplayChannel & { color: string };
/** A rendered plot plus its slot number in the assignment selectors. */
export type NumberedPlotGroup = PlotGroup & { slot: number };
/** A channel as the page plots it; pulse-ox raw fields are not lab channels. */
export type PlotChannel = DisplayChannel & { pin?: string; units?: string; allowed_conversions?: string[] };

// One shared color mapping. The uPlot stroke and the Svelte legend both
// consume `visiblePlotSeries`, so a legend can never drift from the line it
// identifies. The gold is darkened for at least 3:1 against the plot background.
const plotColors = ['#002855', '#B07800', '#007A78', '#9D2235', '#5D4E99', '#C65E00'];

/**
 * Pulse-ox raw fields keep meaningful colours wherever they are drawn: RED is
 * red, IR a dark violet, and the two DARK phases greys. RX uses a lighter
 * shade of the same colour as TX so an overlay still tells them apart.
 */
const pulseOxColors: Record<string, string> = {
  red_tx: '#C8102E',
  red_rx: '#E35D6A',
  dark1_tx: '#5B6670',
  dark1_rx: '#7D8891',
  ir_tx: '#4B1F7A',
  ir_rx: '#8C6BC2',
  dark2_tx: '#1E252B',
  dark2_rx: '#4A535B'
};

export function seriesColor(index: number): string {
  return plotColors[index % plotColors.length];
}

/**
 * The colour of one channel. It depends on the channel itself (its position
 * among all of the lab's plot channels, or its pulse-ox role), never on which
 * traces are visible or which plot it was assigned to, so a signal keeps its
 * colour when others are hidden or rearranged.
 */
export function channelColor(channels: DisplayChannel[], channelId: string): string {
  return pulseOxColors[channelId] ?? seriesColor(Math.max(0, channels.findIndex((channel) => channel.id === channelId)));
}

export function visibleChannels(channels: DisplayChannel[], selected: string[]): DisplayChannel[] {
  return channels.filter((channel) => selected.includes(channel.id));
}

/** Series currently visible in one display-only plot group, in uPlot order. */
export function visiblePlotSeries(channels: DisplayChannel[], selected: string[]): PlotSeries[] {
  return visibleChannels(channels, selected).map((channel) => ({
    ...channel,
    color: channelColor(channels, channel.id)
  }));
}

/** The eight raw pulse-ox fields, in record order. One stable array. */
export const PULSEOX_PLOT_CHANNELS: readonly PlotChannel[] = Object.freeze([
  { id: 'red_tx', label: 'TX Red', csv_name: 'red_TX', allowed_conversions: ['counts_volts'] },
  { id: 'dark1_tx', label: 'TX Dark 1', csv_name: 'dark1_TX', allowed_conversions: ['counts_volts'] },
  { id: 'ir_tx', label: 'TX IR', csv_name: 'ir_TX', allowed_conversions: ['counts_volts'] },
  { id: 'dark2_tx', label: 'TX Dark 2', csv_name: 'dark2_TX', allowed_conversions: ['counts_volts'] },
  { id: 'red_rx', label: 'RX Red', csv_name: 'red_RX', allowed_conversions: ['counts_volts'] },
  { id: 'dark1_rx', label: 'RX Dark 1', csv_name: 'dark1_RX', allowed_conversions: ['counts_volts'] },
  { id: 'ir_rx', label: 'RX IR', csv_name: 'ir_RX', allowed_conversions: ['counts_volts'] },
  { id: 'dark2_rx', label: 'RX Dark 2', csv_name: 'dark2_RX', allowed_conversions: ['counts_volts'] }
]);

export function isPulseOxProfile(profile: LabProfile | undefined): boolean {
  return profile?.acquisition.acquisition_mode === 'pulseox_4state';
}

/** A lab's analog channels; a legacy single-pin lab becomes one 'raw' channel. */
export function profileChannels(profile: LabProfile | undefined): LabChannel[] {
  if (!profile) return [];
  if (profile.acquisition.channels?.length) return profile.acquisition.channels;
  return [{
    pin: profile.acquisition.analog_pin,
    id: 'raw',
    label: profile.display.channel_label,
    csv_name: profile.export.signal_name,
    units: 'ADC counts'
  }];
}

/** The signals the live plots draw for a lab, in record (`values`) order. */
export function plotChannelsFor(profile: LabProfile | undefined): PlotChannel[] {
  return isPulseOxProfile(profile) ? [...PULSEOX_PLOT_CHANNELS] : profileChannels(profile);
}

/**
 * Identifies one lab revision. While a recording runs the plots follow the
 * recorded lab; this key changes only when that revision really changes, not
 * on every status poll that returns a new (equal) snapshot object.
 */
export function labRevisionKey(profile: LabProfile | undefined): string {
  return profile ? `${profile.profile_id}@${profile.profile_version}` : '';
}

export function shouldShowPlotLegend(series: PlotSeries[]): boolean {
  return series.length >= 2;
}

/**
 * The profile owns the set of possible traces; this creates a fresh, single source of
 * truth for their visibility when that profile changes.  A checkbox must update this
 * map directly rather than being reconciled back to "all visible" on each render.
 */
export function initialTraceVisibility(channels: DisplayChannel[]): VisibleChannelMap {
  return Object.fromEntries(channels.map((channel) => [channel.id, channel.default_visible !== false]));
}

export function setTraceVisibility(
  visibility: VisibleChannelMap,
  channelId: string,
  isVisible: boolean
): VisibleChannelMap {
  return { ...visibility, [channelId]: isVisible };
}

export function visibleChannelIds(channels: DisplayChannel[], visibility: VisibleChannelMap): string[] {
  return channels.filter((channel) => visibility[channel.id] !== false).map((channel) => channel.id);
}

function groupId(index: number): string {
  return `plot-${index + 1}`;
}

function emptyGroups(count: number): PlotGroup[] {
  return Array.from({ length: count }, (_, index) => ({ id: groupId(index), channelIds: [] }));
}

/** Default, display-only layout for the current app session. */
export function defaultPlotGroups(category: string | undefined, channels: DisplayChannel[]): PlotGroup[] {
  if (!channels.length) return [];
  if (category === 'course_pulseox') {
    return [
      { id: groupId(0), channelIds: channels.filter((channel) => ['red_tx', 'dark1_tx', 'ir_tx', 'dark2_tx'].includes(channel.id)).map((channel) => channel.id) },
      { id: groupId(1), channelIds: channels.filter((channel) => ['red_rx', 'dark1_rx', 'ir_rx', 'dark2_rx'].includes(channel.id)).map((channel) => channel.id) }
    ];
  }
  // ECG is easiest to read as one trace. General Analog and the other course
  // profiles begin with one independently scaling plot per recorded signal.
  if (channels.length === 1 || category === 'course_ecg' || category === 'ecg') {
    return [{ id: groupId(0), channelIds: channels.map((channel) => channel.id) }];
  }
  return channels.map((channel, index) => ({ id: groupId(index), channelIds: [channel.id] }));
}

/**
 * Keeps every active signal in exactly one slot. Empty slots are retained for the
 * assignment UI but callers should not render charts for them.
 */
export function normalizePlotGroups(channels: DisplayChannel[], groups: PlotGroup[]): PlotGroup[] {
  if (!channels.length) return [];
  const activeIds = new Set(channels.map((channel) => channel.id));
  const normalized = (groups.length ? groups : emptyGroups(1)).map((group, index) => ({
    id: groupId(index),
    channelIds: group.channelIds.filter((id, position, ids) => activeIds.has(id) && ids.indexOf(id) === position)
  }));
  const assigned = new Set<string>();
  for (const group of normalized) {
    group.channelIds = group.channelIds.filter((id) => {
      if (assigned.has(id)) return false;
      assigned.add(id);
      return true;
    });
  }
  for (const channel of channels) {
    if (!assigned.has(channel.id)) normalized[0].channelIds.push(channel.id);
  }
  return normalized;
}

export function setPlotGroupCount(channels: DisplayChannel[], groups: PlotGroup[], requestedCount: number): PlotGroup[] {
  const count = Math.max(1, Math.min(channels.length, Math.floor(requestedCount)));
  const normalized = normalizePlotGroups(channels, groups);
  if (count >= normalized.length) return normalizePlotGroups(channels, [...normalized, ...emptyGroups(count - normalized.length)]);
  const retained = normalized.slice(0, count).map((group) => ({ ...group, channelIds: [...group.channelIds] }));
  for (const removed of normalized.slice(count)) retained[count - 1].channelIds.push(...removed.channelIds);
  return normalizePlotGroups(channels, retained);
}

export function assignChannelToPlot(channels: DisplayChannel[], groups: PlotGroup[], channelId: string, targetIndex: number): PlotGroup[] {
  const normalized = normalizePlotGroups(channels, groups).map((group) => ({ ...group, channelIds: group.channelIds.filter((id) => id !== channelId) }));
  const safeIndex = Math.max(0, Math.min(normalized.length - 1, targetIndex));
  normalized[safeIndex].channelIds.push(channelId);
  return normalizePlotGroups(channels, normalized);
}

export function overlayAll(channels: DisplayChannel[]): PlotGroup[] {
  return channels.length ? [{ id: groupId(0), channelIds: channels.map((channel) => channel.id) }] : [];
}

export function onePlotPerSignal(channels: DisplayChannel[]): PlotGroup[] {
  return channels.map((channel, index) => ({ id: groupId(index), channelIds: [channel.id] }));
}

export function visiblePlotGroups(channels: DisplayChannel[], groups: PlotGroup[], visibility: VisibleChannelMap): PlotGroup[] {
  return numberedVisiblePlotGroups(channels, groups, visibility).map(({ id, channelIds }) => ({ id, channelIds }));
}

/**
 * The plots to render, each numbered by its slot, so "Plot 2" is the same
 * plot as "Plot 2" in the assignment selectors even when an earlier slot is
 * empty or all of its signals are hidden (and therefore not rendered).
 */
export function numberedVisiblePlotGroups(
  channels: DisplayChannel[],
  groups: PlotGroup[],
  visibility: VisibleChannelMap
): NumberedPlotGroup[] {
  return normalizePlotGroups(channels, groups)
    .map((group, index) => ({
      ...group,
      slot: index + 1,
      channelIds: group.channelIds.filter((id) => visibility[id] !== false)
    }))
    .filter((group) => group.channelIds.length > 0);
}
