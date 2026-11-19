import { describe, expect, it } from 'vitest';
import {
  acceptsLinearCalibration,
  calibrationById,
  countsToVoltsForAdc,
  displayedValue,
  fieldDisplayUnits,
  initialChannelUnits,
  isFixedMpxvChannel,
  mpxvKpa,
  mpxvMmhg,
  mergeCalibrationPresets,
  newCalibrationId,
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
      supportedDisplayUnits('pressure', false, ['counts_volts', 'mpxv_pressure'])
    ).toEqual(['counts', 'volts', 'kpa', 'mmhg']);

    const volts = 2.5;
    expect(mpxvMmhg(volts, 5)).toBeCloseTo(mpxvKpa(volts, 5) * 7.5006);
    expect(displayedValue(4095, 'pressure', 'counts', 12, base)).toBe(4095);
    expect(displayedValue(4095, 'pressure', 'kpa', 12, base)).toBeCloseTo(mpxvKpa(5, 5));
    expect(displayedValue(4095, 'pressure', 'mmhg', 12, base)).toBeCloseTo(mpxvMmhg(5, 5));
  });

  it('only exposes XGZP mmHg when an active linear calibration exists', () => {
    expect(supportedDisplayUnits('xgzp', false)).toEqual(['counts', 'volts']);
    expect(supportedDisplayUnits('xgzp', true)).toEqual(['counts', 'volts', 'mmhg']);
  });

  it('keeps an instructor-declared generic linear channel in its named engineering units', () => {
    expect(
      supportedDisplayUnits('load_cell', true, ['linear_calibration'], 'grams')
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
    ])).toEqual({ ppg: 'volts', mpxv: 'mmhg', xgzp: 'counts', a0: 'counts', a1: 'counts' });
  });

  it('keeps pulse-ox traces in counts or volts only', () => {
    expect(initialChannelUnits([
      { id: 'red_tx', default_display_unit: 'kpa', allowed_conversions: ['mpxv_pressure'], rawPulseField: true },
      { id: 'red_rx', default_display_unit: 'Volts', rawPulseField: true }
    ])).toEqual({ red_tx: 'counts', red_rx: 'volts' });
  });

  it('decides the units per field, not by the lab category', () => {
    // A legacy 4-state raw field: counts and volts, whatever it lists.
    expect(fieldDisplayUnits({ id: 'red_tx', allowed_conversions: ['mpxv_pressure'], rawPulseField: true })).toEqual(['counts', 'volts']);
    // An LED-sequenced lab's channels follow their own conversions, so a
    // pulse-ox-category lab may still have an MPXV or a calibrated channel.
    expect(fieldDisplayUnits({ id: 'cuff', allowed_conversions: ['counts_volts', 'mpxv_pressure'] })).toEqual(['counts', 'volts', 'kpa', 'mmhg']);
    expect(fieldDisplayUnits({ id: 'tx', allowed_conversions: ['counts_volts'] })).toEqual(['counts', 'volts']);
    expect(fieldDisplayUnits({ id: 'load', allowed_conversions: ['linear_calibration'] })).toEqual(['counts', 'volts']);
    expect(fieldDisplayUnits({ id: 'load', allowed_conversions: ['linear_calibration'] }, true, 'N')).toEqual(['counts', 'volts', 'calibrated']);
    expect(fieldDisplayUnits({ id: 'load', allowed_conversions: ['linear_calibration'] }, true, 'mmHg')).toEqual(['counts', 'volts', 'mmhg']);
    expect(initialChannelUnits([
      { id: 'tx', default_display_unit: 'volts', allowed_conversions: ['counts_volts'] },
      { id: 'cuff', default_display_unit: 'mmhg', allowed_conversions: ['counts_volts', 'mpxv_pressure'] }
    ])).toEqual({ tx: 'volts', cuff: 'mmhg' });
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

  it('follows a lab’s declared conversions even when a channel reuses a course ID', () => {
    // An instructor channel named 'mpxv' that allows only counts/volts.
    expect(isFixedMpxvChannel('development', { id: 'mpxv', allowed_conversions: ['counts_volts'] })).toBe(false);
    expect(supportedDisplayUnits('mpxv', false, ['counts_volts'])).toEqual(['counts', 'volts']);
    expect(supportedDisplayUnits('xgzp', true, ['counts_volts'])).toEqual(['counts', 'volts']);
    expect(acceptsLinearCalibration({ id: 'xgzp', allowed_conversions: ['counts_volts'] })).toBe(false);
    expect(acceptsLinearCalibration({ id: 'load', allowed_conversions: ['linear_calibration'] })).toBe(true);
    // A legacy lab snapshot without the list keeps the course-ID rules.
    expect(acceptsLinearCalibration({ id: 'xgzp' })).toBe(true);
    expect(supportedDisplayUnits('mpxv', false)).toEqual(['counts', 'volts', 'kpa', 'mmhg']);
  });

  it('gives a channel at most one active calibration', () => {
    const both = {
      profile_id: 'team.lab', category: 'development',
      acquisition: { channels: [{ id: 'mpxv', allowed_conversions: ['mpxv_pressure', 'linear_calibration'] }] }
    };
    const linear = {
      schema_version: 1, calibration_id: 'cal.mpxv.1.000000', profile_id: 'team.lab', channel_id: 'mpxv', calibration_type: 'linear' as const,
      input_quantity: 'volts', output_quantity: 'pressure', output_units: 'mmHg', parameters: { slope: 2, offset: 1 },
      created_at: '2026-11-18T00:00:00Z', label: 'Team'
    };
    expect(recordingCalibrationFor(both, linear, 5, 5, {}).active_calibrations).toEqual([linear]);
    expect(recordingCalibrationFor(both, undefined, 5, 5, {}).active_calibrations.map((preset) => preset.calibration_id))
      .toEqual(['builtin.mpxv.mpxv']);
  });
});

describe('a new calibration ID', () => {
  // Mirrors `calibration::safe_component` (the preset file name).
  const safe = (id: string) => id.length > 0 && id.length <= 120 && !id.includes('..') && /^[A-Za-z0-9._-]+$/.test(id);

  it('is short and file-name safe whatever the channel ID', () => {
    const id = newCalibrationId('xgzp', 1_763_424_000_000, 0.5);
    expect(id).toBe(`cal.xgzp.1763424000000.${Math.floor(0.5 * 36 ** 6).toString(36)}`);
    expect(safe(id)).toBe(true);
    for (const channel of ['a..b', 'x'.repeat(500), 'Ünïcode ch@nnel', '', '.', 'ends.']) {
      expect(safe(newCalibrationId(channel, Date.now(), Math.random())), channel).toBe(true);
    }
    expect(newCalibrationId('xgzp', 1, 0)).not.toBe(newCalibrationId('xgzp', 1, 0.5));
  });
});
