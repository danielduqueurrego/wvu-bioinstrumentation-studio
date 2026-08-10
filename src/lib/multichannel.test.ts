import { describe, expect, it } from 'vitest';
import {
  assignChannelToPlot,
  defaultPlotGroups,
  hasUniqueAnalogPins,
  initialTraceVisibility,
  onePlotPerSignal,
  overlayAll,
  setPlotGroupCount,
  pulseoxAmbientSubtractedPreview,
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
      { id: 'red_tx', label: 'RED TX', csv_name: 'red_tx' },
      { id: 'ir_tx', label: 'IR TX', csv_name: 'ir_tx' },
      { id: 'red_rx', label: 'RED RX', csv_name: 'red_rx' },
      { id: 'ir_rx', label: 'IR RX', csv_name: 'ir_rx' }
    ]).map((group) => group.channelIds)).toEqual([['red_tx', 'ir_tx'], ['red_rx', 'ir_rx']]);
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

  it('preserves the raw pulse layout and derives preview subtraction explicitly', () => {
    expect(pulseoxAmbientSubtractedPreview([100, 10, 120, 20, 200, 30, 240, 40]))
      .toEqual([90, 100, 170, 200]);
    expect(pulseoxAmbientSubtractedPreview([100, 10])).toEqual([]);
  });

  it('accepts one through six unique A0–A5 general-development pins only', () => {
    expect(hasUniqueAnalogPins(['A0', 'A1', 'A5'])).toBe(true);
    expect(hasUniqueAnalogPins(['A0', 'A0'])).toBe(false);
    expect(hasUniqueAnalogPins(['A6'])).toBe(false);
  });
});
