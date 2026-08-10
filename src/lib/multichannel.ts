/** Small pure helpers shared by the profile-aware Acquisition page and plot. */
export type DisplayChannel = { id: string; label: string; csv_name: string };
export type VisibleChannelMap = Record<string, boolean>;
export type PlotLayout = 'overlay' | 'stacked';

export function visibleChannels(channels: DisplayChannel[], selected: string[]): DisplayChannel[] {
  return channels.filter((channel) => selected.includes(channel.id));
}

/**
 * The profile owns the set of possible traces; this creates a fresh, single source of
 * truth for their visibility when that profile changes.  A checkbox must update this
 * map directly rather than being reconciled back to "all visible" on each render.
 */
export function initialTraceVisibility(channels: DisplayChannel[]): VisibleChannelMap {
  return Object.fromEntries(channels.map((channel) => [channel.id, true]));
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

export function defaultPlotLayout(channelCount: number): PlotLayout {
  return channelCount > 1 ? 'stacked' : 'overlay';
}

/**
 * Phase 4 pulse-ox preview only. Raw LED-state counts are never altered in BMEG/CSV.
 */
export function pulseoxAmbientSubtractedPreview(values: number[]): number[] {
  if (values.length < 8) return [];
  return [
    values[0] - values[1],
    values[2] - values[3],
    values[4] - values[5],
    values[6] - values[7]
  ];
}

export function hasUniqueAnalogPins(pins: string[]): boolean {
  return pins.length > 0 && pins.length <= 6 && new Set(pins).size === pins.length
    && pins.every((pin) => /^A[0-5]$/.test(pin));
}
