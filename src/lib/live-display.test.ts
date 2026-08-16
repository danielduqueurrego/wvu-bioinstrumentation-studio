import { describe, expect, it } from 'vitest';
import {
  DEFAULT_PLOT_TIME_WINDOW_SECONDS,
  filterStartupDisplayTransient,
  formatElapsedSeconds,
  HARDWARE_STARTUP_DISPLAY_WARMUP_SECONDS,
  formatLiveDisplayValue,
  layoutEndpointLabels,
  normalizePlotTimeWindow,
  previewPlotTimeWindow
} from './live-display';

describe('live display helpers', () => {
  it('uses a five-second default and clamps committed time windows to 0.5–30 seconds', () => {
    expect(DEFAULT_PLOT_TIME_WINDOW_SECONDS).toBe(5);
    expect(normalizePlotTimeWindow(0.1)).toBe(0.5);
    expect(normalizePlotTimeWindow(0.5)).toBe(0.5);
    expect(normalizePlotTimeWindow(5)).toBe(5);
    expect(normalizePlotTimeWindow(30)).toBe(30);
    expect(normalizePlotTimeWindow(45)).toBe(30);
    expect(normalizePlotTimeWindow(2.3)).toBe(2.5);
  });

  it('preserves the last valid time window while a numeric input is temporarily blank or invalid', () => {
    expect(previewPlotTimeWindow('', 5)).toBe(5);
    expect(previewPlotTimeWindow('not a number', 10)).toBe(10);
    expect(normalizePlotTimeWindow('', 10)).toBe(10);
  });

  it('formats live endpoint values using the selected display unit', () => {
    expect(formatLiveDisplayValue(8192, 'counts')).toBe('8192');
    expect(formatLiveDisplayValue(2.3176, 'volts')).toBe('2.318 V');
    expect(formatLiveDisplayValue(9.64, 'kpa')).toBe('9.6 kPa');
    expect(formatLiveDisplayValue(72.36, 'mmhg')).toBe('72.4 mmHg');
    expect(formatLiveDisplayValue(12.345, 'calibrated', {
      adc_reference_v: 5,
      mpxv_sensor_supply_v: 5,
      channel_units: {},
      active_calibrations: [{
        schema_version: 1, calibration_id: 'load-cell', profile_id: 'general', channel_id: 'load',
        calibration_type: 'linear', input_quantity: 'volts', output_quantity: 'mass', output_units: 'g',
        parameters: {}, created_at: '', label: 'Load'
      }]
    }, 'load')).toBe('12.35 g');
  });

  it('formats live x-axis values as elapsed recording seconds, not dates', () => {
    expect(formatElapsedSeconds(0)).toBe('0 s');
    expect(formatElapsedSeconds(2.5)).toBe('2.5 s');
    expect(formatElapsedSeconds(12.4)).toBe('12.4 s');
  });

  it('removes only a converging first hardware ADC transient from the display', () => {
    const samples = Array.from({ length: 8 }, (_, sequence) => ({
      sequence,
      timestamp_us: sequence * 1_000,
      values: [sequence === 0 ? 5_061 : [5_371, 5_518, 5_632, 5_672, 5_665, 5_660, 5_662][sequence - 1]]
    }));
    const filtered = filterStartupDisplayTransient(samples, [0], 14, true);
    expect(filtered[0]?.sequence).toBe(1);
    expect(filtered).toHaveLength(7);
    expect(filterStartupDisplayTransient(samples, [0], 14, false)).toBe(samples);
  });

  it('uses the recording origin to suppress the first hardware warm-up interval after decimation', () => {
    const origin = 1_000_000;
    const samples = [
      { sequence: 0, timestamp_us: origin, values: [5_000] },
      { sequence: 10, timestamp_us: origin + 10_000, values: [5_500] },
      { sequence: 100, timestamp_us: origin + 100_000, values: [5_600] },
      { sequence: 200, timestamp_us: origin + 200_000, values: [5_610] }
    ];
    const filtered = filterStartupDisplayTransient(samples, [0], 14, true, origin);
    expect(filtered.map((sample) => sample.sequence)).toEqual([100, 200]);
    expect(HARDWARE_STARTUP_DISPLAY_WARMUP_SECONDS).toBe(0.1);
  });

  it('does not remove a startup point when it is not an isolated converging transient', () => {
    const samples = Array.from({ length: 8 }, (_, sequence) => ({
      sequence,
      timestamp_us: sequence * 1_000,
      values: [5_000 + (sequence % 2 ? 100 : -100)]
    }));
    expect(filterStartupDisplayTransient(samples, [0], 14, true)).toBe(samples);
  });

  it('keeps clustered endpoint labels distinct and inside the plot bounds', () => {
    const labels = layoutEndpointLabels([
      { id: 'a', naturalTop: 100 },
      { id: 'b', naturalTop: 102 },
      { id: 'c', naturalTop: 104 },
      { id: 'd', naturalTop: 106 }
    ], 0, 180);
    const tops = labels.map((label) => label.top ?? 0);
    expect(tops).toEqual([...tops].sort((left, right) => left - right));
    expect(tops.every((top) => top >= 10 && top <= 170)).toBe(true);
    expect(tops.every((top, index) => index === 0 || top - tops[index - 1] >= 20)).toBe(true);
  });

  it('keeps well-separated endpoint labels near their natural positions', () => {
    const labels = layoutEndpointLabels([
      { id: 'top', naturalTop: 25 },
      { id: 'middle', naturalTop: 90 },
      { id: 'bottom', naturalTop: 155 }
    ], 0, 180);
    expect(labels.map((label) => label.top)).toEqual([25, 90, 155]);
  });
});
