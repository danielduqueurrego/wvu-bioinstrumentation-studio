import { describe, expect, it } from 'vitest';
import {
  calibrationById,
  countsToVoltsForAdc,
  displayedValue,
  mpxvKpa,
  mpxvMmhg,
  mergeCalibrationPresets,
  supportedDisplayUnits,
  unitsForGroup,
  xgzpFitRequestPayload,
  type RecordingCalibration
} from './calibration';

const base: RecordingCalibration = {
  adc_reference_v: 5,
  mpxv_sensor_supply_v: 5,
  channel_units: {},
  active_calibrations: []
};

describe('course calibration display helpers', () => {
  it('converts ADC endpoints with the stored resolution and reference', () => {
    expect(countsToVoltsForAdc(0, 12, 5)).toBe(0);
    expect(countsToVoltsForAdc(4095, 12, 5)).toBe(5);
    expect(countsToVoltsForAdc(16383, 14, 3.3)).toBe(3.3);
  });

  it('applies the documented MPXV formula without clamping negative pressure', () => {
    expect(mpxvKpa(0.2, 5)).toBeCloseTo(0);
    expect(mpxvKpa(0.1, 5)).toBeLessThan(0);
    expect(mpxvMmhg(0.2, 5)).toBeCloseTo(0);
  });

  it('uses the profile-declared MPXV capability for the EMG pressure channel', () => {
    expect(
      supportedDisplayUnits('course_emg_force', 'pressure', false, ['counts_volts', 'mpxv_pressure'])
    ).toEqual(['counts', 'volts', 'kpa', 'mmhg']);

    const volts = 2.5;
    expect(mpxvMmhg(volts, 5)).toBeCloseTo(mpxvKpa(volts, 5) * 7.5006);
    expect(displayedValue(4095, 'pressure', 'counts', 12, base)).toBe(4095);
    expect(displayedValue(4095, 'pressure', 'kpa', 12, base)).toBeCloseTo(mpxvKpa(5, 5));
    expect(displayedValue(4095, 'pressure', 'mmhg', 12, base)).toBeCloseTo(mpxvMmhg(5, 5));
  });

  it('only exposes XGZP mmHg when an active linear calibration exists', () => {
    expect(supportedDisplayUnits('course_blood_pressure', 'xgzp', false)).toEqual(['counts', 'volts']);
    expect(supportedDisplayUnits('course_blood_pressure', 'xgzp', true)).toEqual(['counts', 'volts', 'mmhg']);
  });

  it('keeps an instructor-declared generic linear channel in its named engineering units', () => {
    expect(
      supportedDisplayUnits('development', 'load_cell', true, ['linear_calibration'], 'grams')
    ).toEqual(['counts', 'volts', 'calibrated']);
  });

  it('finds the explicitly selected persisted calibration by its stable ID', () => {
    const preset = { schema_version: 1, calibration_id: 'team.xgzp.1', profile_id: 'bp', channel_id: 'xgzp', calibration_type: 'linear' as const,
      input_quantity: 'volts', output_quantity: 'pressure', output_units: 'mmHg', parameters: { slope: 120, offset: -10 }, created_at: '2026-08-10T00:00:00Z', label: 'Team XGZP' };
    expect(calibrationById([preset], 'team.xgzp.1')).toEqual(preset);
    expect(calibrationById([preset], 'other')).toBeUndefined();
  });

  it('preserves a locally confirmed calibration while a list refresh catches up', () => {
    const saved = { schema_version: 1, calibration_id: 'team.xgzp.1', profile_id: 'bp', channel_id: 'xgzp', calibration_type: 'linear' as const,
      input_quantity: 'volts', output_quantity: 'pressure', output_units: 'mmHg', parameters: { slope: 120, offset: -10 }, created_at: '2026-08-10T00:00:00Z', label: 'Team XGZP' };
    expect(mergeCalibrationPresets([], [saved])).toEqual([saved]);
    expect(mergeCalibrationPresets([{ ...saved, label: 'Stored Team XGZP' }], [saved])[0].label).toBe('Stored Team XGZP');
  });

  it('uses a saved XGZP linear calibration only for the derived display', () => {
    const calibrated: RecordingCalibration = {
      ...base,
      active_calibrations: [{
        schema_version: 1, calibration_id: 'team.xgzp', profile_id: 'bp', channel_id: 'xgzp', calibration_type: 'linear',
        input_quantity: 'volts', output_quantity: 'pressure', output_units: 'mmHg', parameters: { slope: 100, offset: -5 }, created_at: '2026-08-10T00:00:00Z', label: 'Team'
      }]
    };
    expect(displayedValue(4095, 'xgzp', 'mmhg', 12, calibrated)).toBeCloseTo(495);
    expect(displayedValue(4095, 'xgzp', 'counts', 12, calibrated)).toBe(4095);
  });

  it('labels mixed-unit plot groups explicitly rather than pretending they share an axis unit', () => {
    expect(unitsForGroup(['mpxv', 'xgzp'], { mpxv: 'mmhg', xgzp: 'mmhg' })).toBe('Pressure (mmHg)');
    expect(unitsForGroup(['ppg', 'mpxv'], { ppg: 'volts', mpxv: 'mmhg' })).toBe('Mixed units');
  });

  it('uses the exact nested serde field names for the completed-recording fit command', () => {
    expect(xgzpFitRequestPayload('C:/recordings/bp.bmeg', 1, 8, 5, 5)).toEqual({
      bmeg_path: 'C:/recordings/bp.bmeg',
      start_seconds: 1,
      end_seconds: 8,
      adc_reference_v: 5,
      mpxv_sensor_supply_v: 5
    });
  });
});
