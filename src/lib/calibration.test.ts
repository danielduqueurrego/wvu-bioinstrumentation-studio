import { describe, expect, it } from 'vitest';
import {
  calibrationById,
  countsToVoltsForAdc,
  displayedValue,
  initialChannelUnits,
  isFixedMpxvChannel,
  mpxvKpa,
  mpxvMmhg,
  mergeCalibrationPresets,
  reconcileChannelUnits,
  recordingCalibrationFor,
  supportedDisplayUnits,
  unitsForGroup,
  xgzpFitRequestPayload,
  type DisplayUnit,
  type RecordingCalibration
} from './calibration';
import { formatLiveDisplayValue } from './live-display';

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

  it('matches the MPXV transfer equation in both pressure units', () => {
    const kpa = mpxvKpa(2.5, 5);
    expect(kpa).toBeCloseTo((2.5 / 5 - 0.04) / 0.009, 12);
    expect(mpxvMmhg(2.5, 5)).toBeCloseTo(kpa * 7.5006, 12);
    expect(mpxvMmhg(2.5, 5)).toBeCloseTo(383.364, 3);
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

  it('shows no value for a calibrated unit whose calibration is no longer applied', () => {
    // Deselecting a generic channel's fit must not fall through to the MPXV equation.
    expect(displayedValue(2048, 'general_a0', 'calibrated', 12, base)).toBeNaN();
    expect(formatLiveDisplayValue(displayedValue(2048, 'general_a0', 'calibrated', 12, base), 'calibrated', base, 'general_a0')).toBe('—');
  });

  it('returns a channel to volts when its calibrated unit is no longer offered', () => {
    const units: Record<string, DisplayUnit> = { general_a0: 'calibrated', xgzp: 'mmhg', mpxv: 'mmhg', ppg: 'volts' };
    const options: Record<string, DisplayUnit[]> = {
      general_a0: ['counts', 'volts'],
      xgzp: ['counts', 'volts'],
      mpxv: ['counts', 'volts', 'kpa', 'mmhg'],
      ppg: ['counts', 'volts']
    };
    expect(reconcileChannelUnits(units, options)).toEqual({ general_a0: 'volts', xgzp: 'volts', mpxv: 'mmhg', ppg: 'volts' });
    expect(reconcileChannelUnits({ only: 'kpa' }, { only: ['counts'] })).toEqual({ only: 'counts' });
  });

  it('returns the same unit map when every unit is still offered', () => {
    const units: Record<string, DisplayUnit> = { xgzp: 'mmhg', ppg: 'counts' };
    expect(reconcileChannelUnits(units, { xgzp: ['counts', 'volts', 'mmhg'], ppg: ['counts', 'volts'] })).toBe(units);
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

describe('a lab’s default display units', () => {
  it('starts each channel in its default unit when that unit is offered, otherwise in counts', () => {
    expect(initialChannelUnits([
      { id: 'ppg', default_display_unit: 'volts', allowed_conversions: ['counts_volts'] },
      { id: 'mpxv', default_display_unit: 'mmhg', allowed_conversions: ['counts_volts', 'mpxv_pressure'] },
      // mmHg on a linear-calibration channel needs a selected calibration first.
      { id: 'xgzp', default_display_unit: 'mmhg', allowed_conversions: ['counts_volts', 'linear_calibration'] },
      { id: 'a0', default_display_unit: 'furlongs' },
      { id: 'a1' }
    ], 'course_blood_pressure')).toEqual({ ppg: 'volts', mpxv: 'mmhg', xgzp: 'counts', a0: 'counts', a1: 'counts' });
  });

  it('keeps pulse-ox traces in counts or volts only', () => {
    expect(initialChannelUnits([{ id: 'tx_red', default_display_unit: 'kpa' }, { id: 'rx_red', default_display_unit: 'Volts' }], 'course_pulseox'))
      .toEqual({ tx_red: 'counts', rx_red: 'volts' });
  });
});

describe('the recording calibration snapshot', () => {
  const lab = {
    profile_id: 'wvu.bp', category: 'course_blood_pressure',
    acquisition: { channels: [{ id: 'ppg', allowed_conversions: ['counts_volts'] }, { id: 'mpxv', allowed_conversions: ['counts_volts', 'mpxv_pressure'] }, { id: 'xgzp', allowed_conversions: ['linear_calibration'] }] }
  };

  it('adds the fixed MPXV equation for each MPXV channel and the selected linear calibration', () => {
    const linear = {
      schema_version: 1, calibration_id: 'wvu.bp.xgzp.1', profile_id: 'wvu.bp', channel_id: 'xgzp', calibration_type: 'linear' as const,
      input_quantity: 'volts', output_quantity: 'pressure', output_units: 'mmHg', parameters: { slope: 2, offset: 1 },
      created_at: '2026-11-18T00:00:00Z', label: 'Team XGZP'
    };
    const snapshot = recordingCalibrationFor(lab, linear, 3.3, 5, { mpxv: 'mmhg' });
    expect(snapshot).toMatchObject({ adc_reference_v: 3.3, mpxv_sensor_supply_v: 5, channel_units: { mpxv: 'mmhg' } });
    expect(snapshot.active_calibrations.map((preset) => preset.calibration_id)).toEqual(['builtin.mpxv.mpxv', linear.calibration_id]);
    expect(snapshot.active_calibrations[0].parameters).toEqual({ sensor_supply_v: 5, adc_reference_v: 3.3 });
    expect(recordingCalibrationFor(undefined, undefined, 5, 5, {}).active_calibrations).toEqual([]);
  });

  it('recognises the EMG lab’s pressure channel as MPXV', () => {
    expect(isFixedMpxvChannel('course_emg_force', { id: 'pressure' })).toBe(true);
    expect(isFixedMpxvChannel('course_general', { id: 'pressure' })).toBe(false);
    expect(isFixedMpxvChannel(undefined, { id: 'a0', allowed_conversions: ['mpxv_pressure'] })).toBe(true);
  });
});
