/**
 * Which saved calibrations the page may show for the selected lab and
 * channel. Changing the lab must never leave the previous lab's presets in
 * the list: a `list_calibrations` response is applied only when it is the
 * newest request and still matches the current lab and channel, and only
 * presets of that lab and channel are kept.
 */
import { acceptsLinearCalibration, calibrationById, mergeCalibrationPresets, type CalibrationPreset } from './calibration';
import type { LabProfile } from './labs';

/** Timed presets offered when a lab does not list its own. */
export const DEFAULT_TIMED_PRESETS_SECONDS = [10, 30, 60, 300, 600];

type CalibratableChannel = { id: string; allowed_conversions?: string[] };

/**
 * Channels that accept a saved local linear calibration: those whose lab
 * allows `linear_calibration` (the 'xgzp' ID decides only for a legacy lab
 * that lists no conversions).
 */
export function linearCalibrationChannels<T extends CalibratableChannel>(channels: T[]): T[] {
  return channels.filter(acceptsLinearCalibration);
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
 * was sent. Otherwise the saved files are authoritative: only presets of the
 * requested lab and channel are kept, and a preset that is no longer on disk
 * (deleted in another window, or its file removed) is dropped, and with it
 * its selection, so it cannot be frozen into a new recording. The only
 * exception is the preset this window has just saved (`justSavedId`), kept
 * while the file list catches up. The selected preset stays selected only if
 * it is still listed.
 */
export function applyCalibrationList(
  response: { requestId: number; selection: CalibrationSelection; stored: CalibrationPreset[] },
  current: { latestRequestId: number; selection: CalibrationSelection; state: CalibrationListState; justSavedId?: string }
): CalibrationListState | undefined {
  if (response.requestId !== current.latestRequestId) return undefined;
  if (
    response.selection.profileId !== current.selection.profileId
    || response.selection.channelId !== current.selection.channelId
  ) {
    return undefined;
  }
  const matches = belongsTo(response.selection);
  const justSaved = current.justSavedId
    ? [...current.state.presets, ...(current.state.active ? [current.state.active] : [])]
      .find((preset) => preset.calibration_id === current.justSavedId && matches(preset))
    : undefined;
  const presets = mergeCalibrationPresets(response.stored.filter(matches), justSaved ? [justSaved] : []);
  const listed = calibrationById(presets, current.state.selectedId);
  if (listed) return { presets, selectedId: listed.calibration_id, active: listed };
  return { presets, selectedId: '', active: undefined };
}
