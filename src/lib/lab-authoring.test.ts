import { describe, expect, it } from 'vitest';
import {
  derivedColumnName, estimatedPulseOxCycleRate, isValidCsvFieldName, labConfigurationIssues, MAX_LAB_CATEGORY_CHARS, MAX_LAB_IDENTIFIER_CHARS,
  MAX_LAB_NAME_CHARS, MAX_PROFILE_ID_CHARS, newChannel, nextChannelIndex,
  normalizeEditorDraft, PULSEOX_PHASE_ORDER, pulseOxCycleRateHz, pulseOxDraft, pulseOxOutput, resolvedDigitalOutputs, voltageColumnName,
  type RoleAwareOutput
} from './lab-authoring';
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

describe('lab editor normalization and save-time rules', () => {
  it('never gives a pulse-ox draft a phantom analog channel and drops the legacy shorthand', () => {
    const pulse = pulseOxDraft(base());
    const edited = normalizeEditorDraft({
      ...pulse,
      acquisition: { ...pulse.acquisition, channels: [{ pin: 'A0', id: 'channel_1', label: 'Channel 1', csv_name: 'channel_1_counts', units: 'ADC counts' }], led_outputs: { green: null, red: 'D5', ir: 'D6' } }
    });
    expect(edited.acquisition.channels).toEqual([]);
    expect(edited.acquisition.led_outputs).toBeUndefined();
    expect(edited.acquisition.digital_outputs?.map((output) => output.pin)).toEqual(['D5', 'D6']);
    expect(pulse.acquisition.led_outputs).toBeUndefined();
  });

  it('converts legacy led_outputs to explicit outputs so unchecking D4 really removes it', () => {
    const legacy = base();
    legacy.acquisition.led_outputs = { green: 'D4', red: null, ir: null };
    const draft = normalizeEditorDraft(legacy);
    expect(draft.acquisition.led_outputs).toBeUndefined();
    expect(draft.acquisition.digital_outputs).toEqual([{ pin: 'D4', label: 'Green LED', behavior: 'high_while_recording' }]);
    expect(resolvedDigitalOutputs({ ...draft.acquisition, digital_outputs: [] })).toEqual([]);
  });

  it('adds a channel with an unused ID and CSV name after a removal', () => {
    // Channels 1-3 existed and channel 2 was removed.
    const channels = [
      { ...newChannel('A0', []), id: 'channel_1', csv_name: 'channel_1_counts' },
      { ...newChannel('A2', []), id: 'channel_3', csv_name: 'channel_3_counts' }
    ];
    expect(nextChannelIndex(channels)).toBe(2);
    const added = newChannel('A1', channels);
    expect(added.id).toBe('channel_2');
    expect(added.csv_name).toBe('channel_2_counts');
    expect(newChannel('A3', [...channels, added]).id).toBe('channel_4');
    // A CSV name in use blocks that number even when the ID is free.
    expect(nextChannelIndex([{ ...channels[0], id: 'custom', csv_name: 'channel_1_counts' }])).toBe(2);
  });

  it('mirrors the Rust CSV column rules, including derived columns', () => {
    expect(isValidCsvFieldName('01_counts')).toBe(true);
    for (const bad of ['Vout, filtered', 'a b', 'x"y', '=SUM(A1)', '', 'a'.repeat(65)]) expect(isValidCsvFieldName(bad)).toBe(false);
    expect(voltageColumnName('ecg_counts')).toBe('ecg_V');
    expect(voltageColumnName('x')).toBe('x_V');
    expect(derivedColumnName('mpxv_counts', 'kPa')).toBe('mpxv_kPa');
    const draft = base();
    draft.acquisition.channels = [
      { pin: 'A0', id: 'first', label: 'First', csv_name: 'x', units: 'ADC counts' },
      { pin: 'A1', id: 'second', label: 'Second', csv_name: 'X_v', units: 'ADC counts' }
    ];
    expect(labConfigurationIssues(draft).join(' ')).toMatch(/would appear twice/);
    draft.acquisition.channels[1].csv_name = 't_us';
    expect(labConfigurationIssues(draft).join(' ')).toMatch(/fixed column/);
    draft.acquisition.channels = [
      { pin: 'A0', id: 'p', label: 'Pressure', csv_name: 'p_counts', units: 'ADC counts', allowed_conversions: ['counts_volts', 'mpxv_pressure'] },
      { pin: 'A1', id: 'q', label: 'Other', csv_name: 'p_mmHg', units: 'ADC counts' }
    ];
    expect(labConfigurationIssues(draft).join(' ')).toMatch(/would appear twice/);
    draft.acquisition.channels[1].csv_name = '01_counts';
    expect(labConfigurationIssues(draft)).toEqual([]);
  });

  it('accepts only firmware frame rates and ties the pulse-ox rate to the dwell', () => {
    const draft = base();
    draft.acquisition.sample_rate_hz = 300;
    expect(labConfigurationIssues(draft).join(' ')).toMatch(/frame rate/i);
    const pulse = pulseOxDraft(base());
    expect(pulse.acquisition.sample_rate_hz).toBe(250);
    expect(pulseOxCycleRateHz(900)).toBe(278);
    expect(pulseOxCycleRateHz(4000)).toBe(63);
    pulse.acquisition.state_dwell_us = 2000;
    expect(labConfigurationIssues(pulse).join(' ')).toMatch(/125 cycles\/s/);
    pulse.acquisition.sample_rate_hz = 125;
    expect(labConfigurationIssues(pulse)).toEqual([]);
  });

  it('uses one RED/IR rule: an explicit role, else the label in any ASCII case', () => {
    const outputs: RoleAwareOutput[] = [
      { pin: 'D6', label: 'Emitter A', behavior: 'acquisition_sequenced', pulse_ox_role: 'red' },
      { pin: 'D5', label: 'RED LED', behavior: 'acquisition_sequenced', pulse_ox_role: 'ir' }
    ];
    expect(pulseOxOutput(outputs, 'red')?.pin).toBe('D6');
    expect(pulseOxOutput(outputs, 'ir')?.pin).toBe('D5');
    const labelled = [
      { pin: 'D5', label: 'RED LED', behavior: 'acquisition_sequenced' as const },
      { pin: 'D6', label: 'ir led', behavior: 'acquisition_sequenced' as const }
    ];
    expect(pulseOxOutput(labelled, 'red')?.pin).toBe('D5');
    expect(pulseOxOutput(labelled, 'ir')?.pin).toBe('D6');
    expect(pulseOxOutput([{ pin: 'D4', label: 'Green LED', behavior: 'high_while_recording' }], 'red')).toBeUndefined();
    // An extra pulse-ox output is refused rather than silently ignored.
    const pulse = pulseOxDraft(base());
    pulse.acquisition.digital_outputs = [...labelled, { pin: 'D4', label: 'Green LED', behavior: 'high_while_recording' }];
    expect(labConfigurationIssues(pulse).join(' ')).toMatch(/only their RED and IR/);
    // Like Rust, one explicit role may name only one output.
    const doubled = pulseOxDraft(base());
    doubled.acquisition.digital_outputs = [
      { pin: 'D5', label: 'Emitter A', behavior: 'acquisition_sequenced', pulse_ox_role: 'red' },
      { pin: 'D6', label: 'Emitter B', behavior: 'acquisition_sequenced', pulse_ox_role: 'red' }
    ] as RoleAwareOutput[];
    expect(labConfigurationIssues(doubled).join(' ')).toMatch(/may be assigned to only one output/);
  });

  it('bounds lab text lengths', () => {
    const draft = base();
    draft.display_name = 'n'.repeat(MAX_LAB_NAME_CHARS + 1);
    draft.acquisition.channels![0].label = 'two\nlines';
    const issues = labConfigurationIssues(draft).join(' ');
    expect(issues).toMatch(/lab name/);
    expect(issues).toMatch(/channel label/);
    const multiLine = base();
    multiLine.description = 'First line.\nSecond line.';
    expect(labConfigurationIssues(multiLine)).toEqual([]);
    // The same fields Rust bounds, including the ones the editor does not show.
    const hidden = base();
    hidden.profile_id = `team.${'x'.repeat(MAX_PROFILE_ID_CHARS)}`;
    hidden.category = 'c'.repeat(MAX_LAB_CATEGORY_CHARS + 1);
    hidden.display.channel_label = 'n'.repeat(MAX_LAB_NAME_CHARS + 1);
    hidden.export.signal_name = 's'.repeat(MAX_LAB_IDENTIFIER_CHARS + 1);
    const hiddenIssues = labConfigurationIssues(hidden).join(' ');
    for (const field of [/lab ID/, /lab category/, /display channel label/, /export signal name/]) {
      expect(hiddenIssues).toMatch(field);
    }
  });
});
