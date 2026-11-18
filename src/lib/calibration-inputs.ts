/**
 * Validation for the Calibration & Units inputs and the calibration dialog.
 * Nothing here changes a calibration's math: the checks only refuse input
 * that would otherwise be silently replaced, truncated, or ignored.
 */

/** The 0.1–10 V range Rust's `validate_voltage` accepts for Vref and Vs. */
export const MIN_CALIBRATION_VOLTAGE_V = 0.1;
export const MAX_CALIBRATION_VOLTAGE_V = 10;

/**
 * Why a Vref/Vs field value cannot be used, or '' when it can. Svelte binds
 * an emptied number field as `null`, which `Number()` would turn into 0 V.
 */
export function calibrationVoltageError(value: unknown, field: string): string {
  const number = typeof value === 'number' ? value : typeof value === 'string' && value.trim() ? Number(value) : Number.NaN;
  if (!Number.isFinite(number) || number < MIN_CALIBRATION_VOLTAGE_V || number > MAX_CALIBRATION_VOLTAGE_V) {
    return `Enter ${field} between ${MIN_CALIBRATION_VOLTAGE_V} and ${MAX_CALIBRATION_VOLTAGE_V} V.`;
  }
  return '';
}

/** Wire shape of the Rust `CalibrationPoint`. */
export type CalibrationPoint = { input_voltage: number; reference_value: number };

export type ParsedCalibrationPoints =
  | { points: CalibrationPoint[]; error?: undefined }
  | { points?: undefined; error: string };

const NUMBER_FIELD = /^[+-]?(?:\d+(?:[.,]\d*)?|[.,]\d+)(?:[eE][+-]?\d+)?$/;

function parseField(field: string, decimalComma: boolean): number | undefined {
  if (!NUMBER_FIELD.test(field)) return undefined;
  if (field.includes(',') && !decimalComma) return undefined;
  const value = Number(field.replace(',', '.'));
  return Number.isFinite(value) ? value : undefined;
}

/**
 * Manual calibration points: one "channel volts, reference value" pair per
 * line. Each line is split on one separator — a tab if it has one (pasted
 * from a spreadsheet), else a semicolon, else a comma — and must give exactly
 * two numbers. With a tab or semicolon separator a decimal comma is accepted
 * (`0,80;20`); with a comma separator it is not, so `0,80, 20` is refused
 * rather than read as 0 V and 80. The offending line is named.
 */
export function parseCalibrationPoints(text: string): ParsedCalibrationPoints {
  const points: CalibrationPoint[] = [];
  const lines = text.split(/\r?\n/);
  for (const [index, raw] of lines.entries()) {
    const line = raw.trim();
    if (!line) continue;
    const separator = line.includes('\t') ? '\t' : line.includes(';') ? ';' : ',';
    const fields = line.split(separator).map((field) => field.trim());
    const values = fields.map((field) => parseField(field, separator !== ','));
    if (fields.length !== 2 || values.some((value) => value === undefined)) {
      return {
        error: `Line ${index + 1} (“${line}”) must contain exactly two numbers: the channel volts, then the reference value. `
          + 'Separate them with a comma, semicolon, or tab; use a decimal point when the separator is a comma.'
      };
    }
    points.push({ input_voltage: values[0] as number, reference_value: values[1] as number });
  }
  if (points.length < 2) {
    return { error: 'Enter at least two points, one “volts, reference value” pair per line.' };
  }
  return { points };
}

export type CalibrationMethod = 'xgzp_recording' | 'manual_points';

/**
 * The quantity and units a new calibration is saved with. A fit from a BP
 * recording is always against the MPXV pressure in mmHg, whatever the manual
 * fields say. A manual fit needs both fields: nothing is filled in silently.
 */
export function calibrationOutput(
  method: CalibrationMethod,
  quantity: string,
  units: string
): { quantity: string; units: string; error?: undefined } | { error: string } {
  if (method === 'xgzp_recording') return { quantity: 'pressure', units: 'mmHg' };
  const trimmedQuantity = quantity.trim();
  const trimmedUnits = units.trim();
  if (!trimmedQuantity) return { error: 'Enter the output quantity, for example pressure or force.' };
  if (!trimmedUnits) return { error: 'Enter the output units, for example mmHg or N.' };
  return { quantity: trimmedQuantity, units: trimmedUnits };
}

/** A label that names the channel and the day, instead of a generic one. */
export function defaultCalibrationLabel(channelLabel: string, date: Date): string {
  const day = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
  return `${channelLabel.trim() || 'Channel'} calibration ${day}`.slice(0, 80);
}

/** A linear fit, plus (for a fit from a recording) the voltages behind it. */
export type CalibrationFitResult = {
  slope: number;
  offset: number;
  r_squared: number;
  paired_samples: number;
  /** The Vref and Vs the XGZP fit used. */
  adc_reference_v?: number;
  mpxv_sensor_supply_v?: number;
  /** The values frozen into the recording, when it stores them. */
  recording_adc_reference_v?: number | null;
  recording_mpxv_sensor_supply_v?: number | null;
};

function volts(value: number): string {
  return `${value.toFixed(3)} V`;
}

function differs(recorded: number | null | undefined, used: number): recorded is number {
  return recorded !== null && recorded !== undefined && Math.abs(recorded - used) > 1e-9;
}

/**
 * States which Vref and Vs a fit from a recording used and warns when they
 * differ from the values frozen into that recording. The fit itself always
 * uses the page's values; this only makes a difference visible.
 */
export function fitVoltageNotice(fit: CalibrationFitResult | undefined): { used: string; warning?: string } | undefined {
  if (!fit || fit.adc_reference_v === undefined || fit.mpxv_sensor_supply_v === undefined) return undefined;
  let used = `This fit used ADC reference ${volts(fit.adc_reference_v)} and MPXV supply ${volts(fit.mpxv_sensor_supply_v)}.`;
  const recordedNeither = fit.recording_adc_reference_v == null && fit.recording_mpxv_sensor_supply_v == null;
  if (recordedNeither) used += ' The recording does not state the values it was made with.';
  const differences: string[] = [];
  if (differs(fit.recording_adc_reference_v, fit.adc_reference_v)) {
    differences.push(`ADC reference ${volts(fit.recording_adc_reference_v)}`);
  }
  if (differs(fit.recording_mpxv_sensor_supply_v, fit.mpxv_sensor_supply_v)) {
    differences.push(`MPXV supply ${volts(fit.recording_mpxv_sensor_supply_v)}`);
  }
  if (!differences.length) return { used };
  return {
    used,
    warning: `The recording was made with ${differences.join(' and ')}. This fit uses the values above, so its MPXV reference does not match the recording’s own CSV. `
      + 'If the recording’s values were correct, enter them above and calculate the fit again.'
  };
}
