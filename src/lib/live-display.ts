import { calibrationForChannel, type DisplayUnit, type RecordingCalibration } from './calibration';

export const DEFAULT_PLOT_TIME_WINDOW_SECONDS = 5;
export const MIN_PLOT_TIME_WINDOW_SECONDS = 0.5;
export const MAX_PLOT_TIME_WINDOW_SECONDS = 30;
export const PLOT_TIME_WINDOW_STEP_SECONDS = 0.5;
export const MAX_RENDERED_DISPLAY_POINTS = 2_000;
/** Display-only ADC settling interval after a hardware START. */
export const HARDWARE_STARTUP_DISPLAY_WARMUP_SECONDS = 0.1;

type DisplaySample = { sequence: number; timestamp_us: number; values: number[] };

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
 * uPlot's default time scale treats values as Unix seconds. Live acquisition
 * timestamps are board-relative microseconds, so format the x-axis explicitly
 * as elapsed recording time instead of a calendar date.
 */
export function formatElapsedSeconds(value: number): string {
  if (!Number.isFinite(value)) return '—';
  if (Math.abs(value) < 0.05) return '0 s';
  return `${value.toFixed(1)} s`;
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
 */
export function filterStartupDisplayTransient(
  samples: DisplaySample[],
  channelIndices: number[],
  adcBits: number,
  hardwareSource: boolean,
  originTimestampUs?: number
): DisplaySample[] {
  if (!hardwareSource || !samples.length || !channelIndices.length) return samples;

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
  const stableSamples = samples.slice(4, 8);
  const transient = channelIndices.some((channelIndex) => {
    const first = samples[0]?.values[channelIndex];
    if (first === undefined) return false;
    const stable = median(stableSamples.map((sample) => sample.values[channelIndex] ?? first));
    const firstDistance = Math.abs(first - stable);
    if (firstDistance <= threshold) return false;
    const approach = samples.slice(1, 4).map((sample) => Math.abs((sample.values[channelIndex] ?? first) - stable));
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
