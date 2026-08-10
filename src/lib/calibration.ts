/** Display/export calibration helpers. Raw BMEG values remain ADC counts. */
export type DisplayUnit = 'counts' | 'volts' | 'kpa' | 'mmhg' | 'calibrated';

export type CalibrationPreset = {
  schema_version: number;
  calibration_id: string;
  profile_id: string;
  channel_id: string;
  calibration_type: 'fixed_formula' | 'linear';
  input_quantity: string;
  output_quantity: string;
  output_units: string;
  parameters: Record<string, number>;
  created_at: string;
  label: string;
};

export type RecordingCalibration = {
  adc_reference_v: number;
  mpxv_sensor_supply_v: number;
  channel_units: Record<string, DisplayUnit>;
  active_calibrations: CalibrationPreset[];
};

/** Wire shape for the Rust `XgzpFitRequest`. Tauri does not rename nested
 * command payload fields, so this deliberately uses the Rust/serde names. */
export type XgzpFitRequestPayload = {
  bmeg_path: string;
  start_seconds: number;
  end_seconds: number;
  adc_reference_v: number;
  mpxv_sensor_supply_v: number;
};

export const DEFAULT_ADC_REFERENCE_V = 5;
export const DEFAULT_MPXV_SUPPLY_V = 5;
export const MMHG_PER_KPA = 7.5006;

export function countsToVoltsForAdc(counts: number, adcBits: number, referenceV = DEFAULT_ADC_REFERENCE_V): number {
  return counts * referenceV / (Math.pow(2, adcBits) - 1);
}

export function mpxvKpa(volts: number, sensorSupplyV = DEFAULT_MPXV_SUPPLY_V): number {
  return (volts / sensorSupplyV - 0.04) / 0.009;
}

export function mpxvMmhg(volts: number, sensorSupplyV = DEFAULT_MPXV_SUPPLY_V): number {
  return mpxvKpa(volts, sensorSupplyV) * MMHG_PER_KPA;
}

export function calibrationForChannel(calibrations: CalibrationPreset[], channelId: string): CalibrationPreset | undefined {
  return calibrations.find((calibration) => calibration.channel_id === channelId);
}

export function calibrationById(calibrations: CalibrationPreset[], calibrationId: string): CalibrationPreset | undefined {
  return calibrations.find((calibration) => calibration.calibration_id === calibrationId);
}

export function mergeCalibrationPresets(
  stored: CalibrationPreset[],
  locallyConfirmed: CalibrationPreset[]
): CalibrationPreset[] {
  const merged = new Map<string, CalibrationPreset>();
  for (const calibration of locallyConfirmed) merged.set(calibration.calibration_id, calibration);
  for (const calibration of stored) merged.set(calibration.calibration_id, calibration);
  return [...merged.values()].sort((left, right) =>
    left.label.localeCompare(right.label) || left.calibration_id.localeCompare(right.calibration_id)
  );
}

export function supportedDisplayUnits(
  category: string | undefined,
  channelId: string,
  hasLinearCalibration: boolean,
  allowedConversions: string[] = [],
  linearOutputUnits = 'mmHg'
): DisplayUnit[] {
  if (category === 'course_pulseox') return ['counts', 'volts'];
  if (channelId === 'pressure' && category === 'course_emg_force') return ['counts', 'volts', 'kpa'];
  if (allowedConversions.includes('mpxv_pressure')) return ['counts', 'volts', 'kpa', 'mmhg'];
  if (allowedConversions.includes('linear_calibration')) {
    const calibratedUnit: DisplayUnit = linearOutputUnits.trim().toLowerCase() === 'mmhg'
      ? 'mmhg'
      : 'calibrated';
    return hasLinearCalibration ? ['counts', 'volts', calibratedUnit] : ['counts', 'volts'];
  }
  if (channelId === 'mpxv') return ['counts', 'volts', 'kpa', 'mmhg'];
  if (channelId === 'xgzp') return hasLinearCalibration ? ['counts', 'volts', 'mmhg'] : ['counts', 'volts'];
  return ['counts', 'volts'];
}

export function displayUnitLabel(
  unit: DisplayUnit,
  calibration?: RecordingCalibration,
  channelId?: string
): string {
  if (unit === 'calibrated') {
    return calibrationForChannel(calibration?.active_calibrations ?? [], channelId ?? '')?.output_units
      || 'Calibrated units';
  }
  return ({ counts: 'ADC counts', volts: 'V', kpa: 'Pressure (kPa)', mmhg: 'Pressure (mmHg)' })[unit];
}

export function displayedValue(
  counts: number,
  channelId: string,
  unit: DisplayUnit,
  adcBits: number,
  calibration: RecordingCalibration
): number {
  if (unit === 'counts') return counts;
  const volts = countsToVoltsForAdc(counts, adcBits, calibration.adc_reference_v);
  if (unit === 'volts') return volts;
  if (unit === 'kpa') return mpxvKpa(volts, calibration.mpxv_sensor_supply_v);
  const preset = calibrationForChannel(calibration.active_calibrations, channelId);
  if (preset?.calibration_type === 'linear') {
    return (preset.parameters.slope ?? Number.NaN) * volts + (preset.parameters.offset ?? Number.NaN);
  }
  return mpxvMmhg(volts, calibration.mpxv_sensor_supply_v);
}

export function unitsForGroup(
  channelIds: string[],
  units: Record<string, DisplayUnit>,
  calibration?: RecordingCalibration
): string {
  const labels = new Set(channelIds.map((channelId) =>
    displayUnitLabel(units[channelId] ?? 'counts', calibration, channelId)
  ));
  return labels.size === 1 ? [...labels][0] : 'Mixed units';
}

export function initialChannelUnits(channelIds: string[]): Record<string, DisplayUnit> {
  return Object.fromEntries(channelIds.map((channelId) => [channelId, 'counts']));
}

export function fixedMpxvCalibration(profileId: string, channelId: string, sensorSupplyV: number, adcReferenceV: number): CalibrationPreset {
  return {
    schema_version: 1,
    calibration_id: `builtin.mpxv.${channelId}`,
    profile_id: profileId,
    channel_id: channelId,
    calibration_type: 'fixed_formula',
    input_quantity: 'volts',
    output_quantity: 'pressure',
    output_units: 'kPa/mmHg',
    parameters: { sensor_supply_v: sensorSupplyV, adc_reference_v: adcReferenceV },
    // This controlled fixed formula is parameterized by Vs/Vref; it is not a
    // user-created preset. A stable timestamp avoids making plot configuration
    // appear to change on every display refresh.
    created_at: '1970-01-01T00:00:00.000Z',
    label: 'MPXV transfer equation'
  };
}

export function xgzpFitRequestPayload(
  bmegPath: string,
  startSeconds: number,
  endSeconds: number,
  adcReferenceV: number,
  mpxvSensorSupplyV: number
): XgzpFitRequestPayload {
  return {
    bmeg_path: bmegPath,
    start_seconds: startSeconds,
    end_seconds: endSeconds,
    adc_reference_v: adcReferenceV,
    mpxv_sensor_supply_v: mpxvSensorSupplyV
  };
}
