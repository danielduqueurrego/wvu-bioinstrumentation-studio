import { describe, expect, it } from 'vitest';
import {
  derivedColumnName, estimatedPulseOxCycleRate, isValidCsvFieldName, labConfigurationIssues, MAX_LAB_CATEGORY_CHARS, MAX_LAB_IDENTIFIER_CHARS,
  MAX_LAB_NAME_CHARS, MAX_PROFILE_ID_CHARS, newChannel, nextChannelIndex,
  normalizeEditorDraft, PULSEOX_PHASE_ORDER, pulseOxCycleRateHz, pulseOxDraft, pulseOxOutput, resolvedDigitalOutputs, voltageColumnName,
  assignDraftPlotNumber, formatFirmwareBuild, isPulseOxDwell, isSequencedDwell, labConfigurationText, labModeLabel, labModeOptions, labRateLabel, labRateShortText, labRateText,
  ledSequenceText, normalizeDraftPlotGroups, parseFirmwareBuild, plotDefaultsIssues, PULSEOX_SEQUENCED_DWELL_MAX_US, PULSEOX_SEQUENCED_DWELL_MIN_US,
  pulseOxSequencedDraft, pulseOxSequencedRateHz, RESERVED_CSV_COLUMNS, SEQUENCED_MIN_FIRMWARE_BUILD,
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

  it('accepts only a whole-microsecond legacy 4-state dwell of 250–5000 µs', () => {
    expect([249, 250, 5000, 5001, 500.5].map(isPulseOxDwell)).toEqual([false, true, true, false, false]);
    const draft = pulseOxDraft(base());
    draft.acquisition.state_dwell_us = 500;
    draft.acquisition.sample_rate_hz = pulseOxCycleRateHz(500);
    expect(labConfigurationIssues(draft)).toEqual([]);
    // Rust stores the dwell as a u32; the editor refuses a fraction with its own message.
    draft.acquisition.state_dwell_us = 500.5;
    expect(labConfigurationIssues(draft)).toEqual(['Pulse-ox state dwell must be 250–5000 µs, in whole microseconds.']);
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

  it('resolves a stored legacy revision’s outputs as the board drives them', () => {
    // A 1.0.2 edit that unchecked D4 kept `led_outputs` and wrote no
    // `digital_outputs`; Rust still drives D4 HIGH while recording.
    const stored = base();
    stored.acquisition.led_outputs = { green: 'D4', red: null, ir: null };
    delete stored.acquisition.digital_outputs;
    expect(resolvedDigitalOutputs(stored.acquisition)).toEqual([{ pin: 'D4', label: 'Green LED', behavior: 'high_while_recording' }]);
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
    // The LED-sequenced CSV's state columns are reserved too (any case).
    for (const reserved of ['led_state', 'LED_State_Name']) {
      draft.acquisition.channels[1].csv_name = reserved;
      expect(labConfigurationIssues(draft).join(' ')).toMatch(/fixed column/);
    }
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

describe('LED-sequenced pulse-ox drafts', () => {
  const draft = () => pulseOxSequencedDraft(base());
  const issues = (change: (lab: LabProfile) => void) => {
    const lab = structuredClone(draft());
    change(lab);
    return labConfigurationIssues(lab).join(' ');
  };

  it('starts a new pulse-ox lab as TX and RX shown by LED state, D5 RED, D6 IR, 1000 µs', () => {
    const lab = draft();
    expect(lab.acquisition).toMatchObject({
      acquisition_mode: 'pulseox_sequenced', analog_pin: 'A0', adc_resolution_bits: 14, state_dwell_us: 1000, sample_rate_hz: 1000,
      digital_outputs: [
        { pin: 'D5', label: 'Red LED', behavior: 'acquisition_sequenced' },
        { pin: 'D6', label: 'IR LED', behavior: 'acquisition_sequenced' }
      ]
    });
    expect(lab.acquisition.analog_inputs).toBeUndefined();
    expect(lab.acquisition.led_outputs).toBeUndefined();
    expect(lab.acquisition.channels?.map((channel) => [channel.pin, channel.id, channel.label, channel.csv_name, channel.units, channel.split_by_led_state]))
      .toEqual([['A0', 'tx', 'TX', 'tx_counts', 'ADC counts', true], ['A1', 'rx', 'RX', 'rx_counts', 'ADC counts', true]]);
    expect(lab.plot_defaults?.groups).toEqual([{ channel_ids: ['tx'] }, { channel_ids: ['rx'] }]);
    // The mode needs firmware build 0x00010007; a later lab build is kept.
    expect(lab.required_firmware.build).toBe('0x00010007');
    expect(pulseOxSequencedDraft({ ...base(), required_firmware: { ...base().required_firmware, build: '0x00010008' } }).required_firmware.build).toBe('0x00010008');
    expect(labConfigurationIssues(lab)).toEqual([]);
  });

  it('ties the record rate to the dwell: round(1e6 / dwell), 1000–5000 µs', () => {
    expect(pulseOxSequencedRateHz(1000)).toBe(1000);
    expect(pulseOxSequencedRateHz(1500)).toBe(667);
    expect(pulseOxSequencedRateHz(2500)).toBe(400);
    expect(pulseOxSequencedRateHz(3000)).toBe(333);
    expect(pulseOxSequencedRateHz(5000)).toBe(200);
    expect(pulseOxSequencedRateHz(0)).toBe(0);
    expect([PULSEOX_SEQUENCED_DWELL_MIN_US, PULSEOX_SEQUENCED_DWELL_MAX_US]).toEqual([1000, 5000]);
    expect([999, 1000, 5000, 5001].map(isSequencedDwell)).toEqual([false, true, true, false]);
    // Rust stores the dwell as a u32: a fractional dwell is refused here, not by a serde type error.
    expect([1000.5, Number.NaN].map(isSequencedDwell)).toEqual([false, false]);
    expect(issues((lab) => { lab.acquisition.state_dwell_us = 1000.5; lab.acquisition.sample_rate_hz = 1000; })).toContain('dwell must be 1000–5000 µs, in whole microseconds');
    for (const dwell of [1000, 2500, 5000]) {
      expect(issues((lab) => { lab.acquisition.state_dwell_us = dwell; lab.acquisition.sample_rate_hz = pulseOxSequencedRateHz(dwell); })).toBe('');
    }
    expect(issues((lab) => { lab.acquisition.state_dwell_us = 999; lab.acquisition.sample_rate_hz = 1001; })).toContain('dwell must be 1000–5000 µs');
    expect(issues((lab) => { lab.acquisition.state_dwell_us = 5001; })).toContain('dwell must be 1000–5000 µs');
    expect(issues((lab) => { lab.acquisition.state_dwell_us = undefined; })).toContain('dwell must be 1000–5000 µs');
    // The legacy 4-state minimum (250 µs) is not enough when every state reads every channel.
    expect(issues((lab) => { lab.acquisition.state_dwell_us = 250; lab.acquisition.sample_rate_hz = 4000; })).toContain('dwell must be 1000–5000 µs');
    expect(issues((lab) => { lab.acquisition.sample_rate_hz = 250; })).toContain('1000 records/s for 1000 µs per state');
  });

  it('mirrors the Rust channel and CSV rules', () => {
    expect(issues((lab) => { lab.acquisition.channels = []; })).toContain('one through six analog channels');
    const channel = (index: number) => ({ pin: `A${index % 6}`, id: `c${index}`, label: `C${index}`, csv_name: `c${index}_counts`, units: 'ADC counts' });
    expect(issues((lab) => { lab.acquisition.channels = Array.from({ length: 6 }, (_, index) => channel(index)); lab.plot_defaults = undefined; })).toBe('');
    expect(issues((lab) => { lab.acquisition.channels = Array.from({ length: 7 }, (_, index) => channel(index)); lab.plot_defaults = undefined; }))
      .toContain('one through six analog channels');
    expect(issues((lab) => { lab.acquisition.channels![1].pin = 'A0'; })).toContain('Each analog pin must be present and unique.');
    expect(issues((lab) => { lab.acquisition.channels![1].id = 'tx'; })).toContain('Each channel ID must be present and unique.');
    expect(issues((lab) => { lab.acquisition.channels![1].label = ' '; })).toContain('Each channel label must be present.');
    expect(issues((lab) => { lab.acquisition.channels![1].csv_name = 'tx_counts'; })).toContain('Each CSV field name must be present and unique.');
    expect(issues((lab) => { lab.acquisition.channels![1].units = 'V'; })).toContain('raw ADC counts');
    // The state columns are reserved, and the derived columns must stay distinct.
    expect(RESERVED_CSV_COLUMNS).toContain('led_state');
    expect(RESERVED_CSV_COLUMNS).toContain('led_state_name');
    expect(issues((lab) => { lab.acquisition.channels![1].csv_name = 'LED_STATE_NAME'; })).toContain('CSV column "LED_STATE_NAME"');
    expect(issues((lab) => { lab.acquisition.channels![1].csv_name = 'tx_V'; })).toContain('CSV column "tx_V"');
    expect(issues((lab) => {
      lab.acquisition.channels![0].allowed_conversions = ['mpxv_pressure'];
      lab.acquisition.channels![1].csv_name = 'tx_kPa';
    })).toContain('CSV column "tx_kPa"');
    // Calibration capabilities are allowed: units are decided per channel.
    expect(issues((lab) => { lab.acquisition.channels![1].allowed_conversions = ['linear_calibration']; })).toBe('');
  });

  it('mirrors the Rust RED/IR output, firmware build, and plot-default rules', () => {
    expect(issues((lab) => { lab.acquisition.digital_outputs![1].pin = 'D5'; })).toContain('distinct sequenced Red and IR output pins');
    expect(issues((lab) => { lab.acquisition.digital_outputs = lab.acquisition.digital_outputs!.slice(0, 1); })).toContain('distinct sequenced Red and IR output pins');
    expect(issues((lab) => { lab.acquisition.digital_outputs![1].behavior = 'always_low'; })).toContain('distinct sequenced Red and IR output pins');
    expect(issues((lab) => { lab.acquisition.digital_outputs![0].pin = 'D7'; })).toContain('distinct sequenced Red and IR output pins');
    expect(issues((lab) => { lab.acquisition.digital_outputs!.push({ pin: 'D4', label: 'Green LED', behavior: 'high_while_recording' }); }))
      .toContain('only their RED and IR outputs');
    expect(issues((lab) => {
      (lab.acquisition.digital_outputs as RoleAwareOutput[]).push({ pin: 'D4', label: 'Spare', behavior: 'acquisition_sequenced', pulse_ox_role: 'red' });
      (lab.acquisition.digital_outputs as RoleAwareOutput[])[0].pulse_ox_role = 'red';
    })).toContain('Each pulse-ox role (RED, IR) may be assigned to only one output.');
    // Explicit roles may swap the LEDs onto other pins.
    expect(issues((lab) => {
      lab.acquisition.digital_outputs = [
        { pin: 'D4', label: 'Emitter A', behavior: 'acquisition_sequenced', pulse_ox_role: 'ir' },
        { pin: 'D6', label: 'Emitter B', behavior: 'acquisition_sequenced', pulse_ox_role: 'red' }
      ] as RoleAwareOutput[];
    })).toBe('');
    expect(issues((lab) => { lab.required_firmware.build = '0x00010006'; })).toContain('require WVU firmware build 0x00010007 or later');
    expect(issues((lab) => { lab.required_firmware.build = 'banana'; })).toContain('require WVU firmware build 0x00010007 or later');
    expect(issues((lab) => { lab.plot_defaults = { groups: [{ channel_ids: ['tx'] }] }; })).toContain('default plots must show every channel');
    expect(issues((lab) => { lab.plot_defaults = { groups: [{ channel_ids: ['tx', 'rx'] }, { channel_ids: ['rx'] }] }; })).toContain('default plots must show every channel');
    expect(issues((lab) => { lab.plot_defaults = { groups: [{ channel_ids: ['tx', 'rx', 'led:0:tx'] }] }; })).toContain('default plots must show every channel');
    expect(issues((lab) => { lab.plot_defaults = { groups: [{ channel_ids: ['tx', 'rx'] }, { channel_ids: [] }] }; })).toContain('default plots must show every channel');
    expect(issues((lab) => { lab.plot_defaults = { groups: [{ channel_ids: ['rx', 'tx'] }] }; })).toBe('');
    expect(issues((lab) => { lab.plot_defaults = undefined; })).toBe('');
  });

  it('allows "Show by LED state" only in an LED-sequenced lab', () => {
    const simultaneous = base();
    simultaneous.acquisition.channels![0].split_by_led_state = true;
    expect(labConfigurationIssues(simultaneous).join(' ')).toContain('Show by LED state is available only in an LED-sequenced pulse-ox lab.');
    simultaneous.acquisition.channels![0].split_by_led_state = false;
    expect(labConfigurationIssues(simultaneous)).toEqual([]);
  });

  it('normalizes an edited sequenced draft the way Rust does', () => {
    const lab = draft();
    const edited = normalizeEditorDraft({
      ...lab,
      required_firmware: { ...lab.required_firmware, build: '0x00010003' },
      acquisition: { ...lab.acquisition, state_dwell_us: 2500, sample_rate_hz: 1000, analog_inputs: { tx: 'A0', rx: 'A1' }, led_outputs: { green: null, red: 'D5', ir: 'D6' }, digital_outputs: undefined },
      plot_defaults: undefined
    });
    expect(edited.acquisition.sample_rate_hz).toBe(400);
    expect(edited.acquisition.analog_inputs).toBeUndefined();
    expect(edited.acquisition.led_outputs).toBeUndefined();
    expect(edited.acquisition.digital_outputs?.map((output) => output.pin)).toEqual(['D5', 'D6']);
    expect(edited.acquisition.channels?.map((channel) => channel.id)).toEqual(['tx', 'rx']);
    expect(edited.required_firmware.build).toBe('0x00010007');
    expect(edited.plot_defaults?.groups).toEqual([{ channel_ids: ['tx'] }, { channel_ids: ['rx'] }]);
    // A dwell out of range leaves the rate for the save-time check to report.
    expect(normalizeEditorDraft({ ...lab, acquisition: { ...lab.acquisition, state_dwell_us: 400, sample_rate_hz: 77 } }).acquisition.sample_rate_hz).toBe(77);
  });

  it('parses and formats firmware builds like Rust', () => {
    expect(parseFirmwareBuild('0x00010007')).toBe(0x00010007);
    expect(parseFirmwareBuild('00010006')).toBe(0x00010006);
    expect(parseFirmwareBuild('0xZZ')).toBeNaN();
    expect(parseFirmwareBuild('0x1FFFFFFFF')).toBeNaN();
    expect(parseFirmwareBuild(undefined)).toBeNaN();
    expect(formatFirmwareBuild(SEQUENCED_MIN_FIRMWARE_BUILD)).toBe('0x00010007');
  });
});

describe('Lab Manager modes and lab texts', () => {
  it('offers the legacy 4-state mode only for a lab that already uses it', () => {
    expect(labModeOptions('simultaneous', 'simultaneous')).toEqual([
      { value: 'simultaneous', label: 'Simultaneous Analog' },
      { value: 'pulseox_sequenced', label: 'Pulse Oximetry — LED-sequenced' }
    ]);
    expect(labModeOptions(undefined).map((option) => option.value)).toEqual(['simultaneous', 'pulseox_sequenced']);
    expect(labModeOptions('pulseox_sequenced', 'simultaneous').map((option) => option.value)).toEqual(['simultaneous', 'pulseox_sequenced']);
    expect(labModeOptions('pulseox_4state', 'simultaneous')).toEqual([
      { value: 'simultaneous', label: 'Simultaneous Analog' },
      { value: 'pulseox_sequenced', label: 'Pulse Oximetry — LED-sequenced' },
      { value: 'pulseox_4state', label: 'Pulse Oximetry — legacy 4-state' }
    ]);
    expect(labModeLabel('pulseox_4state')).toBe('Pulse Oximetry — legacy 4-state');
  });

  it('names each mode’s rate, configuration, and LED sequence', () => {
    const simultaneous = base().acquisition;
    const legacy = pulseOxDraft(base()).acquisition;
    const sequenced = pulseOxSequencedDraft(base()).acquisition;
    expect([simultaneous, legacy, sequenced].map(labRateLabel)).toEqual(['Sample rate', 'Cycle rate', 'Record rate']);
    expect([simultaneous, legacy, sequenced].map(labRateText)).toEqual(['500 frames/s', '250 cycles/s', '1000 records/s (one per LED state)']);
    // The Session setup field's text fits its column.
    expect([simultaneous, legacy, sequenced].map((acquisition) => labRateShortText(acquisition))).toEqual(['500 frames/s', '250 cycles/s', '1000 records/s']);
    expect([simultaneous, legacy, sequenced].map(labConfigurationText)).toEqual([
      '1 analog channel(s)', 'Pulse oximetry — legacy 4-state', 'Pulse oximetry — LED-sequenced, 2 channel(s)'
    ]);
    expect(ledSequenceText(simultaneous)).toBe('');
    expect(ledSequenceText(legacy)).toBe('RED → DARK → IR → DARK; 1000 µs per state');
    expect(ledSequenceText(sequenced)).toBe('RED → DARK 1 → IR → DARK 2, every channel read in each state; 1000 µs per state');
  });

  it('keeps every channel in exactly one default plot while channels change', () => {
    const channels = [newChannel('A0', []), { ...newChannel('A1', []), id: 'b', csv_name: 'b_counts' }, { ...newChannel('A2', []), id: 'c', csv_name: 'c_counts' }];
    expect(normalizeDraftPlotGroups(channels, [{ channel_ids: ['b', 'gone', 'b'] }, { channel_ids: [] }])).toEqual([
      { channel_ids: ['b'] }, { channel_ids: ['channel_1'] }, { channel_ids: ['c'] }
    ]);
    const groups = assignDraftPlotNumber(channels, [], 'c', 1);
    expect(groups).toEqual([{ channel_ids: ['channel_1', 'c'] }, { channel_ids: ['b'] }]);
    expect(assignDraftPlotNumber(channels, groups, 'channel_1', 3)).toEqual([{ channel_ids: ['c'] }, { channel_ids: ['b'] }, { channel_ids: ['channel_1'] }]);
    expect(plotDefaultsIssues(groups, channels.map((channel) => channel.id))).toEqual([]);
  });
});
