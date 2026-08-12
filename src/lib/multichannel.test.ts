import { describe, expect, it } from 'vitest';
import {
  assignChannelToPlot,
  defaultPlotGroups,
  hasUniqueAnalogPins,
  initialTraceVisibility,
  onePlotPerSignal,
  overlayAll,
  seriesColor,
  setPlotGroupCount,
  setTraceVisibility,
  shouldShowPlotLegend,
  visiblePlotSeries,
  visiblePlotGroups,
  visibleChannelIds,
  visibleChannels
} from './multichannel';

describe('multi-channel UI helpers', () => {
  const channels = [
    { id: 'raw_emg', label: 'Raw EMG', csv_name: 'raw_emg_counts' },
    { id: 'rectified_emg', label: 'Rectified EMG', csv_name: 'rectified_emg_counts' },
    { id: 'envelope', label: 'Envelope', csv_name: 'emg_envelope_counts' },
    { id: 'pressure', label: 'Pressure / Force Surrogate', csv_name: 'pressure_counts' }
  ];

  it('keeps each selected synchronized trace visible independently', () => {
    expect(visibleChannels(channels, ['raw_emg', 'envelope']).map((channel) => channel.id))
      .toEqual(['raw_emg', 'envelope']);
  });

  it('uses one visibility map so hiding a trace does not reset the other live traces', () => {
    let visibility = initialTraceVisibility(channels);
    visibility = setTraceVisibility(visibility, 'raw_emg', false);
    expect(visibility.raw_emg).toBe(false);
    expect(visibleChannelIds(channels, visibility)).toEqual([
      'rectified_emg', 'envelope', 'pressure'
    ]);
    visibility = setTraceVisibility(visibility, 'raw_emg', true);
    expect(visibleChannelIds(channels, visibility)).toEqual(channels.map((channel) => channel.id));
  });

  it('uses course-specific display-only plot-group defaults', () => {
    expect(defaultPlotGroups('course_ecg', channels.slice(0, 1))).toHaveLength(1);
    expect(defaultPlotGroups('course_emg_force', channels)).toHaveLength(4);
    expect(defaultPlotGroups('development', channels)).toHaveLength(4);
    expect(defaultPlotGroups('course_blood_pressure', channels.slice(0, 3))).toHaveLength(3);
    expect(defaultPlotGroups('course_pulseox', [
      { id: 'red_tx', label: 'TX Red', csv_name: 'red_TX' },
      { id: 'dark1_tx', label: 'TX Dark 1', csv_name: 'dark1_TX' },
      { id: 'ir_tx', label: 'TX IR', csv_name: 'ir_TX' },
      { id: 'dark2_tx', label: 'TX Dark 2', csv_name: 'dark2_TX' },
      { id: 'red_rx', label: 'RX Red', csv_name: 'red_RX' },
      { id: 'dark1_rx', label: 'RX Dark 1', csv_name: 'dark1_RX' },
      { id: 'ir_rx', label: 'RX IR', csv_name: 'ir_RX' },
      { id: 'dark2_rx', label: 'RX Dark 2', csv_name: 'dark2_RX' }
    ]).map((group) => group.channelIds)).toEqual([
      ['red_tx', 'dark1_tx', 'ir_tx', 'dark2_tx'],
      ['red_rx', 'dark1_rx', 'ir_rx', 'dark2_rx']
    ]);
  });

  it('merges removed plots deterministically and preserves assignments while hidden', () => {
    let groups = onePlotPerSignal(channels);
    groups = setPlotGroupCount(channels, groups, 2);
    expect(groups.map((group) => group.channelIds)).toEqual([
      ['raw_emg'],
      ['rectified_emg', 'envelope', 'pressure']
    ]);
    groups = assignChannelToPlot(channels, groups, 'rectified_emg', 0);
    expect(groups.map((group) => group.channelIds)).toEqual([
      ['raw_emg', 'rectified_emg'],
      ['envelope', 'pressure']
    ]);
    const visibility = setTraceVisibility(initialTraceVisibility(channels), 'pressure', false);
    expect(visiblePlotGroups(channels, groups, visibility).map((group) => group.channelIds))
      .toEqual([['raw_emg', 'rectified_emg'], ['envelope']]);
  });

  it('keeps empty slots available for assignment but never asks the UI to render an empty plot', () => {
    const groups = setPlotGroupCount(channels, overlayAll(channels), 4);
    expect(groups).toHaveLength(4);
    expect(visiblePlotGroups(channels, groups, initialTraceVisibility(channels))).toEqual([
      { id: 'plot-1', channelIds: channels.map((channel) => channel.id) }
    ]);
  });

  it('shows a per-plot legend only when that plot has at least two visible series', () => {
    const one = visiblePlotSeries(channels, ['raw_emg']);
    const two = visiblePlotSeries(channels, ['raw_emg', 'rectified_emg']);
    const four = visiblePlotSeries(channels, channels.map((channel) => channel.id));

    expect(shouldShowPlotLegend(one)).toBe(false);
    expect(shouldShowPlotLegend(two)).toBe(true);
    expect(shouldShowPlotLegend(four)).toBe(true);
    expect(two.map((series) => series.label)).toEqual(['Raw EMG', 'Rectified EMG']);
    expect(four).toHaveLength(4);
  });

  it('updates legend membership and exact stroke colors when signals are hidden or reassigned', () => {
    const overlay = visiblePlotSeries(channels, ['raw_emg', 'rectified_emg']);
    expect(overlay.map((series) => series.color)).toEqual([seriesColor(0), seriesColor(1)]);

    const oneRemaining = visiblePlotSeries(channels, ['raw_emg']);
    expect(shouldShowPlotLegend(oneRemaining)).toBe(false);

    const firstPlot = visiblePlotSeries(channels, ['raw_emg', 'rectified_emg']);
    const secondPlot = visiblePlotSeries(channels, ['envelope', 'pressure']);
    expect(firstPlot.map((series) => series.label)).toEqual(['Raw EMG', 'Rectified EMG']);
    expect(secondPlot.map((series) => series.label)).toEqual(['Envelope', 'Pressure / Force Surrogate']);
    expect(firstPlot[0]?.color).toBe(seriesColor(0));
    expect(secondPlot[0]?.color).toBe(seriesColor(0));
  });

  it('keeps plot IDs stable while an active recording is rearranged', () => {
    let groups = onePlotPerSignal(channels);
    expect(groups.map((group) => group.id)).toEqual(['plot-1', 'plot-2', 'plot-3', 'plot-4']);

    groups = setPlotGroupCount(channels, groups, 2);
    expect(groups.map((group) => group.id)).toEqual(['plot-1', 'plot-2']);
    groups = setPlotGroupCount(channels, groups, 3);
    expect(groups.map((group) => group.id)).toEqual(['plot-1', 'plot-2', 'plot-3']);
    groups = setPlotGroupCount(channels, groups, 4);
    expect(groups.map((group) => group.id)).toEqual(['plot-1', 'plot-2', 'plot-3', 'plot-4']);
    expect(new Set(groups.map((group) => group.id)).size).toBe(groups.length);
  });

  it('keeps layout changes display-only and retains every captured channel assignment', () => {
    let groups = onePlotPerSignal(channels);
    groups = setPlotGroupCount(channels, groups, 2);
    groups = assignChannelToPlot(channels, groups, 'rectified_emg', 0);
    groups = overlayAll(channels);
    groups = onePlotPerSignal(channels);

    expect(groups.flatMap((group) => group.channelIds)).toEqual(channels.map((channel) => channel.id));
    expect(visiblePlotGroups(channels, groups, initialTraceVisibility(channels))).toHaveLength(4);
  });

  it('applies overlay and one-per-signal convenience presets without changing capture fields', () => {
    expect(overlayAll(channels)).toEqual([{ id: 'plot-1', channelIds: channels.map((channel) => channel.id) }]);
    expect(onePlotPerSignal(channels).map((group) => group.channelIds)).toEqual([
      ['raw_emg'], ['rectified_emg'], ['envelope'], ['pressure']
    ]);
  });

  it('keeps all eight raw pulse-ox phase fields in the plot arrangement', () => {
    const rawPulseChannels = [
      ['red_tx', 'TX Red'], ['dark1_tx', 'TX Dark 1'], ['ir_tx', 'TX IR'], ['dark2_tx', 'TX Dark 2'],
      ['red_rx', 'RX Red'], ['dark1_rx', 'RX Dark 1'], ['ir_rx', 'RX IR'], ['dark2_rx', 'RX Dark 2']
    ].map(([id, label]) => ({ id, label, csv_name: id }));
    expect(defaultPlotGroups('course_pulseox', rawPulseChannels).flatMap((group) => group.channelIds))
      .toEqual(rawPulseChannels.map((channel) => channel.id));
    const pulseGroups = defaultPlotGroups('course_pulseox', rawPulseChannels);
    expect(visiblePlotSeries(rawPulseChannels, pulseGroups[0]?.channelIds ?? []).map((series) => series.label))
      .toEqual(['TX Red', 'TX Dark 1', 'TX IR', 'TX Dark 2']);
    expect(visiblePlotSeries(rawPulseChannels, pulseGroups[1]?.channelIds ?? [])).toHaveLength(4);
    expect(shouldShowPlotLegend(visiblePlotSeries(rawPulseChannels, pulseGroups[0]?.channelIds ?? []))).toBe(true);
  });

  it('accepts one through six unique A0–A5 general-development pins only', () => {
    expect(hasUniqueAnalogPins(['A0', 'A1', 'A5'])).toBe(true);
    expect(hasUniqueAnalogPins(['A0', 'A0'])).toBe(false);
    expect(hasUniqueAnalogPins(['A6'])).toBe(false);
  });
});
