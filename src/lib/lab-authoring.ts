import type { LabAcquisition, LabAcquisitionMode, LabChannel, LabDigitalOutput, LabPlotGroup, LabProfile } from './labs';

export const PULSEOX_PHASE_ORDER = ['RED ON', 'DARK 1', 'IR ON', 'DARK 2'] as const;

/** Simultaneous frame rates the controlled WVU firmware accepts. */
export const SIMULTANEOUS_RATES_HZ = [100, 200, 250, 500, 1000] as const;
/** Pulse-ox state dwell accepted by the controlled firmware (µs). */
export const PULSEOX_DWELL_MIN_US = 250;
export const PULSEOX_DWELL_MAX_US = 5000;
/**
 * LED-state dwell of the LED-sequenced mode (µs); the same range as
 * `PULSEOX_SEQUENCED_DWELL_RANGE_US` in profiles.rs. Every state reads every
 * channel, so the shortest dwell is longer than the legacy mode's.
 */
export const PULSEOX_SEQUENCED_DWELL_MIN_US = 1000;
export const PULSEOX_SEQUENCED_DWELL_MAX_US = 5000;
/** The first WVU firmware build with the LED-sequenced mode (`SEQUENCED_MIN_FIRMWARE_BUILD`). */
export const SEQUENCED_MIN_FIRMWARE_BUILD = 0x0001_0007;
/** A lab reads at most this many analog channels (A0–A5). */
export const MAX_ANALOG_CHANNELS = 6;
/** LED-state names by state code, as the CSV's `led_state_name` column writes them. */
export const LED_STATE_NAMES = ['RED', 'DARK1', 'IR', 'DARK2'] as const;
/** The LED states as the page names them, by state code. */
export const LED_STATE_LABELS = ['RED', 'DARK 1', 'IR', 'DARK 2'] as const;
/** Fixed CSV columns a channel's CSV field may not reuse (`led_state` and
 * `led_state_name` are the LED-sequenced CSV's state columns). The same list
 * as `RESERVED_CSV_COLUMNS` in profiles.rs. */
export const RESERVED_CSV_COLUMNS = [
  'record_sequence',
  't_us',
  'cycle_index',
  'led_state',
  'led_state_name'
] as const;

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

/**
 * The stored LED-sequenced record rate for a dwell: one record per LED
 * state, `round(1e6 / dwell)` records/s (`pulse_ox_sequenced_rate_hz`).
 */
export function pulseOxSequencedRateHz(stateDwellUs: number): number {
  return stateDwellUs > 0 ? Math.round(1_000_000 / stateDwellUs) : 0;
}

/**
 * Whether a dwell is one the LED-sequenced mode accepts: a whole number of
 * microseconds in range (Rust stores `state_dwell_us` as a u32).
 */
export function isSequencedDwell(stateDwellUs: number | undefined): boolean {
  const dwell = Number(stateDwellUs);
  return Number.isInteger(dwell) && dwell >= PULSEOX_SEQUENCED_DWELL_MIN_US && dwell <= PULSEOX_SEQUENCED_DWELL_MAX_US;
}

/** Whether a dwell is one the legacy 4-state mode accepts (a whole number of µs, as Rust's u32). */
export function isPulseOxDwell(stateDwellUs: number | undefined): boolean {
  const dwell = Number(stateDwellUs);
  return Number.isInteger(dwell) && dwell >= PULSEOX_DWELL_MIN_US && dwell <= PULSEOX_DWELL_MAX_US;
}

/** A `required_firmware.build` text as a number, like Rust's `parse_hex` (NaN when invalid). */
export function parseFirmwareBuild(build: string | undefined): number {
  const digits = (build ?? '').replace(/^(0x)+/, '');
  if (!/^\+?[0-9A-Fa-f]+$/.test(digits)) return Number.NaN;
  const value = Number.parseInt(digits, 16);
  return value <= 0xFFFF_FFFF ? value : Number.NaN;
}

/** A firmware build as a lab names it, for example `0x00010007`. */
export function formatFirmwareBuild(build: number): string {
  return `0x${build.toString(16).toUpperCase().padStart(8, '0')}`;
}

/** The lab's build, raised to the first build with the LED-sequenced mode when it is older. */
function atLeastSequencedBuild(build: string): string {
  return parseFirmwareBuild(build) >= SEQUENCED_MIN_FIRMWARE_BUILD ? build : formatFirmwareBuild(SEQUENCED_MIN_FIRMWARE_BUILD);
}

const MODE_LABELS: Record<LabAcquisitionMode, string> = {
  simultaneous: 'Simultaneous Analog',
  pulseox_sequenced: 'Pulse Oximetry — LED-sequenced',
  pulseox_4state: 'Pulse Oximetry — legacy 4-state'
};

/** The name of an acquisition mode in Lab Manager. */
export function labModeLabel(mode: LabAcquisitionMode): string {
  return MODE_LABELS[mode] ?? mode;
}

export type LabModeOption = { value: LabAcquisitionMode; label: string };

/**
 * Lab Manager's acquisition modes. A new pulse-ox lab uses the LED-sequenced
 * mode; the legacy 4-state mode is offered only while editing a lab that
 * already uses it (`modesInUse`: the mode the edited lab was opened with,
 * and the draft's current mode).
 */
export function labModeOptions(...modesInUse: Array<LabAcquisitionMode | undefined>): LabModeOption[] {
  const modes: LabAcquisitionMode[] = ['simultaneous', 'pulseox_sequenced'];
  if (modesInUse.includes('pulseox_4state')) modes.push('pulseox_4state');
  return modes.map((value) => ({ value, label: labModeLabel(value) }));
}

/** The lab's rate label: a cycle, record, or frame rate. */
export function labRateLabel(acquisition: Pick<LabAcquisition, 'acquisition_mode'>): string {
  if (acquisition.acquisition_mode === 'pulseox_4state') return 'Cycle rate';
  if (acquisition.acquisition_mode === 'pulseox_sequenced') return 'Record rate';
  return 'Sample rate';
}

/** The lab's rate with its unit. */
export function labRateText(acquisition: Pick<LabAcquisition, 'acquisition_mode' | 'sample_rate_hz'>): string {
  const rate = acquisition.sample_rate_hz;
  if (acquisition.acquisition_mode === 'pulseox_4state') return `${rate} cycles/s`;
  if (acquisition.acquisition_mode === 'pulseox_sequenced') return `${rate} records/s (one per LED state)`;
  return `${rate} frames/s`;
}

/**
 * The lab's rate with its unit only, for a narrow field beside its label
 * (an LED-sequenced lab's "(one per LED state)" is left to `labRateText`).
 */
export function labRateShortText(acquisition: Pick<LabAcquisition, 'acquisition_mode' | 'sample_rate_hz'>): string {
  if (acquisition.acquisition_mode === 'pulseox_sequenced') return `${acquisition.sample_rate_hz} records/s`;
  return labRateText(acquisition);
}

/** One phrase describing a lab's acquisition (Lab Manager's version history). */
export function labConfigurationText(acquisition: Pick<LabAcquisition, 'acquisition_mode' | 'channels'>): string {
  if (acquisition.acquisition_mode === 'pulseox_4state') return 'Pulse oximetry — legacy 4-state';
  const count = acquisition.channels?.length ?? 1;
  if (acquisition.acquisition_mode === 'pulseox_sequenced') return `Pulse oximetry — LED-sequenced, ${count} channel(s)`;
  return `${count} analog channel(s)`;
}

/** The LED sequence of a pulse-ox lab, or '' for other labs. */
export function ledSequenceText(acquisition: Pick<LabAcquisition, 'acquisition_mode' | 'state_dwell_us'>): string {
  if (acquisition.acquisition_mode === 'pulseox_4state') return `RED → DARK → IR → DARK; ${acquisition.state_dwell_us} µs per state`;
  if (acquisition.acquisition_mode === 'pulseox_sequenced') {
    return `RED → DARK 1 → IR → DARK 2, every channel read in each state; ${acquisition.state_dwell_us} µs per state`;
  }
  return '';
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

/** A raw-count channel of an LED-sequenced draft, drawn by LED state by default. */
function sequencedChannel(pin: string, id: string, label: string): LabChannel {
  return {
    pin,
    id,
    label,
    csv_name: `${id}_counts`,
    units: 'ADC counts',
    allowed_conversions: ['counts_volts'],
    default_visible: true,
    split_by_led_state: true
  };
}

/**
 * The LED-sequenced pulse-ox configuration a new pulse-ox lab starts with:
 * `tx` on A0 and `rx` on A1 (both shown by LED state), D5 RED and D6 IR, a
 * 1000 µs dwell (1000 records/s), one plot per channel, and at least the
 * firmware build that has the mode. Rust normalizes a saved draft the same way.
 */
export function pulseOxSequencedDraft(profile: LabProfile): LabProfile {
  const channels = [sequencedChannel('A0', 'tx', 'TX'), sequencedChannel('A1', 'rx', 'RX')];
  return {
    ...profile,
    required_firmware: { ...profile.required_firmware, build: atLeastSequencedBuild(profile.required_firmware.build) },
    acquisition: {
      ...profile.acquisition,
      acquisition_mode: 'pulseox_sequenced',
      analog_pin: 'A0',
      channels,
      analog_inputs: undefined,
      adc_resolution_bits: 14,
      sample_rate_hz: pulseOxSequencedRateHz(1000),
      state_dwell_us: 1000,
      // The explicit list is authoritative; the legacy shorthand is not written.
      led_outputs: undefined,
      digital_outputs: [
        { pin: 'D5', label: 'Red LED', behavior: 'acquisition_sequenced' },
        { pin: 'D6', label: 'IR LED', behavior: 'acquisition_sequenced' }
      ]
    },
    plot_defaults: defaultPlotGroups(channels)
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
 * same way): explicit outputs only; no channels in the legacy 4-state mode;
 * in the LED-sequenced mode the record rate that follows from the dwell, no
 * legacy TX/RX inputs, and at least the firmware build with the mode; and at
 * least one channel plus plot defaults in the other modes.
 */
export function normalizeEditorDraft(draft: LabProfile): LabProfile {
  const acquisition = draft.acquisition;
  const digital_outputs = resolvedDigitalOutputs(acquisition);
  if (acquisition.acquisition_mode === 'pulseox_4state') {
    return { ...draft, acquisition: { ...acquisition, channels: [], led_outputs: undefined, digital_outputs } };
  }
  if (acquisition.acquisition_mode === 'pulseox_sequenced') {
    const channels = acquisition.channels?.length ? acquisition.channels : pulseOxSequencedDraft(draft).acquisition.channels ?? [];
    const sample_rate_hz = isSequencedDwell(acquisition.state_dwell_us)
      ? pulseOxSequencedRateHz(Number(acquisition.state_dwell_us))
      : acquisition.sample_rate_hz;
    return {
      ...draft,
      required_firmware: { ...draft.required_firmware, build: atLeastSequencedBuild(draft.required_firmware.build) },
      acquisition: { ...acquisition, channels, sample_rate_hz, analog_inputs: undefined, led_outputs: undefined, digital_outputs },
      plot_defaults: draft.plot_defaults?.groups?.length ? draft.plot_defaults : defaultPlotGroups(channels)
    };
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
  } else if (acquisition.acquisition_mode === 'pulseox_sequenced') {
    // Mirrors `validate_led_sequenced`: every rule applies when a lab loads.
    const channels = acquisition.channels ?? [];
    if (channels.length < 1 || channels.length > MAX_ANALOG_CHANNELS) issues.push('An LED-sequenced pulse-ox lab needs one through six analog channels.');
    duplicateIssue(channels.map((channel) => channel.pin), 'analog pin', issues);
    duplicateIssue(channels.map((channel) => channel.id), 'channel ID', issues);
    duplicateIssue(channels.map((channel) => channel.label), 'channel label', issues, false);
    duplicateIssue(channels.map((channel) => channel.csv_name), 'CSV field name', issues);
    if (channels.some((channel) => channel.units !== 'ADC counts')) issues.push('Each channel of an LED-sequenced lab records raw ADC counts.');
    issues.push(...csvColumnIssues(channels));
    const dwell = Number(acquisition.state_dwell_us);
    if (!acquisition.state_dwell_us || !isSequencedDwell(dwell)) {
      issues.push(`The LED-state dwell must be ${PULSEOX_SEQUENCED_DWELL_MIN_US}–${PULSEOX_SEQUENCED_DWELL_MAX_US} µs, in whole microseconds.`);
    } else if (Number(acquisition.sample_rate_hz) !== pulseOxSequencedRateHz(dwell)) {
      issues.push(`The record rate must follow the LED-state dwell: ${pulseOxSequencedRateHz(dwell)} records/s for ${dwell} µs per state.`);
    }
    issues.push(...pulseOxOutputIssues(acquisition.digital_outputs ?? []));
    if (!(parseFirmwareBuild(profile.required_firmware?.build) >= SEQUENCED_MIN_FIRMWARE_BUILD)) {
      issues.push(`LED-sequenced pulse-ox labs require WVU firmware build ${formatFirmwareBuild(SEQUENCED_MIN_FIRMWARE_BUILD)} or later.`);
    }
    issues.push(...plotDefaultsIssues(profile.plot_defaults?.groups ?? [], channels.map((channel) => channel.id)));
  } else {
    const inputs = acquisition.analog_inputs;
    if (!inputs || inputs.tx === inputs.rx) issues.push('Pulse oximetry needs distinct TX and RX analog pins.');
    issues.push(...pulseOxOutputIssues(acquisition.digital_outputs ?? []));
    const dwell = Number(acquisition.state_dwell_us);
    if (!acquisition.state_dwell_us || !isPulseOxDwell(dwell)) {
      issues.push(`Pulse-ox state dwell must be ${PULSEOX_DWELL_MIN_US}–${PULSEOX_DWELL_MAX_US} µs, in whole microseconds.`);
    } else if (Number(acquisition.sample_rate_hz) !== pulseOxCycleRateHz(dwell)) {
      issues.push(`The pulse-ox cycle rate must follow the state dwell: ${pulseOxCycleRateHz(dwell)} cycles/s for ${dwell} µs per state.`);
    }
    if (acquisition.channels?.length) issues.push('Pulse-ox labs must not define analog channels.');
  }
  if (acquisition.acquisition_mode !== 'pulseox_sequenced' && (acquisition.channels ?? []).some((channel) => channel.split_by_led_state)) {
    issues.push('Show by LED state is available only in an LED-sequenced pulse-ox lab.');
  }
  if (![12, 14].includes(Number(acquisition.adc_resolution_bits))) issues.push('ADC resolution must be 12 or 14 bit.');
  issues.push(...textIssues(profile));
  return issues;
}

/**
 * The RED/IR output rules of both pulse-ox modes (Rust: one output per
 * explicit role, distinct acquisition-sequenced RED and IR outputs on D4–D6,
 * and no other outputs).
 */
function pulseOxOutputIssues(outputs: LabDigitalOutput[]): string[] {
  const issues: string[] = [];
  for (const role of ['red', 'ir'] as const) {
    if (outputs.filter((output) => (output as RoleAwareOutput).pulse_ox_role === role).length > 1) {
      issues.push('Each pulse-ox role (RED, IR) may be assigned to only one output.');
      break;
    }
  }
  const red = pulseOxOutput(outputs, 'red');
  const ir = pulseOxOutput(outputs, 'ir');
  const usable = (output: LabDigitalOutput | undefined) => output?.behavior === 'acquisition_sequenced' && ['D4', 'D5', 'D6'].includes(output.pin);
  if (!red || !ir || red.pin === ir.pin || !usable(red) || !usable(ir)) issues.push('Pulse oximetry needs distinct sequenced Red and IR output pins.');
  else if (outputs.some((output) => output.pin !== red.pin && output.pin !== ir.pin)) {
    issues.push('Pulse-ox labs may define only their RED and IR outputs; remove the other outputs.');
  }
  return issues;
}

/** Mirrors `validate_plot_defaults`: no defaults, or every channel in exactly one non-empty plot. */
export function plotDefaultsIssues(groups: LabPlotGroup[], channelIds: string[]): string[] {
  if (!groups.length) return [];
  const known = new Set(channelIds);
  const assigned = new Set<string>();
  const invalid = groups.some((group) => !group.channel_ids.length
    || group.channel_ids.some((id) => !known.has(id) || assigned.has(id) || !assigned.add(id)));
  return invalid || assigned.size !== known.size
    ? ['The default plots must show every channel in exactly one plot.']
    : [];
}

function duplicateIssue(values: string[], label: string, issues: string[], unique = true) {
  const clean = values.map((value) => value.trim()).filter(Boolean);
  if ((unique && new Set(clean).size !== clean.length) || clean.length !== values.length) {
    issues.push(unique ? `Each ${label} must be present and unique.` : `Each ${label} must be present.`);
  }
}

export function defaultPlotGroups(channels: LabChannel[]) {
  return { groups: channels.map((channel) => ({ channel_ids: [channel.id] })) };
}

/**
 * The editor's default plots for a draft's channels: each channel in exactly
 * one plot, empty plots dropped, and a channel without a plot in its own.
 */
export function normalizeDraftPlotGroups(channels: LabChannel[], groups: LabPlotGroup[]): LabPlotGroup[] {
  const ids = new Set(channels.map((channel) => channel.id));
  const seen = new Set<string>();
  const result = groups
    .map((group) => ({ channel_ids: group.channel_ids.filter((id) => ids.has(id) && !seen.has(id) && Boolean(seen.add(id))) }))
    .filter((group) => group.channel_ids.length);
  for (const channel of channels) if (!seen.has(channel.id)) result.push({ channel_ids: [channel.id] });
  return result;
}

/** Moves one channel to default plot `plotNumber` (1-based), creating plots up to it. */
export function assignDraftPlotNumber(channels: LabChannel[], groups: LabPlotGroup[], channelId: string, plotNumber: number): LabPlotGroup[] {
  const requested = Math.max(1, Math.min(channels.length, Math.floor(Number(plotNumber)) || 1));
  const result = normalizeDraftPlotGroups(channels, groups).map((group) => ({ channel_ids: group.channel_ids.filter((id) => id !== channelId) }));
  while (result.length < requested) result.push({ channel_ids: [] });
  result[requested - 1].channel_ids.push(channelId);
  return result.filter((group) => group.channel_ids.length);
}
