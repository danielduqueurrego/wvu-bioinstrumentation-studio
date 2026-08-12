import { calibrationForChannel, type DisplayUnit, type RecordingCalibration } from './calibration';

export const DEFAULT_PLOT_TIME_WINDOW_SECONDS = 5;
export const MIN_PLOT_TIME_WINDOW_SECONDS = 0.5;
export const MAX_PLOT_TIME_WINDOW_SECONDS = 30;
export const PLOT_TIME_WINDOW_STEP_SECONDS = 0.5;
export const MAX_RENDERED_DISPLAY_POINTS = 2_000;

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
