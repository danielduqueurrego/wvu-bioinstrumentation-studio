/** Small pure helpers shared by the profile-aware Acquisition page and plot. */
export type DisplayChannel = { id: string; label: string; csv_name: string; default_visible?: boolean };
export type VisibleChannelMap = Record<string, boolean>;
export type PlotGroup = { id: string; channelIds: string[] };
export type PlotSeries = DisplayChannel & { color: string };

// One shared color mapping for each LivePlot instance.  The uPlot stroke and
// the Svelte legend both consume this helper so a legend can never drift from
// the line it identifies.
const plotColors = ['#002855', '#EEAA00', '#007A78', '#9D2235', '#5D4E99', '#C65E00'];

export function seriesColor(index: number): string {
  return plotColors[index % plotColors.length];
}

export function visibleChannels(channels: DisplayChannel[], selected: string[]): DisplayChannel[] {
  return channels.filter((channel) => selected.includes(channel.id));
}

/** Series currently visible in one display-only plot group, in uPlot order. */
export function visiblePlotSeries(channels: DisplayChannel[], selected: string[]): PlotSeries[] {
  return visibleChannels(channels, selected).map((channel, index) => ({
    ...channel,
    color: seriesColor(index)
  }));
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
  return normalizePlotGroups(channels, groups)
    .map((group) => ({ ...group, channelIds: group.channelIds.filter((id) => visibility[id] !== false) }))
    .filter((group) => group.channelIds.length > 0);
}

export function hasUniqueAnalogPins(pins: string[]): boolean {
  return pins.length > 0 && pins.length <= 6 && new Set(pins).size === pins.length
    && pins.every((pin) => /^A[0-5]$/.test(pin));
}
