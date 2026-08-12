import { describe, expect, it } from 'vitest';
import {
  assignChannelToPlot,
  defaultPlotGroups,
  hasUniqueAnalogPins,
  initialTraceVisibility,
  onePlotPerSignal,
  overlayAll,
  setPlotGroupCount,
  setTraceVisibility,
  visiblePlotGroups,
  visibleChannelIds,
  visibleChannels
} from './multichannel';

describe('Phase 4 multi-channel UI helpers', () => {
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

  it('applies overlay and one-per-signal convenience presets without changing capture fields', () => {
    expect(overlayAll(channels)).toEqual([{ id: 'plot-1', channelIds: channels.map((channel) => channel.id) }]);
    expect(onePlotPerSignal(channels).map((group) => group.channelIds)).toEqual([
      ['raw_emg'], ['rectified_emg'], ['envelope'], ['pressure']
    ]);
  });

  it('keeps all eight raw pulse-ox phase fields in the plot arrangement', () => {
    const rawPulseChannels = [
      'red_tx', 'dark1_tx', 'ir_tx', 'dark2_tx',
      'red_rx', 'dark1_rx', 'ir_rx', 'dark2_rx'
    ].map((id) => ({ id, label: id, csv_name: id }));
    expect(defaultPlotGroups('course_pulseox', rawPulseChannels).flatMap((group) => group.channelIds))
      .toEqual(rawPulseChannels.map((channel) => channel.id));
  });

  it('accepts one through six unique A0–A5 general-development pins only', () => {
    expect(hasUniqueAnalogPins(['A0', 'A1', 'A5'])).toBe(true);
    expect(hasUniqueAnalogPins(['A0', 'A0'])).toBe(false);
    expect(hasUniqueAnalogPins(['A6'])).toBe(false);
  });
});
