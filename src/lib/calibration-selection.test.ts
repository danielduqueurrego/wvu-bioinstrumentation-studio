import { describe, expect, it } from 'vitest';
import type { CalibrationPreset } from './calibration';
import {
  applyCalibrationList,
  calibrationChannelFor,
  defaultDurationPreset,
  linearCalibrationChannels,
  type CalibrationListState
} from './calibration-selection';
import type { LabProfile } from './labs';

const preset = (profileId: string, channelId: string, id: string, label = id): CalibrationPreset => ({
  schema_version: 1, calibration_id: id, profile_id: profileId, channel_id: channelId, calibration_type: 'linear',
  input_quantity: 'volts', output_quantity: 'pressure', output_units: 'mmHg', parameters: { slope: 100, offset: -5 },
  created_at: '2026-11-18T00:00:00Z', label
});

const general = { profileId: 'general', channelId: 'general_a0' };
const bp = { profileId: 'bp', channelId: 'xgzp' };
const empty: CalibrationListState = { presets: [], selectedId: '', active: undefined };

describe('calibration list for the selected lab', () => {
  it('drops the previous lab’s response when the lab changed while it was loading', () => {
    // Request 1 was sent for General Analog; the student then picked BP and
    // request 2 was sent. Request 1 answers last and must not fill the list.
    const stale = applyCalibrationList(
      { requestId: 1, selection: general, stored: [preset('general', 'general_a0', 'g1')] },
      { latestRequestId: 2, selection: bp, state: empty }
    );
    expect(stale).toBeUndefined();
    // Even with the same request number, a different lab/channel is stale.
    expect(applyCalibrationList(
      { requestId: 2, selection: general, stored: [preset('general', 'general_a0', 'g1')] },
      { latestRequestId: 2, selection: bp, state: empty }
    )).toBeUndefined();
  });

  it('applies the newest response for the current lab and channel', () => {
    const next = applyCalibrationList(
      { requestId: 2, selection: bp, stored: [preset('bp', 'xgzp', 'b2', 'Team B'), preset('bp', 'xgzp', 'b1', 'Team A')] },
      { latestRequestId: 2, selection: bp, state: empty }
    );
    expect(next?.presets.map((item) => item.calibration_id)).toEqual(['b1', 'b2']);
    expect(next?.selectedId).toBe('');
    expect(next?.active).toBeUndefined();
  });

  it('never merges presets of another lab or channel into the list', () => {
    const next = applyCalibrationList(
      { requestId: 3, selection: bp, stored: [preset('bp', 'xgzp', 'b1'), preset('general', 'general_a0', 'foreign-stored')] },
      {
        latestRequestId: 3,
        selection: bp,
        state: { presets: [preset('general', 'general_a0', 'g1'), preset('bp', 'mpxv', 'other-channel')], selectedId: 'g1', active: preset('general', 'general_a0', 'g1') }
      }
    );
    expect(next?.presets.map((item) => item.calibration_id)).toEqual(['b1']);
    // The foreign selection is cleared, so its units cannot stay applied.
    expect(next?.selectedId).toBe('');
    expect(next?.active).toBeUndefined();
  });

  it('keeps a preset this window just saved while the file list catches up', () => {
    const saved = preset('bp', 'xgzp', 'b-new', 'New');
    const next = applyCalibrationList(
      { requestId: 4, selection: bp, stored: [preset('bp', 'xgzp', 'b1', 'Old')] },
      { latestRequestId: 4, selection: bp, state: { presets: [saved], selectedId: 'b-new', active: saved } }
    );
    expect(next?.presets.map((item) => item.calibration_id)).toEqual(['b-new', 'b1']);
    expect(next?.selectedId).toBe('b-new');
    expect(next?.active).toEqual(saved);
  });

  it('prefers the stored copy of the selected preset and clears a deleted selection', () => {
    const local = preset('bp', 'xgzp', 'b1', 'Local label');
    const refreshed = applyCalibrationList(
      { requestId: 5, selection: bp, stored: [{ ...local, label: 'Stored label' }] },
      { latestRequestId: 5, selection: bp, state: { presets: [local], selectedId: 'b1', active: local } }
    );
    expect(refreshed?.active?.label).toBe('Stored label');
    const deleted = applyCalibrationList(
      { requestId: 6, selection: bp, stored: [] },
      { latestRequestId: 6, selection: bp, state: { presets: [], selectedId: 'b1', active: undefined } }
    );
    expect(deleted).toEqual({ presets: [], selectedId: '', active: undefined });
  });
});

describe('lab change defaults', () => {
  const channels = [
    { id: 'ppg' },
    { id: 'mpxv', allowed_conversions: ['mpxv_pressure'] },
    { id: 'xgzp' },
    { id: 'load', allowed_conversions: ['counts_volts', 'linear_calibration'] }
  ];

  it('offers only channels that accept a linear calibration', () => {
    expect(linearCalibrationChannels(channels).map((channel) => channel.id)).toEqual(['xgzp', 'load']);
  });

  it('keeps the calibration channel when the new lab offers it, else uses its first one', () => {
    expect(calibrationChannelFor(channels, 'load')).toBe('load');
    expect(calibrationChannelFor(channels, 'general_a0')).toBe('xgzp');
    expect(calibrationChannelFor([{ id: 'ecg' }], 'xgzp')).toBe('');
  });

  it('uses the new lab’s presets for the default duration', () => {
    const lab = (presets: number[] | undefined) => ({ acquisition: { timed_presets_seconds: presets } }) as unknown as LabProfile;
    expect(defaultDurationPreset(lab([10, 30, 60]))).toBe('60');
    expect(defaultDurationPreset(lab([30, 120]))).toBe('30');
    expect(defaultDurationPreset(lab(undefined))).toBe('60');
    expect(defaultDurationPreset(undefined)).toBe('60');
  });
});
