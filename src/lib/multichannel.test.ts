import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { initialChannelUnits, supportedDisplayUnits } from './calibration';
import type { LabProfile } from './labs';
import {
  assignChannelToPlot,
  channelColor,
  defaultPlotGroups,
  initialPlotGroups,
  initialTraceVisibility,
  isLedSequencedProfile,
  isPulseOxProfile,
  labRevisionKey,
  ledSplitChoices,
  ledTraceId,
  normalizePlotGroups,
  numberedVisiblePlotGroups,
  onePlotPerSignal,
  overlayAll,
  plotChannelsFor,
  plotGroupHeading,
  plotLayoutKey,
  profileChannels,
  PULSEOX_PLOT_CHANNELS,
  remapTraceLayout,
  seriesColor,
  setPlotGroupCount,
  setTraceVisibility,
  shouldShowPlotLegend,
  unitChannelsFor,
  visiblePlotSeries,
  visiblePlotGroups,
  visibleChannelIds,
  visibleChannels
} from './multichannel';

describe('multi-channel UI helpers', () => {
  const channels = [
    { id: 'raw_emg', label: 'Raw EMG', csv_name: 'raw_emg_counts' },
    { id: 'rectified_emg', label: 'Rectified EMG', csv_name: 'rectified_emg_counts' },
    { id: 'envelope', label: 'Envelope', csv_name: 'emg_envelope_counts' },
    { id: 'pressure', label: 'Pressure / Force Surrogate', csv_name: 'pressure_counts' }
  ];

  it('keeps each selected synchronized trace visible independently', () => {
    expect(visibleChannels(channels, ['raw_emg', 'envelope']).map((channel) => channel.id))
      .toEqual(['raw_emg', 'envelope']);
  });

  it('uses one visibility map so hiding a trace does not reset the other live traces', () => {
    let visibility = initialTraceVisibility(channels);
    visibility = setTraceVisibility(visibility, 'raw_emg', false);
    expect(visibility.raw_emg).toBe(false);
    expect(visibleChannelIds(channels, visibility)).toEqual([
      'rectified_emg', 'envelope', 'pressure'
    ]);
    visibility = setTraceVisibility(visibility, 'raw_emg', true);
    expect(visibleChannelIds(channels, visibility)).toEqual(channels.map((channel) => channel.id));
  });

  it('uses course-specific display-only plot-group defaults', () => {
    expect(defaultPlotGroups('course_ecg', channels.slice(0, 1))).toHaveLength(1);
    expect(defaultPlotGroups('course_emg_force', channels)).toHaveLength(4);
    expect(defaultPlotGroups('development', channels)).toHaveLength(4);
    expect(defaultPlotGroups('course_blood_pressure', channels.slice(0, 3))).toHaveLength(3);
    expect(defaultPlotGroups('course_pulseox', [
      { id: 'red_tx', label: 'TX Red', csv_name: 'red_TX' },
      { id: 'dark1_tx', label: 'TX Dark 1', csv_name: 'dark1_TX' },
      { id: 'ir_tx', label: 'TX IR', csv_name: 'ir_TX' },
      { id: 'dark2_tx', label: 'TX Dark 2', csv_name: 'dark2_TX' },
      { id: 'red_rx', label: 'RX Red', csv_name: 'red_RX' },
      { id: 'dark1_rx', label: 'RX Dark 1', csv_name: 'dark1_RX' },
      { id: 'ir_rx', label: 'RX IR', csv_name: 'ir_RX' },
      { id: 'dark2_rx', label: 'RX Dark 2', csv_name: 'dark2_RX' }
    ]).map((group) => group.channelIds)).toEqual([
      ['red_tx', 'dark1_tx', 'ir_tx', 'dark2_tx'],
      ['red_rx', 'dark1_rx', 'ir_rx', 'dark2_rx']
    ]);
  });

  it('merges removed plots deterministically and preserves assignments while hidden', () => {
    let groups = onePlotPerSignal(channels);
    groups = setPlotGroupCount(channels, groups, 2);
    expect(groups.map((group) => group.channelIds)).toEqual([
      ['raw_emg'],
      ['rectified_emg', 'envelope', 'pressure']
    ]);
    groups = assignChannelToPlot(channels, groups, 'rectified_emg', 0);
    expect(groups.map((group) => group.channelIds)).toEqual([
      ['raw_emg', 'rectified_emg'],
      ['envelope', 'pressure']
    ]);
    const visibility = setTraceVisibility(initialTraceVisibility(channels), 'pressure', false);
    expect(visiblePlotGroups(channels, groups, visibility).map((group) => group.channelIds))
      .toEqual([['raw_emg', 'rectified_emg'], ['envelope']]);
  });

  it('keeps empty slots available for assignment but never asks the UI to render an empty plot', () => {
    const groups = setPlotGroupCount(channels, overlayAll(channels), 4);
    expect(groups).toHaveLength(4);
    expect(visiblePlotGroups(channels, groups, initialTraceVisibility(channels))).toEqual([
      { id: 'plot-1', channelIds: channels.map((channel) => channel.id) }
    ]);
  });

  it('shows a per-plot legend only when that plot has at least two visible series', () => {
    const one = visiblePlotSeries(channels, ['raw_emg']);
    const two = visiblePlotSeries(channels, ['raw_emg', 'rectified_emg']);
    const four = visiblePlotSeries(channels, channels.map((channel) => channel.id));

    expect(shouldShowPlotLegend(one)).toBe(false);
    expect(shouldShowPlotLegend(two)).toBe(true);
    expect(shouldShowPlotLegend(four)).toBe(true);
    expect(two.map((series) => series.label)).toEqual(['Raw EMG', 'Rectified EMG']);
    expect(four).toHaveLength(4);
  });

  it('keeps each signal in one colour when other traces are hidden or reassigned', () => {
    const overlay = visiblePlotSeries(channels, ['raw_emg', 'rectified_emg']);
    expect(overlay.map((series) => series.color)).toEqual([seriesColor(0), seriesColor(1)]);

    // Hiding Raw EMG does not recolour Rectified EMG.
    const oneRemaining = visiblePlotSeries(channels, ['rectified_emg']);
    expect(shouldShowPlotLegend(oneRemaining)).toBe(false);
    expect(oneRemaining[0]?.color).toBe(overlay[1]?.color);

    // The same signal has the same colour in whichever plot it is drawn, and
    // different signals in different plots do not share a colour.
    const firstPlot = visiblePlotSeries(channels, ['raw_emg', 'rectified_emg']);
    const secondPlot = visiblePlotSeries(channels, ['envelope', 'pressure']);
    expect(firstPlot.map((series) => series.label)).toEqual(['Raw EMG', 'Rectified EMG']);
    expect(secondPlot.map((series) => series.label)).toEqual(['Envelope', 'Pressure / Force Surrogate']);
    expect(secondPlot[0]?.color).toBe(channelColor(channels, 'envelope'));
    expect(secondPlot[0]?.color).not.toBe(firstPlot[0]?.color);
    const reassigned = visiblePlotSeries(channels, ['raw_emg', 'envelope']);
    expect(reassigned.map((series) => series.color)).toEqual([
      channelColor(channels, 'raw_emg'), channelColor(channels, 'envelope')
    ]);
    expect(new Set(channels.map((channel) => channelColor(channels, channel.id))).size).toBe(channels.length);
  });

  it('draws pulse-ox RED in red and IR in a distinct dark colour wherever they appear', () => {
    const pulse = [...PULSEOX_PLOT_CHANNELS];
    const red = channelColor(pulse, 'red_tx');
    const ir = channelColor(pulse, 'ir_tx');
    expect(red).toBe('#C8102E');
    expect(ir).not.toBe(red);
    expect(ir).not.toBe(channelColor(pulse, 'dark1_tx'));
    expect(ir).not.toBe(channelColor(pulse, 'dark2_tx'));
    // The navy/gold position palette is not used for the LED phases.
    expect([red, ir]).not.toContain(seriesColor(0));
    // Hiding RED changes no other trace's colour.
    const all = visiblePlotSeries(pulse, ['red_tx', 'dark1_tx', 'ir_tx', 'dark2_tx']);
    const withoutRed = visiblePlotSeries(pulse, ['dark1_tx', 'ir_tx', 'dark2_tx']);
    expect(withoutRed.map((series) => series.color)).toEqual(all.slice(1).map((series) => series.color));
    // TX and RX stay distinguishable in an overlay.
    expect(channelColor(pulse, 'red_rx')).not.toBe(red);
    expect(new Set(pulse.map((channel) => channelColor(pulse, channel.id))).size).toBe(8);
  });

  it('numbers rendered plots by their assignment slot, not by render position', () => {
    const groups = onePlotPerSignal(channels);
    const visibility = setTraceVisibility(initialTraceVisibility(channels), 'raw_emg', false);
    const numbered = numberedVisiblePlotGroups(channels, groups, visibility);
    expect(numbered.map((group) => group.slot)).toEqual([2, 3, 4]);
    expect(numbered[0]?.channelIds).toEqual(['rectified_emg']);
    // An empty slot is skipped without renumbering the plots after it.
    const withEmpty = setPlotGroupCount(channels, overlayAll(channels), 3);
    const moved = assignChannelToPlot(channels, withEmpty, 'pressure', 2);
    expect(numberedVisiblePlotGroups(channels, moved, initialTraceVisibility(channels)).map((group) => group.slot))
      .toEqual([1, 3]);
    expect(visiblePlotGroups(channels, moved, initialTraceVisibility(channels)).map((group) => group.id))
      .toEqual(['plot-1', 'plot-3']);
  });

  it('keeps plot IDs stable while an active recording is rearranged', () => {
    let groups = onePlotPerSignal(channels);
    expect(groups.map((group) => group.id)).toEqual(['plot-1', 'plot-2', 'plot-3', 'plot-4']);

    groups = setPlotGroupCount(channels, groups, 2);
    expect(groups.map((group) => group.id)).toEqual(['plot-1', 'plot-2']);
    groups = setPlotGroupCount(channels, groups, 3);
    expect(groups.map((group) => group.id)).toEqual(['plot-1', 'plot-2', 'plot-3']);
    groups = setPlotGroupCount(channels, groups, 4);
    expect(groups.map((group) => group.id)).toEqual(['plot-1', 'plot-2', 'plot-3', 'plot-4']);
    expect(new Set(groups.map((group) => group.id)).size).toBe(groups.length);
  });

  it('keeps layout changes display-only and retains every captured channel assignment', () => {
    let groups = onePlotPerSignal(channels);
    groups = setPlotGroupCount(channels, groups, 2);
    groups = assignChannelToPlot(channels, groups, 'rectified_emg', 0);
    groups = overlayAll(channels);
    groups = onePlotPerSignal(channels);

    expect(groups.flatMap((group) => group.channelIds)).toEqual(channels.map((channel) => channel.id));
    expect(visiblePlotGroups(channels, groups, initialTraceVisibility(channels))).toHaveLength(4);
  });

  it('applies overlay and one-per-signal convenience presets without changing capture fields', () => {
    expect(overlayAll(channels)).toEqual([{ id: 'plot-1', channelIds: channels.map((channel) => channel.id) }]);
    expect(onePlotPerSignal(channels).map((group) => group.channelIds)).toEqual([
      ['raw_emg'], ['rectified_emg'], ['envelope'], ['pressure']
    ]);
  });

  it('keeps all eight raw pulse-ox phase fields in the plot arrangement', () => {
    const rawPulseChannels = [
      ['red_tx', 'TX Red'], ['dark1_tx', 'TX Dark 1'], ['ir_tx', 'TX IR'], ['dark2_tx', 'TX Dark 2'],
      ['red_rx', 'RX Red'], ['dark1_rx', 'RX Dark 1'], ['ir_rx', 'RX IR'], ['dark2_rx', 'RX Dark 2']
    ].map(([id, label]) => ({ id, label, csv_name: id }));
    expect(defaultPlotGroups('course_pulseox', rawPulseChannels).flatMap((group) => group.channelIds))
      .toEqual(rawPulseChannels.map((channel) => channel.id));
    const pulseGroups = defaultPlotGroups('course_pulseox', rawPulseChannels);
    expect(visiblePlotSeries(rawPulseChannels, pulseGroups[0]?.channelIds ?? []).map((series) => series.label))
      .toEqual(['TX Red', 'TX Dark 1', 'TX IR', 'TX Dark 2']);
    expect(visiblePlotSeries(rawPulseChannels, pulseGroups[1]?.channelIds ?? [])).toHaveLength(4);
    expect(shouldShowPlotLegend(visiblePlotSeries(rawPulseChannels, pulseGroups[0]?.channelIds ?? []))).toBe(true);
  });

  it('derives the plotted signals from the lab, including a legacy single-pin lab and pulse-ox raw fields', () => {
    const base = {
      profile_id: 'lab', profile_version: '1.0.0', category: 'development',
      acquisition: { analog_pin: 'A2', acquisition_mode: 'simultaneous', channels: [] },
      display: { channel_label: 'Signal' }, export: { signal_name: 'signal_counts' }
    } as unknown as LabProfile;
    expect(profileChannels(undefined)).toEqual([]);
    expect(profileChannels(base)).toEqual([
      { pin: 'A2', id: 'raw', label: 'Signal', csv_name: 'signal_counts', units: 'ADC counts' }
    ]);
    const twoChannels = { ...base, acquisition: { ...base.acquisition, channels: [
      { pin: 'A0', id: 'vmid', label: 'Vmid', csv_name: 'vmid', units: 'ADC counts' },
      { pin: 'A1', id: 'vout', label: 'Vout', csv_name: 'vout', units: 'ADC counts' }
    ] } } as LabProfile;
    expect(plotChannelsFor(twoChannels).map((channel) => channel.id)).toEqual(['vmid', 'vout']);
    const pulse = { ...base, acquisition: { ...base.acquisition, acquisition_mode: 'pulseox_4state' } } as LabProfile;
    expect(plotChannelsFor(pulse).map((channel) => channel.id)).toEqual([
      'red_tx', 'dark1_tx', 'ir_tx', 'dark2_tx', 'red_rx', 'dark1_rx', 'ir_rx', 'dark2_rx'
    ]);
    expect(labRevisionKey(twoChannels)).toBe('lab@1.0.0');
    expect(labRevisionKey({ ...twoChannels, profile_version: '1.0.1' })).not.toBe(labRevisionKey(twoChannels));
    expect(labRevisionKey(undefined)).toBe('');
  });

  it('treats the built-in Op Amp Lab as an ordinary two-channel analog lab', () => {
    const opAmp = JSON.parse(
      readFileSync(resolve(process.cwd(), 'profiles', 'op_amp_course_capture.profile.json'), 'utf8')
    ) as LabProfile;
    expect(opAmp.category).toBe('course_op_amp');
    expect(isPulseOxProfile(opAmp)).toBe(false);
    const channels = plotChannelsFor(opAmp);
    expect(channels.map((channel) => [channel.id, channel.label])).toEqual([['vmid', 'Vmid (A0)'], ['vout', 'Vout (A1)']]);
    // Counts and volts, starting in counts, like any other analog channel.
    expect(initialChannelUnits(channels)).toEqual({ vmid: 'counts', vout: 'counts' });
    for (const channel of channels) {
      expect(supportedDisplayUnits(channel.id, false, channel.allowed_conversions ?? [])).toEqual(['counts', 'volts']);
    }
    // The lab's plot defaults (one plot per signal) match the generic default.
    const fromLab = normalizePlotGroups(
      channels,
      (opAmp.plot_defaults?.groups ?? []).map((group, index) => ({ id: `profile-${index + 1}`, channelIds: group.channel_ids }))
    );
    expect(fromLab.map((group) => group.channelIds)).toEqual([['vmid'], ['vout']]);
    expect(defaultPlotGroups(opAmp.category, channels).map((group) => group.channelIds)).toEqual([['vmid'], ['vout']]);
  });
});

describe('LED-sequenced plot traces (display-only)', () => {
  // The built-in pulse-ox lab 1.0.0 (legacy 4-state), kept in profiles/legacy/.
  const legacyPulse = JSON.parse(
    readFileSync(resolve(process.cwd(), 'profiles', 'legacy', 'pulseox_tx_rx_course_capture_1.0.0.profile.json'), 'utf8')
  ) as LabProfile;
  // The built-in pulse-ox lab 1.1.0 as shipped.
  const builtInSequenced = JSON.parse(
    readFileSync(resolve(process.cwd(), 'profiles', 'pulseox_tx_rx_course_capture.profile.json'), 'utf8')
  ) as LabProfile;
  // The built-in 1.1.0 lab built from 1.0.0: tx (A0) and rx (A1), both shown by LED state.
  const sequenced: LabProfile = {
    ...legacyPulse,
    profile_version: '1.1.0',
    required_firmware: { ...legacyPulse.required_firmware, build: '0x00010007' },
    acquisition: {
      ...legacyPulse.acquisition,
      acquisition_mode: 'pulseox_sequenced',
      analog_inputs: undefined,
      led_outputs: undefined,
      sample_rate_hz: 1000,
      state_dwell_us: 1000,
      channels: [
        { pin: 'A0', id: 'tx', label: 'TX', csv_name: 'tx_counts', units: 'ADC counts', allowed_conversions: ['counts_volts'], split_by_led_state: true },
        { pin: 'A1', id: 'rx', label: 'RX', csv_name: 'rx_counts', units: 'ADC counts', allowed_conversions: ['counts_volts'], split_by_led_state: true }
      ]
    },
    plot_defaults: { groups: [{ channel_ids: ['tx'] }, { channel_ids: ['rx'] }] }
  };
  const withSplit = (tx: boolean, rx: boolean): LabProfile => ({
    ...sequenced,
    acquisition: {
      ...sequenced.acquisition,
      channels: sequenced.acquisition.channels!.map((channel) => ({ ...channel, split_by_led_state: channel.id === 'tx' ? tx : rx }))
    }
  });
  const opAmp = JSON.parse(
    readFileSync(resolve(process.cwd(), 'profiles', 'op_amp_course_capture.profile.json'), 'utf8')
  ) as LabProfile;

  it('matches the built-in pulse-ox lab 1.1.0, whose 1.0.0 stays a legacy 4-state lab', () => {
    expect(builtInSequenced.profile_version).toBe('1.1.0');
    expect(builtInSequenced.acquisition).toEqual(sequenced.acquisition);
    expect(builtInSequenced.plot_defaults).toEqual(sequenced.plot_defaults);
    expect(builtInSequenced.required_firmware).toEqual(sequenced.required_firmware);
    expect(isLedSequencedProfile(builtInSequenced)).toBe(true);
    expect(isPulseOxProfile(legacyPulse)).toBe(true);
    expect(legacyPulse.profile_version).toBe('1.0.0');
    expect(plotChannelsFor(builtInSequenced).map((trace) => trace.label)).toEqual([
      'TX — RED', 'TX — DARK 1', 'TX — IR', 'TX — DARK 2',
      'RX — RED', 'RX — DARK 1', 'RX — IR', 'RX — DARK 2'
    ]);
  });

  it('expands a channel shown by LED state into four traces and keeps the others at the full record rate', () => {
    expect(isLedSequencedProfile(sequenced)).toBe(true);
    expect(isPulseOxProfile(sequenced)).toBe(false);
    const traces = plotChannelsFor(withSplit(true, false));
    expect(traces.map((trace) => [trace.id, trace.label, trace.valueIndex, trace.ledState, trace.parentId])).toEqual([
      ['led:0:tx', 'TX — RED', 1, 0, 'tx'],
      ['led:1:tx', 'TX — DARK 1', 1, 1, 'tx'],
      ['led:2:tx', 'TX — IR', 1, 2, 'tx'],
      ['led:3:tx', 'TX — DARK 2', 1, 3, 'tx'],
      ['rx', 'RX', 2, undefined, 'rx']
    ]);
    expect(traces.every((trace) => trace.ledSequenced)).toBe(true);
    expect(ledTraceId(2, 'rx')).toBe('led:2:rx');
    // The display-only choice overrides the lab's default either way.
    expect(plotChannelsFor(sequenced, { tx: false }).map((trace) => trace.id))
      .toEqual(['tx', 'led:0:rx', 'led:1:rx', 'led:2:rx', 'led:3:rx']);
    expect(plotChannelsFor(withSplit(false, false), { rx: true }).map((trace) => trace.id))
      .toEqual(['tx', 'led:0:rx', 'led:1:rx', 'led:2:rx', 'led:3:rx']);
    expect(ledSplitChoices(sequenced, { rx: false })).toEqual([
      { id: 'tx', label: 'TX', split: true },
      { id: 'rx', label: 'RX', split: false }
    ]);
    expect(ledSplitChoices(legacyPulse)).toEqual([]);
    expect(ledSplitChoices(opAmp)).toEqual([]);
  });

  it('maps every trace to its record value: its position, a legacy raw field, or 1 + the channel after the LED state', () => {
    expect(plotChannelsFor(opAmp).map((trace) => [trace.id, trace.valueIndex, trace.parentId]))
      .toEqual([['vmid', 0, 'vmid'], ['vout', 1, 'vout']]);
    expect(plotChannelsFor(legacyPulse).map((trace) => trace.valueIndex)).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
    expect(plotChannelsFor(legacyPulse).every((trace) => trace.rawPulseField && trace.parentId === trace.id)).toBe(true);
    expect(plotChannelsFor(withSplit(false, false)).map((trace) => [trace.id, trace.valueIndex])).toEqual([['tx', 1], ['rx', 2]]);
    // Units belong to the lab channels, never to their LED-state traces.
    expect(unitChannelsFor(sequenced).map((field) => [field.id, field.valueIndex])).toEqual([['tx', 1], ['rx', 2]]);
    expect(unitChannelsFor(legacyPulse).map((field) => field.id)).toEqual(PULSEOX_PLOT_CHANNELS.map((field) => field.id));
    expect(unitChannelsFor(opAmp).map((field) => field.id)).toEqual(['vmid', 'vout']);
    // Showing a channel by LED state does not change the lab's layout key.
    expect(plotLayoutKey(sequenced)).toBe(plotLayoutKey(withSplit(false, true)));
    expect(plotLayoutKey(sequenced)).not.toBe(plotLayoutKey(legacyPulse));
    expect(plotLayoutKey(undefined)).toBe('');
  });

  it('starts with one plot per lab channel, a split channel keeping its four traces together', () => {
    const traces = plotChannelsFor(withSplit(true, false));
    // A pulse-ox category does not make the sequenced lab use the legacy TX/RX fields.
    expect(defaultPlotGroups('course_pulseox', traces).map((group) => group.channelIds)).toEqual([
      ['led:0:tx', 'led:1:tx', 'led:2:tx', 'led:3:tx'],
      ['rx']
    ]);
    // Even an ECG-category sequenced lab gets one plot per channel.
    expect(defaultPlotGroups('course_ecg', plotChannelsFor(withSplit(false, false))).map((group) => group.channelIds))
      .toEqual([['tx'], ['rx']]);
    // The legacy raw fields keep their TX and RX plots, whatever the category.
    expect(defaultPlotGroups('development', [...PULSEOX_PLOT_CHANNELS]).map((group) => group.channelIds)).toEqual([
      ['red_tx', 'dark1_tx', 'ir_tx', 'dark2_tx'],
      ['red_rx', 'dark1_rx', 'ir_rx', 'dark2_rx']
    ]);
  });

  it('honours the lab plot defaults in every mode, a channel ID standing for all of its traces', () => {
    const overlay = { ...withSplit(true, true), plot_defaults: { groups: [{ channel_ids: ['rx', 'tx'] }] } };
    expect(initialPlotGroups(overlay, plotChannelsFor(overlay)).map((group) => group.channelIds)).toEqual([[
      'led:0:rx', 'led:1:rx', 'led:2:rx', 'led:3:rx', 'led:0:tx', 'led:1:tx', 'led:2:tx', 'led:3:tx'
    ]]);
    expect(initialPlotGroups(sequenced, plotChannelsFor(sequenced)).map((group) => group.channelIds)).toEqual([
      ['led:0:tx', 'led:1:tx', 'led:2:tx', 'led:3:tx'],
      ['led:0:rx', 'led:1:rx', 'led:2:rx', 'led:3:rx']
    ]);
    // Legacy 4-state: its own plot defaults (here, one overlay) are used too.
    const legacyOverlay = { ...legacyPulse, plot_defaults: { groups: [{ channel_ids: PULSEOX_PLOT_CHANNELS.map((field) => field.id) }] } };
    expect(initialPlotGroups(legacyOverlay, [...PULSEOX_PLOT_CHANNELS])).toHaveLength(1);
    expect(initialPlotGroups(legacyPulse, [...PULSEOX_PLOT_CHANNELS]).map((group) => group.channelIds)).toEqual([
      ['red_tx', 'dark1_tx', 'ir_tx', 'dark2_tx'],
      ['red_rx', 'dark1_rx', 'ir_rx', 'dark2_rx']
    ]);
    // Simultaneous: the defaults' order is kept; without defaults, the mode's defaults apply.
    const reversed = { ...opAmp, plot_defaults: { groups: [{ channel_ids: ['vout', 'vmid'] }] } };
    expect(initialPlotGroups(reversed, plotChannelsFor(reversed)).map((group) => group.channelIds)).toEqual([['vout', 'vmid']]);
    const noDefaults = { ...opAmp, plot_defaults: undefined };
    expect(initialPlotGroups(noDefaults, plotChannelsFor(noDefaults)).map((group) => group.channelIds)).toEqual([['vmid'], ['vout']]);
  });

  it('carries visibility and plots over when a channel is shown, or no longer shown, by LED state', () => {
    const before = plotChannelsFor(withSplit(false, false));
    let groups = initialPlotGroups(sequenced, before);
    let visibility = setTraceVisibility(initialTraceVisibility(before), 'rx', false);
    expect(groups.map((group) => group.channelIds)).toEqual([['tx'], ['rx']]);

    const split = plotChannelsFor(withSplit(true, true));
    ({ groups, visibility } = remapTraceLayout(before, groups, visibility, split));
    expect(groups.map((group) => group.channelIds)).toEqual([
      ['led:0:tx', 'led:1:tx', 'led:2:tx', 'led:3:tx'],
      ['led:0:rx', 'led:1:rx', 'led:2:rx', 'led:3:rx']
    ]);
    // A hidden channel's traces stay hidden; a visible one's are shown.
    expect(visibleChannelIds(split, visibility)).toEqual(['led:0:tx', 'led:1:tx', 'led:2:tx', 'led:3:tx']);

    // Hide one LED-state trace and move another; merging back keeps the first plot of the channel.
    visibility = setTraceVisibility(visibility, 'led:1:tx', false);
    groups = assignChannelToPlot(split, groups, 'led:3:tx', 1);
    const after = plotChannelsFor(withSplit(false, true));
    ({ groups, visibility } = remapTraceLayout(split, groups, visibility, after));
    expect(groups.map((group) => group.channelIds)).toEqual([['tx'], ['led:0:rx', 'led:1:rx', 'led:2:rx', 'led:3:rx']]);
    expect(visibility.tx).toBe(true);
    expect(visibility['led:0:rx']).toBe(false);
  });

  it('colours LED-state traces by state, alternate channels in the lighter shade, and never by visibility', () => {
    const traces = plotChannelsFor(withSplit(true, true));
    const legacy = [...PULSEOX_PLOT_CHANNELS];
    expect(channelColor(traces, 'led:0:tx')).toBe(channelColor(legacy, 'red_tx'));
    expect(channelColor(traces, 'led:0:rx')).toBe(channelColor(legacy, 'red_rx'));
    expect(channelColor(traces, 'led:2:tx')).toBe(channelColor(legacy, 'ir_tx'));
    expect(new Set(traces.map((trace) => channelColor(traces, trace.id))).size).toBe(8);
    // An unsplit channel keeps its position colour whether or not the other channel is split.
    expect(channelColor(plotChannelsFor(withSplit(true, false)), 'rx')).toBe(seriesColor(1));
    expect(channelColor(plotChannelsFor(withSplit(false, false)), 'rx')).toBe(seriesColor(1));
    const all = visiblePlotSeries(traces, traces.map((trace) => trace.id));
    const someHidden = visiblePlotSeries(traces, ['led:2:tx', 'led:3:rx']);
    expect(someHidden.map((series) => series.color)).toEqual([
      all.find((series) => series.id === 'led:2:tx')?.color,
      all.find((series) => series.id === 'led:3:rx')?.color
    ]);
  });

  it('never gives a full-rate channel a colour that reads as the RED or IR traces beside it', () => {
    // Six channels: the first shown by LED state, the other five at the full record rate.
    const six: LabProfile = {
      ...sequenced,
      acquisition: {
        ...sequenced.acquisition,
        channels: ['A0', 'A1', 'A2', 'A3', 'A4', 'A5'].map((pin, index) => ({
          pin, id: `ch${index}`, label: `Channel ${index}`, csv_name: `ch${index}_counts`, units: 'ADC counts', allowed_conversions: ['counts_volts'],
          ...(index === 0 ? { split_by_led_state: true } : {})
        }))
      }
    };
    const traces = plotChannelsFor(six);
    const ledColours = traces.filter((trace) => trace.ledState !== undefined).map((trace) => channelColor(traces, trace.id));
    const fullRate = traces.filter((trace) => trace.ledState === undefined).map((trace) => channelColor(traces, trace.id));
    expect(new Set([...ledColours, ...fullRate]).size).toBe(9);
    // Neither the red nor the purple of the general palette, which resemble RED and IR.
    for (const colour of fullRate) expect([seriesColor(3), seriesColor(4)]).not.toContain(colour);
    // Unsplit, every channel still has its own colour; simultaneous labs keep theirs.
    const unsplit = plotChannelsFor(six, { ch0: false });
    expect(new Set(unsplit.map((trace) => channelColor(unsplit, trace.id))).size).toBe(6);
    expect(channelColor(unsplit, 'ch0')).toBe(seriesColor(0));
    const opAmpTraces = plotChannelsFor(opAmp);
    expect(opAmpTraces.map((trace) => channelColor(opAmpTraces, trace.id))).toEqual([seriesColor(0), seriesColor(1)]);
  });

  it('names the LED-state traces of a channel once in a plot heading', () => {
    const traces = plotChannelsFor(withSplit(true, false));
    expect(plotGroupHeading(traces, ['led:0:tx', 'led:1:tx', 'led:2:tx', 'led:3:tx'])).toBe('TX — RED, DARK 1, IR, DARK 2');
    expect(plotGroupHeading(traces, ['led:0:tx', 'rx', 'led:2:tx'])).toBe('TX — RED, IR + RX');
    expect(plotGroupHeading(traces, ['rx'])).toBe('RX');
    const legacy = [...PULSEOX_PLOT_CHANNELS];
    expect(plotGroupHeading(legacy, ['red_tx', 'ir_tx'])).toBe('TX Red + TX IR');
    expect(plotGroupHeading(traces, ['missing'])).toBe('missing');
  });
});
