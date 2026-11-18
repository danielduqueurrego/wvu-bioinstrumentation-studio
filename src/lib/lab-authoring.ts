import type { LabAcquisition, LabChannel, LabDigitalOutput, LabProfile } from './labs';

export const PULSEOX_PHASE_ORDER = ['RED ON', 'DARK 1', 'IR ON', 'DARK 2'] as const;

/** Simultaneous frame rates the controlled WVU firmware accepts. */
export const SIMULTANEOUS_RATES_HZ = [100, 200, 250, 500, 1000] as const;
/** Pulse-ox state dwell accepted by the controlled firmware (µs). */
export const PULSEOX_DWELL_MIN_US = 250;
export const PULSEOX_DWELL_MAX_US = 5000;
/** Fixed CSV columns a channel's CSV field may not reuse. */
export const RESERVED_CSV_COLUMNS = ['record_sequence', 't_us', 'cycle_index'] as const;

/** Text limits for a revision being saved now; the same values as profiles.rs. */
export const MAX_PROFILE_ID_CHARS = 128;
export const MAX_LAB_NAME_CHARS = 100;
export const MAX_LAB_CATEGORY_CHARS = 64;
export const MAX_LAB_DESCRIPTION_CHARS = 2000;
export const MAX_LAB_LABEL_CHARS = 80;
export const MAX_LAB_IDENTIFIER_CHARS = 64;
export const MAX_LAB_NOTICES = 12;
export const MAX_LAB_NOTICE_CHARS = 500;

export type PulseOxRole = 'red' | 'ir';
/** A digital output that may name its pulse-ox role explicitly (optional field). */
export type RoleAwareOutput = LabDigitalOutput & { pulse_ox_role?: PulseOxRole | null };

export function estimatedPulseOxCycleRate(stateDwellUs: number): number {
  return stateDwellUs > 0 ? 1_000_000 / (4 * stateDwellUs) : 0;
}

/** The stored cycle rate for a dwell: `round(1e6 / (4 × dwell))`, as in Rust. */
export function pulseOxCycleRateHz(stateDwellUs: number): number {
  return stateDwellUs > 0 ? Math.round(1_000_000 / (4 * stateDwellUs)) : 0;
}

/** ASCII-only lower case, so the label rule matches Rust's `eq_ignore_ascii_case`. */
function asciiLower(value: string): string {
  return value.replace(/[A-Z]/g, (character) => character.toLowerCase());
}

function labelRole(label: string): PulseOxRole | undefined {
  const lower = asciiLower(label);
  return lower === 'red led' ? 'red' : lower === 'ir led' ? 'ir' : undefined;
}

/**
 * The one shared RED/IR selection rule (mirrors `profiles::pulse_ox_output`):
 * an output that names the role explicitly wins; otherwise the first output
 * without an explicit role whose label matches.
 */
export function pulseOxOutput(outputs: LabDigitalOutput[] | undefined, role: PulseOxRole): LabDigitalOutput | undefined {
  const list = outputs ?? [];
  return list.find((output) => (output as RoleAwareOutput).pulse_ox_role === role)
    ?? list.find((output) => !(output as RoleAwareOutput).pulse_ox_role && labelRole(output.label) === role);
}

/** Mirrors `AcquisitionSettings::resolved_digital_outputs` (legacy shorthand). */
export function resolvedDigitalOutputs(acquisition: LabAcquisition): LabDigitalOutput[] {
  if (acquisition.digital_outputs?.length) return acquisition.digital_outputs;
  const legacy = acquisition.led_outputs;
  if (!legacy) return [];
  const outputs: LabDigitalOutput[] = [];
  if (legacy.green) outputs.push({ pin: legacy.green, label: 'Green LED', behavior: 'high_while_recording' });
  if (legacy.red) outputs.push({ pin: legacy.red, label: 'Red LED', behavior: 'acquisition_sequenced' });
  if (legacy.ir) outputs.push({ pin: legacy.ir, label: 'IR LED', behavior: 'acquisition_sequenced' });
  return outputs;
}

export function pulseOxDraft(profile: LabProfile): LabProfile {
  return {
    ...profile,
    acquisition: {
      ...profile.acquisition,
      acquisition_mode: 'pulseox_4state',
      analog_pin: 'A0',
      channels: [],
      analog_inputs: { tx: 'A0', rx: 'A1' },
      adc_resolution_bits: 14,
      sample_rate_hz: pulseOxCycleRateHz(1000),
      state_dwell_us: 1000,
      // The explicit list is authoritative; the legacy shorthand is not written.
      led_outputs: undefined,
      digital_outputs: [
        { pin: 'D5', label: 'Red LED', behavior: 'acquisition_sequenced' },
        { pin: 'D6', label: 'IR LED', behavior: 'acquisition_sequenced' }
      ]
    },
    plot_defaults: {
      groups: [
        { channel_ids: ['red_tx', 'dark1_tx', 'ir_tx', 'dark2_tx'] },
        { channel_ids: ['red_rx', 'dark1_rx', 'ir_rx', 'dark2_rx'] }
      ]
    }
  };
}

/** The default simultaneous channel for a pin, numbered so it is unique. */
export function newChannel(pin: string, existing: LabChannel[]): LabChannel {
  const index = nextChannelIndex(existing);
  return {
    pin,
    id: `channel_${index}`,
    label: `Channel ${index}`,
    csv_name: `channel_${index}_counts`,
    units: 'ADC counts',
    allowed_conversions: ['counts_volts'],
    default_display_unit: 'counts',
    default_visible: true
  };
}

/** Lowest N for which neither `channel_N` nor `channel_N_counts` is in use. */
export function nextChannelIndex(channels: LabChannel[]): number {
  const ids = new Set(channels.map((channel) => channel.id));
  const csvNames = new Set(channels.map((channel) => channel.csv_name));
  let index = 1;
  while (ids.has(`channel_${index}`) || csvNames.has(`channel_${index}_counts`)) index += 1;
  return index;
}

/**
 * The editor's view of a draft (the backend already normalizes drafts the
 * same way): explicit outputs only, no channels in pulse-ox mode, and at
 * least one channel plus plot defaults in simultaneous mode.
 */
export function normalizeEditorDraft(draft: LabProfile): LabProfile {
  const acquisition = draft.acquisition;
  const digital_outputs = resolvedDigitalOutputs(acquisition);
  if (acquisition.acquisition_mode === 'pulseox_4state') {
    return { ...draft, acquisition: { ...acquisition, channels: [], led_outputs: undefined, digital_outputs } };
  }
  const channels = acquisition.channels?.length ? acquisition.channels : [newChannel(acquisition.analog_pin || 'A0', [])];
  return {
    ...draft,
    acquisition: { ...acquisition, channels, led_outputs: undefined, digital_outputs },
    plot_defaults: draft.plot_defaults?.groups?.length ? draft.plot_defaults : defaultPlotGroups(channels)
  };
}

/** Mirrors `recording::voltage_column_name`. */
export function voltageColumnName(rawColumn: string): string {
  return rawColumn.endsWith('_counts') ? `${rawColumn.slice(0, -'_counts'.length)}_V` : `${rawColumn}_V`;
}

/** Mirrors `recording::derived_column_name`. */
export function derivedColumnName(rawColumn: string, units: string): string {
  const base = rawColumn.endsWith('_counts') ? rawColumn.slice(0, -'_counts'.length) : rawColumn;
  const clean = units.replace(/[^A-Za-z0-9]/g, '');
  return `${base}_${clean || 'calibrated'}`;
}

/** 1–64 ASCII letters, digits, or underscores (a leading digit is allowed). */
export function isValidCsvFieldName(value: string): boolean {
  return /^[A-Za-z0-9_]{1,64}$/.test(value);
}

/** Mirrors `AcquisitionProfile::check_csv_columns`. */
export function csvColumnIssues(channels: LabChannel[]): string[] {
  const invalid = channels.find((channel) => !isValidCsvFieldName(channel.csv_name));
  if (invalid) return [`CSV field "${invalid.csv_name}" must be 1–64 letters, digits, or underscores (no spaces, commas, or quotes).`];
  const used = new Set<string>(RESERVED_CSV_COLUMNS);
  for (const channel of channels) {
    const columns = [channel.csv_name, voltageColumnName(channel.csv_name)];
    if (channel.allowed_conversions?.includes('mpxv_pressure')) {
      columns.push(derivedColumnName(channel.csv_name, 'kPa'), derivedColumnName(channel.csv_name, 'mmHg'));
    }
    for (const column of columns) {
      const key = asciiLower(column);
      if (used.has(key)) return [`CSV column "${column}" would appear twice or reuses a fixed column; rename a CSV field so raw, _V, and pressure columns are all distinct.`];
      used.add(key);
    }
  }
  return [];
}

function lengthOf(value: string | undefined): number {
  return [...(value ?? '')].length;
}

// Control characters as Rust's `char::is_control` defines them.
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;
const CONTROL_EXCEPT_LINE_BREAKS = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/;

/** Mirrors `AcquisitionProfile::check_text_bounds`. */
function textIssues(profile: LabProfile): string[] {
  const issues: string[] = [];
  const singleLine = (value: string | undefined, limit: number, what: string) => {
    if (lengthOf(value) > limit || CONTROL.test(value ?? '')) issues.push(`${what} must be one line of ${limit} characters or fewer.`);
  };
  singleLine(profile.profile_id, MAX_PROFILE_ID_CHARS, 'The lab ID');
  singleLine(profile.display_name, MAX_LAB_NAME_CHARS, 'The lab name');
  singleLine(profile.category, MAX_LAB_CATEGORY_CHARS, 'The lab category');
  if (lengthOf(profile.description) > MAX_LAB_DESCRIPTION_CHARS || CONTROL_EXCEPT_LINE_BREAKS.test(profile.description)) {
    issues.push(`The lab description must be ${MAX_LAB_DESCRIPTION_CHARS} characters or fewer.`);
  }
  singleLine(profile.display?.channel_label, MAX_LAB_NAME_CHARS, 'The display channel label');
  singleLine(profile.export?.signal_name, MAX_LAB_IDENTIFIER_CHARS, 'The export signal name');
  if (profile.safety.notices.length > MAX_LAB_NOTICES) issues.push(`A lab may have at most ${MAX_LAB_NOTICES} safety notices.`);
  if (profile.safety.notices.some((notice) => lengthOf(notice) > MAX_LAB_NOTICE_CHARS || CONTROL_EXCEPT_LINE_BREAKS.test(notice))) {
    issues.push(`Each safety notice must be ${MAX_LAB_NOTICE_CHARS} characters or fewer.`);
  }
  for (const channel of profile.acquisition.channels ?? []) {
    singleLine(channel.id, MAX_LAB_IDENTIFIER_CHARS, 'Each channel ID');
    singleLine(channel.label, MAX_LAB_LABEL_CHARS, 'Each channel label');
  }
  for (const output of profile.acquisition.digital_outputs ?? []) singleLine(output.label, MAX_LAB_LABEL_CHARS, 'Each output label');
  return [...new Set(issues)];
}

/**
 * Save-time checks shown in the editor before the Rust validator runs. They
 * mirror `validate_for_authoring`; Rust remains authoritative.
 */
export function labConfigurationIssues(profile: LabProfile): string[] {
  const issues: string[] = [];
  const acquisition = profile.acquisition;
  if (acquisition.acquisition_mode === 'simultaneous') {
    const channels = acquisition.channels ?? [];
    if (channels.length < 1 || channels.length > 6) issues.push('A simultaneous lab needs one through six analog channels.');
    duplicateIssue(channels.map((channel) => channel.pin), 'analog pin', issues);
    duplicateIssue(channels.map((channel) => channel.id), 'channel ID', issues);
    duplicateIssue(channels.map((channel) => channel.csv_name), 'CSV field name', issues);
    issues.push(...csvColumnIssues(channels));
    if (!SIMULTANEOUS_RATES_HZ.includes(Number(acquisition.sample_rate_hz) as (typeof SIMULTANEOUS_RATES_HZ)[number])) {
      issues.push('Frame rate must be one the WVU firmware supports: 100, 200, 250, 500, or 1000 frames/s.');
    }
    if ((acquisition.digital_outputs ?? []).some((output) => (output as RoleAwareOutput).pulse_ox_role)) {
      issues.push('A pulse-ox RED/IR role can be set only in a pulse-ox lab.');
    }
  } else {
    const inputs = acquisition.analog_inputs;
    if (!inputs || inputs.tx === inputs.rx) issues.push('Pulse oximetry needs distinct TX and RX analog pins.');
    const outputs = acquisition.digital_outputs ?? [];
    for (const role of ['red', 'ir'] as const) {
      if (outputs.filter((output) => (output as RoleAwareOutput).pulse_ox_role === role).length > 1) {
        issues.push('Each pulse-ox role (RED, IR) may be assigned to only one output.');
        break;
      }
    }
    const red = pulseOxOutput(outputs, 'red');
    const ir = pulseOxOutput(outputs, 'ir');
    if (!red || !ir || red.pin === ir.pin) issues.push('Pulse oximetry needs distinct sequenced Red and IR output pins.');
    else if (outputs.some((output) => output.pin !== red.pin && output.pin !== ir.pin)) {
      issues.push('Pulse-ox labs may define only their RED and IR outputs; remove the other outputs.');
    }
    const dwell = Number(acquisition.state_dwell_us);
    if (!acquisition.state_dwell_us || dwell < PULSEOX_DWELL_MIN_US || dwell > PULSEOX_DWELL_MAX_US) {
      issues.push('Pulse-ox state dwell must be 250–5000 µs.');
    } else if (Number(acquisition.sample_rate_hz) !== pulseOxCycleRateHz(dwell)) {
      issues.push(`The pulse-ox cycle rate must follow the state dwell: ${pulseOxCycleRateHz(dwell)} cycles/s for ${dwell} µs per state.`);
    }
    if (acquisition.channels?.length) issues.push('Pulse-ox labs must not define analog channels.');
  }
  if (![12, 14].includes(Number(acquisition.adc_resolution_bits))) issues.push('ADC resolution must be 12 or 14 bit.');
  issues.push(...textIssues(profile));
  return issues;
}

function duplicateIssue(values: string[], label: string, issues: string[]) {
  const clean = values.map((value) => value.trim()).filter(Boolean);
  if (new Set(clean).size !== clean.length || clean.length !== values.length) issues.push(`Each ${label} must be present and unique.`);
}

export function defaultPlotGroups(channels: LabChannel[]) {
  return { groups: channels.map((channel) => ({ channel_ids: [channel.id] })) };
}
