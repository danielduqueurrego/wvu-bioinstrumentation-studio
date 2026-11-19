import { calibrationForChannel, type DisplayUnit, type RecordingCalibration } from './calibration';

export const DEFAULT_PLOT_TIME_WINDOW_SECONDS = 5;
export const MIN_PLOT_TIME_WINDOW_SECONDS = 0.5;
export const MAX_PLOT_TIME_WINDOW_SECONDS = 30;
export const PLOT_TIME_WINDOW_STEP_SECONDS = 0.5;
export const MAX_RENDERED_DISPLAY_POINTS = 2_000;
/** Display-only ADC settling interval after a hardware START. */
export const HARDWARE_STARTUP_DISPLAY_WARMUP_SECONDS = 0.1;

type DisplaySample = { sequence: number; timestamp_us: number; values: number[] };

/**
 * Which record value a plotted trace draws: `values[valueIndex]` of every
 * record, or, for a trace of one LED state of an LED-sequenced channel, only
 * of the records whose LED state (`values[0]`) is `ledState`.
 */
export type TraceSource = { valueIndex: number; ledState?: number };

/** Whether a record belongs to a trace (always, unless the trace draws one LED state). */
export function sampleInTrace(sample: { values: number[] }, source: TraceSource): boolean {
  return source.ledState === undefined || sample.values[0] === source.ledState;
}

/**
 * A trace's raw values, one per record: `null` where the record belongs to
 * another LED state, so uPlot leaves a gap there (spanned by `spanGaps`).
 */
export function traceRawValues(samples: Array<{ values: number[] }>, source: TraceSource): Array<number | null> {
  return samples.map((sample) => sampleInTrace(sample, source) ? (sample.values[source.valueIndex] ?? 0) : null);
}

/** The newest record a trace draws, for its endpoint badge (the last record of its LED state). */
export function newestTraceSample<T extends { values: number[] }>(samples: T[], source: TraceSource): T | undefined {
  for (let index = samples.length - 1; index >= 0; index -= 1) {
    if (sampleInTrace(samples[index], source)) return samples[index];
  }
  return undefined;
}

function finiteNumber(value: string | number): number | undefined {
  if (typeof value === 'string' && !value.trim()) return undefined;
  const parsed = typeof value === 'number' ? value : Number(value);
  return Number.isFinite(parsed) ? parsed : undefined;
}

/** Keeps a temporary blank/invalid number input from changing the active plot window. */
export function previewPlotTimeWindow(value: string | number, lastValid: number): number {
  const parsed = finiteNumber(value);
  if (parsed === undefined) return lastValid;
  return Math.min(MAX_PLOT_TIME_WINDOW_SECONDS, Math.max(MIN_PLOT_TIME_WINDOW_SECONDS, parsed));
}

/** Clamps a committed value and rounds it to the supported half-second increment. */
export function normalizePlotTimeWindow(value: string | number, fallback = DEFAULT_PLOT_TIME_WINDOW_SECONDS): number {
  const clamped = previewPlotTimeWindow(value, fallback);
  const rounded = Math.round(clamped / PLOT_TIME_WINDOW_STEP_SECONDS) * PLOT_TIME_WINDOW_STEP_SECONDS;
  return Math.min(MAX_PLOT_TIME_WINDOW_SECONDS, Math.max(MIN_PLOT_TIME_WINDOW_SECONDS, rounded));
}

export function formatLiveDisplayValue(
  value: number,
  unit: DisplayUnit,
  calibration?: RecordingCalibration,
  channelId?: string
): string {
  if (!Number.isFinite(value)) return '—';
  if (unit === 'counts') return `${Math.round(value)}`;
  if (unit === 'volts') return `${value.toFixed(3)} V`;
  if (unit === 'kpa') return `${value.toFixed(1)} kPa`;
  if (unit === 'mmhg') return `${value.toFixed(1)} mmHg`;
  const suffix = calibrationForChannel(calibration?.active_calibrations ?? [], channelId ?? '')?.output_units
    || 'units';
  return `${value.toFixed(2)} ${suffix}`;
}

/**
 * Decimals needed to show every tick of an axis whose ticks are `increment`
 * apart: 1 s → 0, 0.5 s → 1, 0.25 s → 2, 0.025 s → 3. uPlot picks
 * 1/2/2.5/5 × 10ⁿ steps, so a fixed one-decimal format would print 0.25 s
 * steps as uneven 0.3/0.5/0.8 labels and 0.025 s steps as repeated labels.
 */
export function axisTickDecimals(increment: number): number {
  if (!Number.isFinite(increment) || increment <= 0) return 1;
  for (let decimals = 0; decimals < 6; decimals += 1) {
    const scaled = increment * 10 ** decimals;
    if (Math.abs(scaled - Math.round(scaled)) < 1e-6 * Math.max(1, scaled)) return decimals;
  }
  return 6;
}

/**
 * uPlot's default time scale treats values as Unix seconds. Live acquisition
 * timestamps are board-relative microseconds, so format the x-axis explicitly
 * as elapsed recording time instead of a calendar date. `increment` is the
 * tick spacing uPlot chose (the 5th argument of an axis `values` callback);
 * without it one decimal is shown.
 */
export function formatElapsedSeconds(value: number, increment?: number): string {
  if (!Number.isFinite(value)) return '—';
  const decimals = increment === undefined ? 1 : axisTickDecimals(increment);
  const text = value.toFixed(decimals);
  // Rounds to zero (including "-0.00"): show a plain origin label.
  if (Number(text) === 0) return '0 s';
  return `${text} s`;
}

/**
 * uPlot reports `bbox` in canvas (device) pixels, while `valToPos(value, 'y')`
 * returns CSS pixels relative to the plotting area. Endpoint badges are
 * positioned in CSS pixels, so convert the plotting area once here. At 125 %
 * or 150 % Windows scaling the two units differ by that ratio.
 */
export function plotAreaCssBounds(
  bbox: { top: number; height: number },
  pixelRatio: number
): { top: number; bottom: number } {
  const ratio = Number.isFinite(pixelRatio) && pixelRatio > 0 ? pixelRatio : 1;
  return { top: bbox.top / ratio, bottom: (bbox.top + bbox.height) / ratio };
}

const activeDisplayStates = new Set(['Connecting', 'Connected', 'Configured', 'Acquiring', 'Stopping']);

/** While no recording runs, the session status is read at this interval. */
export const IDLE_STATUS_POLL_MS = 250;

/**
 * Whether a status-poll tick should read the session now. A recording (or a
 * Start in progress) is followed every 40 ms tick; while idle a few reads per
 * second are enough, because every command that changes the session reads
 * a fresh status itself.
 */
export function statusPollDue(active: boolean, lastPollMs: number, nowMs: number): boolean {
  return active || nowMs - lastPollMs >= IDLE_STATUS_POLL_MS;
}

/**
 * Whether a poll should fetch and redraw the bounded display data. Only an
 * active session produces new frames; when it ends, one more fetch shows the
 * final frames. An explicit request (a time-window change, the first poll)
 * always fetches. Idle polls otherwise skip the fetch and the redraw.
 */
export function shouldFetchDisplayData(input: {
  state: string;
  previousState: string;
  refreshRequested: boolean;
}): boolean {
  return input.refreshRequested
    || activeDisplayStates.has(input.state)
    || activeDisplayStates.has(input.previousState);
}

function median(values: number[]): number {
  const sorted = [...values].sort((left, right) => left - right);
  if (!sorted.length) return 0;
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[middle - 1] + sorted[middle]) / 2
    : sorted[middle];
}

/**
 * The first hardware ADC conversion can be a settling transient immediately
 * after START. Remove it from the live display only when it is clearly
 * converging toward the next samples. Raw BMEG/CSV records are never changed.
 * The sequence check prevents this from re-running as the bounded plot window
 * advances or when plots are rearranged.
 *
 * Each trace is checked within its own LED state (`TraceSource.ledState`):
 * an LED-sequenced record is compared only with later records of the same
 * state, never with the other states' legitimately different values. A
 * number is a trace at that value index across every record.
 */
export function filterStartupDisplayTransient<T extends DisplaySample>(
  samples: T[],
  traces: Array<number | TraceSource>,
  adcBits: number,
  hardwareSource: boolean,
  originTimestampUs?: number
): T[] {
  if (!hardwareSource || !samples.length || !traces.length) return samples;

  // Once the backend has exposed the first accepted frame, use elapsed time
  // rather than the first frame currently visible in the rolling window. This
  // remains stable after decimation and after the window advances.
  if (Number.isFinite(originTimestampUs)) {
    const warmupEnd = (originTimestampUs as number) + HARDWARE_STARTUP_DISPLAY_WARMUP_SECONDS * 1_000_000;
    const warmed = samples.filter((sample) => sample.timestamp_us >= warmupEnd);
    // Keep the early points until at least one post-warmup point is available;
    // this prevents a blank plot during the first few polling cycles.
    if (warmed.length) return warmed;
  }

  if (samples.length < 8 || samples[0]?.sequence !== 0) return samples;
  const fullScale = Math.max(1, (2 ** adcBits) - 1);
  const threshold = Math.max(32, fullScale * 0.02);
  const transient = traces.some((trace) => {
    const source: TraceSource = typeof trace === 'number' ? { valueIndex: trace } : trace;
    const channelIndex = source.valueIndex;
    // Only the first record can be removed, so only a trace that draws it counts.
    const series = source.ledState === undefined ? samples : samples.filter((sample) => sampleInTrace(sample, source));
    if (series.length < 8 || series[0] !== samples[0]) return false;
    const first = series[0]?.values[channelIndex];
    if (first === undefined) return false;
    const stable = median(series.slice(4, 8).map((sample) => sample.values[channelIndex] ?? first));
    const firstDistance = Math.abs(first - stable);
    if (firstDistance <= threshold) return false;
    const approach = series.slice(1, 4).map((sample) => Math.abs((sample.values[channelIndex] ?? first) - stable));
    return approach.every((distance, index) => distance < firstDistance && (index === 0 || distance <= approach[index - 1] + threshold * 0.05));
  });
  return transient ? samples.slice(1) : samples;
}

export type EndpointLabelPosition = { id: string; naturalTop: number; top?: number };

/**
 * Keeps endpoint labels inside a plot and deterministically separates adjacent
 * values. This is display-only layout: it never changes samples or series.
 */
export function layoutEndpointLabels(
  labels: EndpointLabelPosition[],
  plotTop: number,
  plotBottom: number,
  minimumGap = 20,
  padding = 10
): EndpointLabelPosition[] {
  if (!labels.length) return [];
  const minTop = plotTop + padding;
  const maxTop = Math.max(minTop, plotBottom - padding);
  const ordered = labels
    .map((label) => ({ ...label, top: Math.min(maxTop, Math.max(minTop, label.naturalTop)) }))
    .sort((left, right) => (left.top ?? 0) - (right.top ?? 0));

  for (let index = 1; index < ordered.length; index += 1) {
    ordered[index].top = Math.max(ordered[index].top ?? minTop, (ordered[index - 1].top ?? minTop) + minimumGap);
  }

  const overflow = (ordered[ordered.length - 1].top ?? maxTop) - maxTop;
  if (overflow > 0) {
    for (const label of ordered) label.top = (label.top ?? minTop) - overflow;
  }

  if ((ordered[0].top ?? minTop) < minTop) {
    const availableGap = ordered.length > 1 ? (maxTop - minTop) / (ordered.length - 1) : 0;
    const gap = Math.min(minimumGap, availableGap);
    for (const [index, label] of ordered.entries()) label.top = minTop + index * gap;
  }

  const positions = new Map(ordered.map((label) => [label.id, label.top ?? minTop]));
  return labels.map((label) => ({ ...label, top: positions.get(label.id) ?? minTop }));
}
