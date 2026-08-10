import { describe, expect, it } from 'vitest';
import { beginEditSession, beginNewLabSession, labSourceLabel, saveArguments } from './lab-catalog';
import type { LabProfile } from './labs';

const lab = {
  schema_version: 1, profile_id: 'wvu.bmeg420l.emg.force.course.capture.v1', profile_version: '1.0.0',
  display_name: 'EMG + Force — Course Capture', category: 'course_emg_force', status: 'draft', source: 'instructor',
  description: 'Draft', target_board: 'Arduino UNO R4 WiFi', fqbn: 'arduino:renesas_uno:unor4wifi',
  required_firmware: { protocol_major: 0, protocol_minor_min: 3, build: '0x00010003', device: '0x554E4F34' },
  acquisition: { analog_pin: 'A0', adc_resolution_bits: 12, sample_rate_hz: 1000, allowed_duration_modes: ['timed'], timed_presets_seconds: [10], minimum_custom_duration_seconds: 10, acquisition_mode: 'simultaneous', channels: [] },
  display: { primary_quantity: 'arduino_input_volts', channel_label: 'EMG', raw_units_label: 'ADC counts', voltage_units_label: 'V', voltage_reference_v: 5, plot_min_v: 0, plot_max_v: 5 },
  safety: { bench_only: false, human_connection_authorized: false, not_medical_device: true, notices: [] },
  export: { signal_name: 'emg', include_profile_snapshot: true }, integrity: { canonical_hash_algorithm: 'SHA-256', canonical_hash: '' }
} as LabProfile;

describe('Lab Manager detached editor sessions', () => {
  it('keeps the edit base and operation ID stable across a save retry', () => {
    const session = beginEditSession(lab, 'save-emg-0001');
    expect(saveArguments(session)).toEqual({ draft: lab, baseVersion: '1.0.0', requestId: 'save-emg-0001' });
    expect(saveArguments(session)).toEqual({ draft: lab, baseVersion: '1.0.0', requestId: 'save-emg-0001' });
  });

  it('marks duplicate and blank drafts as new IDs without a base revision', () => {
    const session = beginNewLabSession({ ...lab, profile_id: 'team2.emg' }, 'save-team-0001');
    expect(saveArguments(session).baseVersion).toBeNull();
  });

  it('makes factory and local source labels unambiguous', () => {
    expect(labSourceLabel('built_in')).toBe('Factory');
    expect(labSourceLabel('instructor')).toBe('Instructor');
    expect(labSourceLabel('imported')).toBe('Imported');
  });
});
