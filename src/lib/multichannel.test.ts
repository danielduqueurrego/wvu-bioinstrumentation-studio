import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { initialChannelUnits, supportedDisplayUnits } from './calibration';
import type { LabProfile } from './labs';
import {
  assignChannelToPlot,
  channelColor,
  defaultPlotGroups,
  initialTraceVisibility,
  isPulseOxProfile,
  labRevisionKey,
  normalizePlotGroups,
  numberedVisiblePlotGroups,
  onePlotPerSignal,
  overlayAll,
  plotChannelsFor,
  profileChannels,
  PULSEOX_PLOT_CHANNELS,
  seriesColor,
  setPlotGroupCount,
  setTraceVisibility,
  shouldShowPlotLegend,
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
    expect(initialChannelUnits(channels, opAmp.category)).toEqual({ vmid: 'counts', vout: 'counts' });
    for (const channel of channels) {
      expect(supportedDisplayUnits(opAmp.category, channel.id, false, channel.allowed_conversions ?? [])).toEqual(['counts', 'volts']);
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
