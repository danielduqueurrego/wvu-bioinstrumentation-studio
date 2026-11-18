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

/**
 * A channel's conversion capability comes from its lab's
 * `allowed_conversions`. The fixed channel IDs ('mpxv', 'xgzp', and EMG
 * 'pressure') decide only for a legacy lab snapshot that lists none, so an
 * instructor channel that happens to use one of those IDs keeps exactly the
 * conversions its lab allows.
 */
function listsConversions(allowedConversions: string[] | undefined): allowedConversions is string[] {
  return Array.isArray(allowedConversions) && allowedConversions.length > 0;
}

export function supportedDisplayUnits(
  category: string | undefined,
  channelId: string,
  hasLinearCalibration: boolean,
  allowedConversions: string[] = [],
  linearOutputUnits = 'mmHg'
): DisplayUnit[] {
  if (category === 'course_pulseox') return ['counts', 'volts'];
  if (allowedConversions.includes('mpxv_pressure')) return ['counts', 'volts', 'kpa', 'mmhg'];
  if (allowedConversions.includes('linear_calibration')) {
    const calibratedUnit: DisplayUnit = linearOutputUnits.trim().toLowerCase() === 'mmhg'
      ? 'mmhg'
      : 'calibrated';
    return hasLinearCalibration ? ['counts', 'volts', calibratedUnit] : ['counts', 'volts'];
  }
  if (listsConversions(allowedConversions)) return ['counts', 'volts'];
  if (channelId === 'mpxv') return ['counts', 'volts', 'kpa', 'mmhg'];
  if (channelId === 'xgzp') return hasLinearCalibration ? ['counts', 'volts', 'mmhg'] : ['counts', 'volts'];
  return ['counts', 'volts'];
}

/** Channels that accept a saved local linear calibration. */
export function acceptsLinearCalibration(channel: { id: string; allowed_conversions?: string[] }): boolean {
  return listsConversions(channel.allowed_conversions)
    ? channel.allowed_conversions.includes('linear_calibration')
    : channel.id === 'xgzp';
}

/**
 * A short calibration ID that always satisfies the backend's preset filename
 * rule (at most 120 of `[A-Za-z0-9._-]`, no `..`), whatever the lab ID. The
 * lab and channel are stored as separate preset fields.
 */
export function newCalibrationId(channelId: string, now = Date.now(), random = Math.random()): string {
  const channel = channelId.toLowerCase().replace(/[^a-z0-9_-]/g, '_').slice(0, 40) || 'channel';
  const suffix = Math.floor(Math.min(Math.max(random, 0), 0.999999) * 36 ** 6).toString(36).padStart(6, '0');
  return `cal.${channel}.${Math.max(0, Math.trunc(now))}.${suffix}`;
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
  // 'calibrated' means "this channel's own linear fit". Without one there is
  // no value to show ("—"); the MPXV equation belongs to a different sensor.
  if (unit === 'calibrated') return Number.NaN;
  return mpxvMmhg(volts, calibration.mpxv_sensor_supply_v);
}

/**
 * Keeps every channel's display unit among the units it currently offers.
 * When a calibration is deselected, deleted, or belongs to another lab, its
 * 'mmhg' or 'calibrated' unit is no longer offered, and the channel returns
 * to volts (or the first offered unit) instead of showing a converted value
 * for a calibration that is not applied. Returns `units` itself when nothing
 * changes, so a reactive caller settles immediately. Display-only.
 */
export function reconcileChannelUnits(
  units: Record<string, DisplayUnit>,
  options: Record<string, DisplayUnit[]>
): Record<string, DisplayUnit> {
  let reconciled: Record<string, DisplayUnit> | undefined;
  for (const [channelId, offered] of Object.entries(options)) {
    const unit = units[channelId];
    if (unit === undefined || offered.includes(unit)) continue;
    reconciled ??= { ...units };
    reconciled[channelId] = offered.includes('volts') ? 'volts' : offered[0] ?? 'counts';
  }
  return reconciled ?? units;
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

/**
 * The display units a lab starts with: each channel's `default_display_unit`
 * when that unit is offered before any calibration is chosen (for example
 * mmHg on an MPXV channel, but not on a linear-calibration channel, which
 * offers mmHg only once a calibration is selected); otherwise counts.
 * Display-only: raw counts are recorded whatever the unit.
 */
export function initialChannelUnits(
  channels: Array<{ id: string; default_display_unit?: string | null; allowed_conversions?: string[] }>,
  category?: string
): Record<string, DisplayUnit> {
  return Object.fromEntries(channels.map((channel) => {
    const wanted = channel.default_display_unit?.trim().toLowerCase() ?? '';
    const offered = supportedDisplayUnits(category, channel.id, false, channel.allowed_conversions ?? []);
    const unit = offered.find((candidate) => candidate === wanted) ?? 'counts';
    return [channel.id, unit];
  }));
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

/** A channel converted with the fixed MPXV transfer equation (it needs the sensor supply Vs). */
export function isFixedMpxvChannel(category: string | undefined, channel: { id: string; allowed_conversions?: string[] }): boolean {
  if (listsConversions(channel.allowed_conversions)) return channel.allowed_conversions.includes('mpxv_pressure');
  return channel.id === 'mpxv' || (category === 'course_emg_force' && channel.id === 'pressure');
}

/**
 * The calibration snapshot the live display uses and the next Start freezes
 * into the recording: Vref, Vs, the display units, the fixed MPXV equation
 * for each MPXV channel of the lab, and the selected linear calibration.
 * A channel has at most one active preset (the backend refuses more): the
 * selected linear calibration replaces the fixed equation on its channel.
 */
export function recordingCalibrationFor(
  profile: { profile_id: string; category: string; acquisition: { channels?: Array<{ id: string; allowed_conversions?: string[] }> } } | undefined,
  linearCalibration: CalibrationPreset | undefined,
  adcReferenceV: number,
  mpxvSupplyV: number,
  channelUnits: Record<string, DisplayUnit>
): RecordingCalibration {
  const fixed = profile
    ? (profile.acquisition.channels ?? [])
      .filter((channel) => isFixedMpxvChannel(profile.category, channel))
      .filter((channel) => channel.id !== linearCalibration?.channel_id)
      .map((channel) => fixedMpxvCalibration(profile.profile_id, channel.id, mpxvSupplyV, adcReferenceV))
    : [];
  return {
    adc_reference_v: adcReferenceV,
    mpxv_sensor_supply_v: mpxvSupplyV,
    channel_units: channelUnits,
    active_calibrations: linearCalibration ? [...fixed, linearCalibration] : fixed
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
