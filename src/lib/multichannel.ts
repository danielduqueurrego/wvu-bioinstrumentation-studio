/** Small pure helpers shared by the profile-aware Acquisition page and plot. */
import { LED_STATE_LABELS } from './lab-authoring';
import type { LabChannel, LabProfile } from './labs';

export type DisplayChannel = {
  id: string; label: string; csv_name: string; default_visible?: boolean;
  /** The lab channel a trace draws (its own ID unless it is one LED state of a channel). */
  parentId?: string;
  /** Set only on a trace of one LED state of an LED-sequenced channel: that state, 0–3. */
  ledState?: number;
  /** Every trace of an LED-sequenced lab, whose records carry their LED state in `values[0]`. */
  ledSequenced?: boolean;
};
export type VisibleChannelMap = Record<string, boolean>;
export type PlotGroup = { id: string; channelIds: string[] };
export type PlotSeries = DisplayChannel & { color: string };
/** A rendered plot plus its slot number in the assignment selectors. */
export type NumberedPlotGroup = PlotGroup & { slot: number };
/**
 * A trace as the page plots it. `valueIndex` is its value's index in each
 * record's `values`; `parentId` is the lab channel whose display unit and
 * calibration it uses. Legacy pulse-ox raw fields are not lab channels.
 */
export type PlotChannel = DisplayChannel & {
  pin?: string; units?: string; allowed_conversions?: string[]; default_display_unit?: string;
  valueIndex: number;
  parentId: string;
  /** A legacy 4-state raw field (RED/DARK/IR/DARK of TX or RX): counts and volts only. */
  rawPulseField?: boolean;
};
/** One "Show by LED state" choice (display-only) of an LED-sequenced lab's channel. */
export type LedSplitChoice = { id: string; label: string; split: boolean };

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

/**
 * The same colour families by LED state (RED, DARK 1, IR, DARK 2) for the
 * traces of an LED-sequenced channel shown by LED state. Like TX and RX
 * above, alternate lab channels use the darker and the lighter shade.
 */
const ledStateColors: ReadonlyArray<readonly [string, string]> = [
  [pulseOxColors.red_tx, pulseOxColors.red_rx],
  [pulseOxColors.dark1_tx, pulseOxColors.dark1_rx],
  [pulseOxColors.ir_tx, pulseOxColors.ir_rx],
  [pulseOxColors.dark2_tx, pulseOxColors.dark2_rx]
];

/**
 * Full-rate (not split) channels of an LED-sequenced lab. The same palette
 * without the maroon and purple, which read as the RED and IR traces they
 * may share a plot with: a blue and a green take their places.
 */
const ledSequencedPlotColors = ['#002855', '#B07800', '#007A78', '#1F6FB2', '#3A6B12', '#C65E00'];

export function seriesColor(index: number): string {
  return plotColors[index % plotColors.length];
}

/** The colour of a full-rate channel of an LED-sequenced lab, by its position. */
export function ledSequencedSeriesColor(index: number): string {
  return ledSequencedPlotColors[index % ledSequencedPlotColors.length];
}

/** The lab channel a trace belongs to. */
export function traceParentId(channel: DisplayChannel): string {
  return channel.parentId ?? channel.id;
}

/** The lab channels behind a list of traces, in first-appearance order. */
function parentOrder(channels: DisplayChannel[]): string[] {
  return [...new Set(channels.map(traceParentId))];
}

/**
 * The colour of one channel. It depends on the channel itself (the position
 * of its lab channel among all of the lab's plot channels, its pulse-ox
 * role, or its LED state), never on which traces are visible, which plot it
 * was assigned to, or whether another channel is shown by LED state, so a
 * signal keeps its colour when others are hidden or rearranged.
 */
export function channelColor(channels: DisplayChannel[], channelId: string): string {
  if (pulseOxColors[channelId]) return pulseOxColors[channelId];
  const channel = channels.find((candidate) => candidate.id === channelId);
  const parentIndex = Math.max(0, parentOrder(channels).indexOf(channel ? traceParentId(channel) : channelId));
  const family = channel?.ledState === undefined ? undefined : ledStateColors[channel.ledState];
  if (family) return family[parentIndex % 2];
  return channel?.ledSequenced ? ledSequencedSeriesColor(parentIndex) : seriesColor(parentIndex);
}

/**
 * A plot's heading: its traces joined by " + ", with the LED-state traces of
 * one channel named once ("TX — RED, IR + RX"), in first-appearance order.
 * Display-only.
 */
export function plotGroupHeading(channels: DisplayChannel[], channelIds: string[]): string {
  const parts: Array<{ key: string; label: string; states: string[] }> = [];
  for (const id of channelIds) {
    const channel = channels.find((candidate) => candidate.id === id);
    const state = channel?.ledState === undefined ? undefined : LED_STATE_LABELS[channel.ledState];
    const suffix = state === undefined ? '' : ` — ${state}`;
    if (!channel || state === undefined || !channel.label.endsWith(suffix)) {
      parts.push({ key: `trace:${id}`, label: channel?.label ?? id, states: [] });
      continue;
    }
    const key = `led:${traceParentId(channel)}`;
    const existing = parts.find((part) => part.key === key);
    if (existing) existing.states.push(state);
    else parts.push({ key, label: channel.label.slice(0, -suffix.length), states: [state] });
  }
  return parts.map((part) => part.states.length ? `${part.label} — ${part.states.join(', ')}` : part.label).join(' + ');
}

export function visibleChannels<T extends DisplayChannel>(channels: T[], selected: string[]): T[] {
  return channels.filter((channel) => selected.includes(channel.id));
}

/** Series currently visible in one display-only plot group, in uPlot order. */
export function visiblePlotSeries<T extends DisplayChannel>(channels: T[], selected: string[]): Array<T & { color: string }> {
  return visibleChannels(channels, selected).map((channel) => ({
    ...channel,
    color: channelColor(channels, channel.id)
  }));
}

/** The eight raw pulse-ox fields, in record order. One stable array. */
export const PULSEOX_PLOT_CHANNELS: readonly PlotChannel[] = Object.freeze([
  { id: 'red_tx', label: 'TX Red', csv_name: 'red_TX' },
  { id: 'dark1_tx', label: 'TX Dark 1', csv_name: 'dark1_TX' },
  { id: 'ir_tx', label: 'TX IR', csv_name: 'ir_TX' },
  { id: 'dark2_tx', label: 'TX Dark 2', csv_name: 'dark2_TX' },
  { id: 'red_rx', label: 'RX Red', csv_name: 'red_RX' },
  { id: 'dark1_rx', label: 'RX Dark 1', csv_name: 'dark1_RX' },
  { id: 'ir_rx', label: 'RX IR', csv_name: 'ir_RX' },
  { id: 'dark2_rx', label: 'RX Dark 2', csv_name: 'dark2_RX' }
].map((field, valueIndex): PlotChannel => Object.freeze({
  ...field,
  allowed_conversions: ['counts_volts'],
  valueIndex,
  parentId: field.id,
  rawPulseField: true
})));

/** The legacy 4-state pulse-ox mode, whose records hold the eight raw TX/RX fields. */
export function isPulseOxProfile(profile: LabProfile | undefined): boolean {
  return profile?.acquisition.acquisition_mode === 'pulseox_4state';
}

/** The LED-sequenced pulse-ox mode, whose records are `[led_state, ch1..chN]`. */
export function isLedSequencedProfile(profile: LabProfile | undefined): boolean {
  return profile?.acquisition.acquisition_mode === 'pulseox_sequenced';
}

/** The ID of the trace that draws one LED state of an LED-sequenced channel. */
export function ledTraceId(ledState: number, channelId: string): string {
  return `led:${ledState}:${channelId}`;
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

/** Whether an LED-sequenced channel is shown by LED state: the viewer's choice, else the lab's default. */
function shownByLedState(channel: LabChannel, splitOverrides: Record<string, boolean> | undefined): boolean {
  return splitOverrides?.[channel.id] ?? channel.split_by_led_state === true;
}

/**
 * The signals the live plots draw for a lab, in record (`values`) order.
 *
 * - Simultaneous labs: one trace per channel, `valueIndex` = its position.
 * - Legacy 4-state pulse-ox: the eight raw TX/RX fields.
 * - LED-sequenced pulse-ox: `values[0]` is the LED state, so channel i is at
 *   `valueIndex` 1 + i. A channel shown by LED state (`split_by_led_state`,
 *   or the display-only `splitOverrides` choice) becomes four traces
 *   `led:<state>:<id>`, labelled "TX — RED" and so on; any other channel is
 *   one trace at the full record rate.
 *
 * Display-only: which traces exist never changes acquisition or the raw data.
 */
export function plotChannelsFor(profile: LabProfile | undefined, splitOverrides?: Record<string, boolean>): PlotChannel[] {
  if (!profile) return [];
  if (isPulseOxProfile(profile)) return [...PULSEOX_PLOT_CHANNELS];
  const sequenced = isLedSequencedProfile(profile);
  return profileChannels(profile).flatMap((channel, index): PlotChannel[] => {
    const trace: PlotChannel = sequenced
      ? { ...channel, valueIndex: index + 1, parentId: channel.id, ledSequenced: true }
      : { ...channel, valueIndex: index, parentId: channel.id };
    if (!sequenced || !shownByLedState(channel, splitOverrides)) return [trace];
    return LED_STATE_LABELS.map((state, ledState) => ({
      ...trace,
      id: ledTraceId(ledState, channel.id),
      label: `${channel.label} — ${state}`,
      ledState
    }));
  });
}

/**
 * The fields that have a display unit: the lab's channels (a channel shown
 * by LED state has one unit for its four traces), or the eight legacy raw
 * pulse-ox fields. Units, calibrations, and the unit selectors are keyed by
 * these IDs (a trace's `parentId`).
 */
export function unitChannelsFor(profile: LabProfile | undefined): PlotChannel[] {
  return plotChannelsFor(profile, Object.fromEntries((profile?.acquisition.channels ?? []).map((channel) => [channel.id, false])));
}

/**
 * The "Show by LED state" choices of an LED-sequenced lab, one per channel
 * (none for other labs). Display-only: a choice changes which traces are
 * drawn, never the recording.
 */
export function ledSplitChoices(profile: LabProfile | undefined, splitOverrides?: Record<string, boolean>): LedSplitChoice[] {
  if (!isLedSequencedProfile(profile)) return [];
  return profileChannels(profile).map((channel) => ({
    id: channel.id,
    label: channel.label,
    split: shownByLedState(channel, splitOverrides)
  }));
}

/**
 * Identifies the plotted lab's layout: its lab ID, acquisition mode, and lab
 * channels. Visibility, plots, and units start again only when this changes;
 * showing a channel by LED state keeps them (see `remapTraceLayout`).
 */
export function plotLayoutKey(profile: LabProfile | undefined): string {
  if (!profile) return '';
  return `${profile.profile_id}:${profile.acquisition.acquisition_mode}:${unitChannelsFor(profile).map((channel) => channel.id).join('|')}`;
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

const LEGACY_TX_FIELDS = ['red_tx', 'dark1_tx', 'ir_tx', 'dark2_tx'];
const LEGACY_RX_FIELDS = ['red_rx', 'dark1_rx', 'ir_rx', 'dark2_rx'];

/**
 * Default, display-only layout for the current app session.
 *
 * - Legacy 4-state pulse-ox: one TX and one RX plot of four raw fields.
 * - LED-sequenced labs: one plot per lab channel; a channel shown by LED
 *   state keeps its four traces in its plot.
 * - ECG: one plot; other labs: one independently scaling plot per signal.
 */
export function defaultPlotGroups(category: string | undefined, channels: DisplayChannel[]): PlotGroup[] {
  if (!channels.length) return [];
  const legacyPulse = channels.some((channel) => (channel as Partial<PlotChannel>).rawPulseField)
    || (category === 'course_pulseox' && channels.some((channel) => [...LEGACY_TX_FIELDS, ...LEGACY_RX_FIELDS].includes(channel.id)));
  if (legacyPulse) {
    return [
      { id: groupId(0), channelIds: channels.filter((channel) => LEGACY_TX_FIELDS.includes(channel.id)).map((channel) => channel.id) },
      { id: groupId(1), channelIds: channels.filter((channel) => LEGACY_RX_FIELDS.includes(channel.id)).map((channel) => channel.id) }
    ];
  }
  if (channels.some((channel) => channel.ledSequenced || channel.ledState !== undefined)) {
    return parentOrder(channels).map((parent, index) => ({
      id: groupId(index),
      channelIds: channels.filter((channel) => traceParentId(channel) === parent).map((channel) => channel.id)
    }));
  }
  // ECG is easiest to read as one trace. General Analog and the other course
  // profiles begin with one independently scaling plot per recorded signal.
  if (channels.length === 1 || category === 'course_ecg' || category === 'ecg') {
    return [{ id: groupId(0), channelIds: channels.map((channel) => channel.id) }];
  }
  return channels.map((channel, index) => ({ id: groupId(index), channelIds: [channel.id] }));
}

/**
 * The plots a lab starts with: its own `plot_defaults` in every mode, each
 * lab channel ID standing for all of its traces (a channel shown by LED
 * state brings its four), else `defaultPlotGroups`. Display-only.
 */
export function initialPlotGroups(profile: LabProfile | undefined, channels: DisplayChannel[]): PlotGroup[] {
  const defaults = profile?.plot_defaults?.groups ?? [];
  if (!defaults.length) return defaultPlotGroups(profile?.category, channels);
  return normalizePlotGroups(channels, defaults.map((group, index) => ({
    id: `profile-${index + 1}`,
    channelIds: group.channel_ids.flatMap((id) => channels
      .filter((channel) => channel.id === id || traceParentId(channel) === id)
      .map((channel) => channel.id))
  })));
}

/**
 * Carries the display layout over when the traces of the same lab change
 * (a channel is shown, or no longer shown, by LED state). A trace that still
 * exists keeps its visibility and plot. A new trace takes the plot of its lab
 * channel's earlier traces (the first such plot), and is visible when any of
 * them was. The number of plots is kept. Display-only.
 */
export function remapTraceLayout(
  previous: DisplayChannel[],
  groups: PlotGroup[],
  visibility: VisibleChannelMap,
  next: DisplayChannel[]
): { groups: PlotGroup[]; visibility: VisibleChannelMap } {
  const previousIds = new Set(previous.map((channel) => channel.id));
  const previousParent = new Map(previous.map((channel) => [channel.id, traceParentId(channel)]));
  const nextVisibility: VisibleChannelMap = Object.fromEntries(next.map((channel) => {
    if (previousIds.has(channel.id)) return [channel.id, visibility[channel.id] !== false];
    const earlier = previous.filter((candidate) => traceParentId(candidate) === traceParentId(channel));
    return [channel.id, earlier.length ? earlier.some((candidate) => visibility[candidate.id] !== false) : channel.default_visible !== false];
  }));
  const remapped = normalizePlotGroups(previous, groups).map((group) => ({
    ...group,
    channelIds: group.channelIds.flatMap((id) => next
      .filter((channel) => channel.id === id || (!previousIds.has(channel.id) && traceParentId(channel) === previousParent.get(id)))
      .map((channel) => channel.id))
  }));
  return { groups: normalizePlotGroups(next, remapped), visibility: nextVisibility };
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
