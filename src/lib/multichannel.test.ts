import { describe, expect, it } from 'vitest';
import {
  defaultPlotLayout,
  hasUniqueAnalogPins,
  initialTraceVisibility,
  pulseoxAmbientSubtractedPreview,
  setTraceVisibility,
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

  it('defaults multi-signal capture to stacked plots while preserving single-signal overlay', () => {
    expect(defaultPlotLayout(1)).toBe('overlay');
    expect(defaultPlotLayout(4)).toBe('stacked');
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
