import { describe, expect, it } from 'vitest';
import { estimatedPulseOxCycleRate, labConfigurationIssues, PULSEOX_PHASE_ORDER, pulseOxDraft } from './lab-authoring';
import type { LabProfile } from './labs';

const base = (): LabProfile => ({
  schema_version: 1, profile_id: 'team.lab', profile_version: '1.0.1', display_name: 'Team Lab', category: 'development', status: 'draft', source: 'instructor', description: 'Teaching lab', target_board: 'Arduino UNO R4 WiFi', fqbn: 'arduino:renesas_uno:unor4wifi',
  required_firmware: { protocol_major: 0, protocol_minor_min: 3, build: '0x00010003', device: '0x554E4F34' },
  acquisition: { analog_pin: 'A0', adc_resolution_bits: 12, sample_rate_hz: 500, allowed_duration_modes: ['timed', 'until_stopped'], timed_presets_seconds: [10], minimum_custom_duration_seconds: 10, acquisition_mode: 'simultaneous', channels: [{ pin: 'A0', id: 'signal', label: 'Signal', csv_name: 'signal_counts', units: 'ADC counts' }] },
  display: { primary_quantity: 'arduino_input_volts', channel_label: 'Signal', raw_units_label: 'ADC counts', voltage_units_label: 'V', voltage_reference_v: 5, plot_min_v: 0, plot_max_v: 5 },
  safety: { bench_only: false, human_connection_authorized: false, not_medical_device: true, notices: ['Teaching use only'] },
  export: { signal_name: 'signal', include_profile_snapshot: true }, integrity: { canonical_hash_algorithm: 'SHA-256', canonical_hash: '' }
});

describe('instructor lab authoring helpers', () => {
  it('creates the fixed pulse-ox phase configuration without exposing an arbitrary sequencer', () => {
    const draft = pulseOxDraft(base());
    expect(draft.acquisition.acquisition_mode).toBe('pulseox_4state');
    expect(draft.acquisition.analog_inputs).toEqual({ tx: 'A0', rx: 'A1' });
    expect(draft.acquisition.digital_outputs?.map((output) => output.pin)).toEqual(['D5', 'D6']);
    expect(PULSEOX_PHASE_ORDER).toEqual(['RED ON', 'DARK 1', 'IR ON', 'DARK 2']);
    expect(estimatedPulseOxCycleRate(1000)).toBe(250);
  });

  it('flags duplicate resources before the Rust save-time validator runs', () => {
    const draft = base();
    draft.acquisition.channels?.push({ pin: 'A0', id: 'signal', label: 'Duplicate', csv_name: 'signal_counts', units: 'ADC counts' });
    expect(labConfigurationIssues(draft).join(' ')).toMatch(/unique/);
  });

  it('flags an unsafe pulse configuration', () => {
    const draft = pulseOxDraft(base());
    draft.acquisition.analog_inputs = { tx: 'A2', rx: 'A2' };
    draft.acquisition.digital_outputs![1].pin = 'D5';
    expect(labConfigurationIssues(draft)).toHaveLength(2);
  });
});
