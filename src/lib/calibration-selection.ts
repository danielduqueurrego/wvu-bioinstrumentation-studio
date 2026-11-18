/**
 * Which saved calibrations the page may show for the selected lab and
 * channel. Changing the lab must never leave the previous lab's presets in
 * the list: a `list_calibrations` response is applied only when it is the
 * newest request and still matches the current lab and channel, and only
 * presets of that lab and channel are kept.
 */
import { calibrationById, mergeCalibrationPresets, type CalibrationPreset } from './calibration';
import type { LabProfile } from './labs';

/** Timed presets offered when a lab does not list its own. */
export const DEFAULT_TIMED_PRESETS_SECONDS = [10, 30, 60, 300, 600];

type CalibratableChannel = { id: string; allowed_conversions?: string[] };

/** Channels that accept a saved local linear calibration. */
export function linearCalibrationChannels<T extends CalibratableChannel>(channels: T[]): T[] {
  return channels.filter((channel) =>
    channel.allowed_conversions?.includes('linear_calibration') || channel.id === 'xgzp'
  );
}

/** Keeps `current` when the lab still offers it, else the lab's first one. */
export function calibrationChannelFor(channels: CalibratableChannel[], current: string): string {
  const offered = linearCalibrationChannels(channels);
  return offered.some((channel) => channel.id === current) ? current : offered[0]?.id ?? '';
}

/** The lab's 60 s preset when it has one, else its first preset. */
export function defaultDurationPreset(profile: LabProfile | undefined): string {
  const presets = profile?.acquisition.timed_presets_seconds ?? DEFAULT_TIMED_PRESETS_SECONDS;
  return String(presets.includes(60) ? 60 : presets[0] ?? 10);
}

export type CalibrationSelection = { profileId: string; channelId: string };

export type CalibrationListState = {
  presets: CalibrationPreset[];
  selectedId: string;
  active: CalibrationPreset | undefined;
};

function belongsTo(selection: CalibrationSelection) {
  return (preset: CalibrationPreset) =>
    preset.profile_id === selection.profileId && preset.channel_id === selection.channelId;
}

/**
 * Applies one `list_calibrations` response. Returns `undefined` (drop it)
 * when a newer request was sent or the lab or channel changed after this one
 * was sent. Otherwise only presets of the requested lab and channel are kept;
 * a preset this window just saved stays listed even if the file list has not
 * caught up yet. The selected preset stays selected only if it is still
 * listed (or is that just-saved preset).
 */
export function applyCalibrationList(
  response: { requestId: number; selection: CalibrationSelection; stored: CalibrationPreset[] },
  current: { latestRequestId: number; selection: CalibrationSelection; state: CalibrationListState }
): CalibrationListState | undefined {
  if (response.requestId !== current.latestRequestId) return undefined;
  if (
    response.selection.profileId !== current.selection.profileId
    || response.selection.channelId !== current.selection.channelId
  ) {
    return undefined;
  }
  const matches = belongsTo(response.selection);
  const presets = mergeCalibrationPresets(
    response.stored.filter(matches),
    current.state.presets.filter(matches)
  );
  const listed = calibrationById(presets, current.state.selectedId);
  if (listed) return { presets, selectedId: listed.calibration_id, active: listed };
  const active = current.state.active;
  if (active && active.calibration_id === current.state.selectedId && matches(active)) {
    return { presets: mergeCalibrationPresets(presets, [active]), selectedId: active.calibration_id, active };
  }
  return { presets, selectedId: '', active: undefined };
}
