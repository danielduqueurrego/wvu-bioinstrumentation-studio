import type { LabChannel, LabProfile } from './labs';

export const PULSEOX_PHASE_ORDER = ['RED ON', 'DARK 1', 'IR ON', 'DARK 2'] as const;

export function estimatedPulseOxCycleRate(stateDwellUs: number): number {
  return stateDwellUs > 0 ? 1_000_000 / (4 * stateDwellUs) : 0;
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
      sample_rate_hz: 250,
      state_dwell_us: 1000,
      led_outputs: { green: null, red: 'D5', ir: 'D6' },
      digital_outputs: [
        { pin: 'D5', label: 'Red LED', behavior: 'acquisition_sequenced' },
        { pin: 'D6', label: 'IR LED', behavior: 'acquisition_sequenced' }
      ]
    },
    plot_defaults: { groups: [{ channel_ids: ['red_tx', 'ir_tx'] }, { channel_ids: ['red_rx', 'ir_rx'] }] }
  };
}

export function labConfigurationIssues(profile: LabProfile): string[] {
  const issues: string[] = [];
  const acquisition = profile.acquisition;
  if (acquisition.acquisition_mode === 'simultaneous') {
    const channels = acquisition.channels ?? [];
    if (channels.length < 1 || channels.length > 6) issues.push('A simultaneous lab needs one through six analog channels.');
    duplicateIssue(channels.map((channel) => channel.pin), 'analog pin', issues);
    duplicateIssue(channels.map((channel) => channel.id), 'channel ID', issues);
    duplicateIssue(channels.map((channel) => channel.csv_name), 'CSV field name', issues);
  } else {
    const inputs = acquisition.analog_inputs;
    if (!inputs || inputs.tx === inputs.rx) issues.push('Pulse oximetry needs distinct TX and RX analog pins.');
    const red = acquisition.digital_outputs?.find((output) => output.label === 'Red LED');
    const ir = acquisition.digital_outputs?.find((output) => output.label === 'IR LED');
    if (!red || !ir || red.pin === ir.pin) issues.push('Pulse oximetry needs distinct sequenced Red and IR output pins.');
    if (!acquisition.state_dwell_us || acquisition.state_dwell_us < 250 || acquisition.state_dwell_us > 5000) issues.push('Pulse-ox state dwell must be 250–5000 µs.');
  }
  if (![12, 14].includes(Number(acquisition.adc_resolution_bits))) issues.push('ADC resolution must be 12 or 14 bit.');
  if (!Number.isFinite(Number(acquisition.sample_rate_hz)) || Number(acquisition.sample_rate_hz) < 1 || Number(acquisition.sample_rate_hz) > 1000) issues.push('Frame/cycle rate must be 1–1000 Hz.');
  return issues;
}

function duplicateIssue(values: string[], label: string, issues: string[]) {
  const clean = values.map((value) => value.trim()).filter(Boolean);
  if (new Set(clean).size !== clean.length || clean.length !== values.length) issues.push(`Each ${label} must be present and unique.`);
}

export function defaultPlotGroups(channels: LabChannel[]) {
  return { groups: channels.map((channel) => ({ channel_ids: [channel.id] })) };
}
