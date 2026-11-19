import { describe, expect, it } from 'vitest';
import {
  axisTickDecimals,
  DEFAULT_PLOT_TIME_WINDOW_SECONDS,
  filterStartupDisplayTransient,
  formatElapsedSeconds,
  IDLE_STATUS_POLL_MS,
  plotAreaCssBounds,
  shouldFetchDisplayData,
  statusPollDue,
  HARDWARE_STARTUP_DISPLAY_WARMUP_SECONDS,
  formatLiveDisplayValue,
  layoutEndpointLabels,
  newestTraceSample,
  normalizePlotTimeWindow,
  previewPlotTimeWindow,
  sampleInTrace,
  traceRawValues
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

  it('chooses x-axis tick decimals from the tick increment uPlot picked', () => {
    expect(axisTickDecimals(1)).toBe(0);
    expect(axisTickDecimals(2)).toBe(0);
    expect(axisTickDecimals(0.5)).toBe(1);
    expect(axisTickDecimals(0.1)).toBe(1);
    expect(axisTickDecimals(0.25)).toBe(2);
    expect(axisTickDecimals(0.05)).toBe(2);
    expect(axisTickDecimals(0.025)).toBe(3);
    expect(axisTickDecimals(Number.NaN)).toBe(1);
    expect(axisTickDecimals(0)).toBe(1);
  });

  it('labels 0.25 s ticks evenly instead of rounding them to 0.3/0.5/0.8', () => {
    const splits = [0, 0.25, 0.5, 0.75, 1, 1.25];
    expect(splits.map((value) => formatElapsedSeconds(value, 0.25)))
      .toEqual(['0 s', '0.25 s', '0.50 s', '0.75 s', '1.00 s', '1.25 s']);
  });

  it('keeps short-window ticks distinct instead of repeating one-decimal labels', () => {
    const labels = [0.025, 0.05, 0.075, 0.1].map((value) => formatElapsedSeconds(value, 0.025));
    expect(labels).toEqual(['0.025 s', '0.050 s', '0.075 s', '0.100 s']);
    expect(new Set(labels).size).toBe(labels.length);
    // Before, any tick below 0.05 s was printed as "0 s".
    expect(formatElapsedSeconds(0.02, 0.01)).toBe('0.02 s');
  });

  it('prints whole-second ticks without decimals and never shows a negative zero', () => {
    expect([0, 1, 2, 3].map((value) => formatElapsedSeconds(value, 1))).toEqual(['0 s', '1 s', '2 s', '3 s']);
    expect(formatElapsedSeconds(-0.0001, 0.25)).toBe('0 s');
    expect(formatElapsedSeconds(Number.NaN, 1)).toBe('—');
  });

  it('converts the uPlot plotting area from canvas pixels to CSS pixels', () => {
    expect(plotAreaCssBounds({ top: 15, height: 300 }, 1)).toEqual({ top: 15, bottom: 315 });
    // 150 % Windows scaling: canvas pixels are 1.5 CSS pixels each.
    expect(plotAreaCssBounds({ top: 22.5, height: 450 }, 1.5)).toEqual({ top: 15, bottom: 315 });
    expect(plotAreaCssBounds({ top: 10, height: 100 }, 0)).toEqual({ top: 10, bottom: 110 });
  });

  it('keeps endpoint badges inside the CSS plotting area at any display scaling', () => {
    const bounds = plotAreaCssBounds({ top: 22.5, height: 450 }, 1.5);
    // valToPos is relative to the plotting area, so add its top.
    const labels = layoutEndpointLabels([{ id: 'a', naturalTop: bounds.top + 300 }], bounds.top, bounds.bottom);
    expect(labels[0]?.top).toBe(bounds.bottom - 10);
  });

  it('fetches display data only while a session can produce frames, plus once when it ends', () => {
    expect(shouldFetchDisplayData({ state: 'Acquiring', previousState: 'Acquiring', refreshRequested: false })).toBe(true);
    expect(shouldFetchDisplayData({ state: 'Stopping', previousState: 'Acquiring', refreshRequested: false })).toBe(true);
    // The final frames after the recording ends.
    expect(shouldFetchDisplayData({ state: 'Disconnected', previousState: 'Stopping', refreshRequested: false })).toBe(true);
    expect(shouldFetchDisplayData({ state: 'Faulted', previousState: 'Acquiring', refreshRequested: false })).toBe(true);
    // Idle: no fetch or redraw.
    expect(shouldFetchDisplayData({ state: 'Disconnected', previousState: 'Disconnected', refreshRequested: false })).toBe(false);
    expect(shouldFetchDisplayData({ state: 'Faulted', previousState: 'Faulted', refreshRequested: false })).toBe(false);
    // A time-window change (or the first poll) still refreshes the plot.
    expect(shouldFetchDisplayData({ state: 'Disconnected', previousState: 'Disconnected', refreshRequested: true })).toBe(true);
  });

  it('reads the session every tick while recording and a few times a second while idle', () => {
    expect(statusPollDue(true, 1_000, 1_040)).toBe(true);
    expect(statusPollDue(false, 1_000, 1_040)).toBe(false);
    expect(statusPollDue(false, 1_000, 1_000 + IDLE_STATUS_POLL_MS)).toBe(true);
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

describe('LED-sequenced traces (display-only)', () => {
  // Records [led_state, tx, rx]: the LED state is sequence % 4.
  const levels = [3_000, 500, 2_000, 520];
  const records = Array.from({ length: 12 }, (_, sequence) => ({
    sequence,
    timestamp_us: sequence * 1_000,
    values: [sequence % 4, levels[sequence % 4] + sequence, 7_000 + sequence]
  }));

  it('draws a trace of one LED state as null at the other states, and a full-rate trace at every record', () => {
    expect(traceRawValues(records.slice(0, 6), { valueIndex: 1, ledState: 0 })).toEqual([3_000, null, null, null, 3_004, null]);
    expect(traceRawValues(records.slice(0, 6), { valueIndex: 1, ledState: 2 })).toEqual([null, null, 2_002, null, null, null]);
    expect(traceRawValues(records.slice(0, 3), { valueIndex: 2 })).toEqual([7_000, 7_001, 7_002]);
    // A record without that value keeps the old behaviour (0), never another field.
    expect(traceRawValues([{ values: [0] }], { valueIndex: 1, ledState: 0 })).toEqual([0]);
    expect(sampleInTrace(records[5], { valueIndex: 1, ledState: 1 })).toBe(true);
    expect(sampleInTrace(records[5], { valueIndex: 1, ledState: 2 })).toBe(false);
    expect(sampleInTrace(records[5], { valueIndex: 1 })).toBe(true);
  });

  it('puts each endpoint badge on the newest record of its own LED state', () => {
    expect(newestTraceSample(records, { valueIndex: 1, ledState: 0 })?.sequence).toBe(8);
    expect(newestTraceSample(records, { valueIndex: 1, ledState: 3 })?.sequence).toBe(11);
    expect(newestTraceSample(records, { valueIndex: 2 })?.sequence).toBe(11);
    expect(newestTraceSample(records.slice(0, 2), { valueIndex: 1, ledState: 2 })).toBeUndefined();
  });

  it('checks the startup transient within each LED state, never across states', () => {
    const steady = Array.from({ length: 32 }, (_, sequence) => ({
      sequence,
      timestamp_us: sequence * 1_000,
      values: [sequence % 4, [12_000, 400, 9_000, 420][sequence % 4]]
    }));
    // Checked across every record, the states' different levels would look like a transient.
    expect(filterStartupDisplayTransient(steady, [1], 14, true)).toHaveLength(31);
    // Within each LED state, the steady levels are kept.
    for (const ledState of [0, 1, 2, 3]) {
      expect(filterStartupDisplayTransient(steady, [{ valueIndex: 1, ledState }], 14, true)).toBe(steady);
    }
    // A real settling RED transient on the first record is still removed.
    const settling = steady.map((record) => record.sequence % 4 === 0
      ? { ...record, values: [0, [9_000, 11_200, 11_800, 11_950, 12_000, 12_000, 12_000, 12_000][record.sequence / 4]] }
      : record);
    const filtered = filterStartupDisplayTransient(settling, [{ valueIndex: 1, ledState: 0 }], 14, true);
    expect(filtered[0]?.sequence).toBe(1);
    expect(filtered).toHaveLength(31);
    // A trace of another LED state does not draw the first record, so it never removes it.
    expect(filterStartupDisplayTransient(settling, [{ valueIndex: 1, ledState: 2 }], 14, true)).toBe(settling);
  });
});
