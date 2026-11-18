import { describe, expect, it } from 'vitest';
import {
  calibrationOutput,
  calibrationVoltageError,
  defaultCalibrationLabel,
  fitVoltageNotice,
  parseCalibrationPoints
} from './calibration-inputs';

describe('ADC reference and MPXV supply fields', () => {
  it('accepts 0.1–10 V, as Rust does', () => {
    expect(calibrationVoltageError(5, 'the ADC reference voltage')).toBe('');
    expect(calibrationVoltageError(3.3, 'the ADC reference voltage')).toBe('');
    expect(calibrationVoltageError(0.1, 'the ADC reference voltage')).toBe('');
    expect(calibrationVoltageError(10, 'the ADC reference voltage')).toBe('');
    expect(calibrationVoltageError('4.8', 'the MPXV sensor supply')).toBe('');
  });

  it('refuses an emptied field instead of reading it as 0 V', () => {
    // Svelte binds an empty number field as null; Number(null) would be 0.
    expect(calibrationVoltageError(null, 'the ADC reference voltage')).toMatch(/ADC reference voltage between 0.1 and 10 V/);
    expect(calibrationVoltageError(undefined, 'the ADC reference voltage')).not.toBe('');
    expect(calibrationVoltageError('', 'the ADC reference voltage')).not.toBe('');
    expect(calibrationVoltageError(0, 'the ADC reference voltage')).not.toBe('');
    expect(calibrationVoltageError(50, 'the MPXV sensor supply')).toMatch(/MPXV sensor supply/);
    expect(calibrationVoltageError(Number.NaN, 'the MPXV sensor supply')).not.toBe('');
  });
});

describe('manual calibration points', () => {
  it('reads one comma-separated pair per line and ignores blank lines', () => {
    expect(parseCalibrationPoints('0.80, 20\n\n1.20, 60\r\n1.60,100\n')).toEqual({
      points: [
        { input_voltage: 0.8, reference_value: 20 },
        { input_voltage: 1.2, reference_value: 60 },
        { input_voltage: 1.6, reference_value: 100 }
      ]
    });
  });

  it('refuses a line with a third field instead of silently dropping it', () => {
    const result = parseCalibrationPoints('1.2, 60, 3\n1.6, 100');
    expect(result.points).toBeUndefined();
    expect(result.error).toMatch(/Line 1/);
    expect(result.error).toMatch(/exactly two numbers/);
  });

  it('refuses decimal commas with a comma separator instead of reading 0,80 as 0 and 80', () => {
    const result = parseCalibrationPoints('0.5, 10\n0,80, 20');
    expect(result.error).toMatch(/Line 2 \(“0,80, 20”\)/);
  });

  it('accepts spreadsheet pastes separated by tabs or semicolons, with decimal commas', () => {
    expect(parseCalibrationPoints('0,80\t20\n1,20\t60').points).toEqual([
      { input_voltage: 0.8, reference_value: 20 },
      { input_voltage: 1.2, reference_value: 60 }
    ]);
    expect(parseCalibrationPoints('0,80; 20\n1.20; 60').points).toEqual([
      { input_voltage: 0.8, reference_value: 20 },
      { input_voltage: 1.2, reference_value: 60 }
    ]);
    expect(parseCalibrationPoints('-0.5\t-1e1\n.5\t+2').points).toEqual([
      { input_voltage: -0.5, reference_value: -10 },
      { input_voltage: 0.5, reference_value: 2 }
    ]);
  });

  it('refuses text, a single value, or fewer than two points', () => {
    expect(parseCalibrationPoints('one, two\n1, 2').error).toMatch(/Line 1/);
    expect(parseCalibrationPoints('1.0\n2.0, 3').error).toMatch(/Line 1/);
    expect(parseCalibrationPoints('1.0, 2.0').error).toMatch(/at least two points/);
    expect(parseCalibrationPoints('').error).toMatch(/at least two points/);
    expect(parseCalibrationPoints('1,000.5, 2\n3, 4').error).toMatch(/Line 1/);
  });
});

describe('saved calibration output', () => {
  it('saves a fit from a BP recording as pressure in mmHg whatever the manual fields say', () => {
    expect(calibrationOutput('xgzp_recording', 'force', 'grams')).toEqual({ quantity: 'pressure', units: 'mmHg' });
  });

  it('saves a manual fit with the units entered, and fills in nothing silently', () => {
    expect(calibrationOutput('manual_points', ' force ', ' N ')).toEqual({ quantity: 'force', units: 'N' });
    expect(calibrationOutput('manual_points', 'force', '  ')).toEqual({ error: expect.stringMatching(/output units/) });
    expect(calibrationOutput('manual_points', '', 'N')).toEqual({ error: expect.stringMatching(/output quantity/) });
  });

  it('labels a new calibration with the channel and the day', () => {
    expect(defaultCalibrationLabel('General A0', new Date(2026, 10, 18, 9, 30))).toBe('General A0 calibration 2026-11-18');
    expect(defaultCalibrationLabel('  ', new Date(2026, 0, 2))).toBe('Channel calibration 2026-01-02');
    expect(defaultCalibrationLabel('x'.repeat(100), new Date(2026, 0, 2))).toHaveLength(80);
  });
});

describe('Vref and Vs behind a fit from a recording', () => {
  const fit = { slope: 120, offset: -10, r_squared: 0.999, paired_samples: 1201 };

  it('says nothing for a manual fit', () => {
    expect(fitVoltageNotice(fit)).toBeUndefined();
    expect(fitVoltageNotice(undefined)).toBeUndefined();
  });

  it('states the values used and stays quiet when they match the recording', () => {
    const notice = fitVoltageNotice({
      ...fit, adc_reference_v: 5, mpxv_sensor_supply_v: 5,
      recording_adc_reference_v: 5, recording_mpxv_sensor_supply_v: 5
    });
    expect(notice?.used).toBe('This fit used ADC reference 5.000 V and MPXV supply 5.000 V.');
    expect(notice?.warning).toBeUndefined();
  });

  it('warns when the page values differ from the values frozen in the recording', () => {
    const notice = fitVoltageNotice({
      ...fit, adc_reference_v: 5, mpxv_sensor_supply_v: 4.8,
      recording_adc_reference_v: 5, recording_mpxv_sensor_supply_v: 5
    });
    expect(notice?.used).toMatch(/MPXV supply 4\.800 V/);
    expect(notice?.warning).toMatch(/recording was made with MPXV supply 5\.000 V/);
    expect(notice?.warning).not.toMatch(/ADC reference/);
  });

  it('says so when an older recording does not store its values', () => {
    const notice = fitVoltageNotice({
      ...fit, adc_reference_v: 5, mpxv_sensor_supply_v: 5,
      recording_adc_reference_v: null, recording_mpxv_sensor_supply_v: null
    });
    expect(notice?.used).toMatch(/does not state the values/);
    expect(notice?.warning).toBeUndefined();
  });
});
