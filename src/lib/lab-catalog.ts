import { isTimedDurationValid, MIN_TIMED_SECONDS } from './duration';
import type { LabProfile } from './labs';

export type LabDraftSession = {
  draft: LabProfile;
  baseVersion: string | undefined;
  requestId: string;
};

/**
 * A frontend editor session is deliberately detached from the persisted lab
 * catalog. The only payload that can create a version is `saveArguments`.
 */
export function beginEditSession(draft: LabProfile, requestId: string): LabDraftSession {
  return { draft, baseVersion: draft.profile_version, requestId };
}

export function beginNewLabSession(draft: LabProfile, requestId: string): LabDraftSession {
  return { draft, baseVersion: undefined, requestId };
}

export function saveArguments(session: LabDraftSession) {
  return {
    draft: session.draft,
    baseVersion: session.baseVersion ?? null,
    requestId: session.requestId
  };
}

export function labSourceLabel(source: LabProfile['source']) {
  return source === 'built_in' ? 'Factory' : source === 'imported' ? 'Imported' : 'Instructor';
}

/** Prefix for a failed Save. Not every failure is an invalid configuration. */
export function labSaveErrorMessage(reason: unknown): string {
  return `Could not save lab: ${String(reason)}`;
}

export type DurationMode = 'timed' | 'until_stopped';
const DURATION_MODES: DurationMode[] = ['timed', 'until_stopped'];

/**
 * The duration modes the selected lab allows, in display order. Rust enforces
 * the same rule at Start (`AcquisitionSettings::duration_refusal`).
 */
export function labDurationModes(profile: LabProfile | undefined): DurationMode[] {
  if (!profile) return [...DURATION_MODES];
  const allowed = profile.acquisition.allowed_duration_modes ?? [];
  const modes = DURATION_MODES.filter((mode) => allowed.includes(mode));
  return modes.length ? modes : [...DURATION_MODES];
}

/** The mode to show: the current one if the lab allows it, else the first allowed. */
export function allowedDurationMode(profile: LabProfile | undefined, current: DurationMode): DurationMode {
  const modes = labDurationModes(profile);
  return modes.includes(current) ? current : modes[0];
}

/** Shortest custom timed duration the lab accepts (never below the global 10 s). */
export function labMinimumCustomSeconds(profile: LabProfile | undefined): number {
  return Math.max(MIN_TIMED_SECONDS, Number(profile?.acquisition.minimum_custom_duration_seconds) || 0);
}

/** A preset is always allowed; any other whole-second duration must reach the lab minimum. */
export function labTimedDurationAllowed(profile: LabProfile | undefined, seconds: number): boolean {
  if (!isTimedDurationValid(seconds)) return false;
  if (!profile) return true;
  return (profile.acquisition.timed_presets_seconds ?? []).includes(seconds)
    || seconds >= labMinimumCustomSeconds(profile);
}

/**
 * Student-facing reason the lab refuses this duration. For a whole number of
 * seconds the text is exactly Rust's `AcquisitionSettings::duration_refusal`;
 * a fractional or non-numeric value (which never reaches Rust) also says that
 * whole seconds are required.
 */
export function labDurationRefusal(
  profile: LabProfile | undefined,
  mode: DurationMode,
  seconds: number
): string | undefined {
  if (!labDurationModes(profile).includes(mode)) {
    return mode === 'timed'
      ? 'This lab does not allow timed recordings. Choose Until stopped, then start again.'
      : 'This lab does not allow Until-stopped recordings. Choose a timed duration, then start again.';
  }
  if (mode === 'timed' && !labTimedDurationAllowed(profile, seconds)) {
    const minimum = labMinimumCustomSeconds(profile);
    return Number.isInteger(seconds)
      ? `This lab requires timed recordings of at least ${minimum} seconds, or one of its preset durations. Choose a longer duration, then start again.`
      : `Timed recordings must be whole seconds. This lab requires at least ${minimum} seconds, or one of its preset durations.`;
  }
  return undefined;
}

/** `reset_local_lab_customizations` result. */
export type LabResetOutcome = {
  backup_path?: string | null;
  removed_local_revisions: number;
};

function revisionCount(count: number): string {
  return count === 1 ? '1 local instructor/imported lab revision' : `${count} local instructor/imported lab revisions`;
}

/**
 * The Reset confirmation. `listedRevisions` comes from the Lab Manager list;
 * stored labs that could not be loaded are not listed but are removed too.
 * Another open window may have saved more, so the count is "currently".
 */
export function resetConfirmationText(listedRevisions: number, skippedLabs: number): string {
  const unloadable = skippedLabs > 0
    ? `, plus ${skippedLabs === 1 ? '1 stored lab' : `${skippedLabs} stored labs`} that could not be loaded`
    : '';
  return `Reset local lab customizations? This removes every local instructor/imported lab from this computer's lab list (currently ${revisionCount(listedRevisions)}${unloadable}). Export any lab you want to keep first. A copy of the current catalog file is saved before the reset. Shipped course defaults and existing recording files are preserved.`;
}

/** Status after a reset, with the count Rust actually removed. */
export function resetOutcomeText(outcome: LabResetOutcome | null | undefined): string {
  const removed = outcome ? ` Removed ${revisionCount(outcome.removed_local_revisions)}.` : '';
  const backup = outcome?.backup_path ? ` A copy of the previous catalog was saved to ${outcome.backup_path}.` : '';
  return `Reset local lab customizations.${removed} Shipped factory course labs are active. Existing recordings retain their embedded snapshots.${backup}`;
}

export type SkippedLab = {
  profile_id?: string | null;
  profile_version?: string | null;
  display_name?: string | null;
  reason: string;
};

/** `get_catalog_status`: lab catalog health plus unreadable calibration files. */
export type CatalogStatus = {
  writable: boolean;
  load_error?: string | null;
  skipped_labs: SkippedLab[];
  recovered_from?: string | null;
  /** Unreadable leftovers of an interrupted save, renamed aside and kept. */
  kept_unreadable_files?: string[];
  calibration_issues?: string[];
};

function skippedLabName(lab: SkippedLab): string {
  const name = lab.display_name ?? lab.profile_id ?? 'Unnamed lab';
  return lab.profile_version ? `${name} ${lab.profile_version}` : name;
}

/**
 * The Lab panel notice, shown in both modes. Students get a short pointer to
 * their instructor; instructors get the detail needed to repair the catalog.
 */
export function catalogNotice(status: CatalogStatus | undefined, instructor: boolean): string | undefined {
  if (!status) return undefined;
  const parts: string[] = [];
  if (!status.writable) {
    parts.push(instructor
      ? `The local lab catalog could not be loaded, so only the factory labs are shown and lab changes are disabled. Close other programs that may hold the file and restart the application; if the problem remains, Manage Labs → Reset local customizations keeps a copy of the file and starts a new catalog. Detail: ${status.load_error ?? 'unknown'}`
      : 'Local labs could not be loaded, so only the factory labs are shown. Ask your instructor.');
  }
  const skipped = status.skipped_labs ?? [];
  if (skipped.length) {
    parts.push(instructor
      ? `${skipped.length === 1 ? 'One local lab' : `${skipped.length} local labs`} could not be loaded and ${skipped.length === 1 ? 'is' : 'are'} hidden: ${skipped.map((lab) => `${skippedLabName(lab)} (${lab.reason})`).join('; ')}. ${skipped.length === 1 ? 'It is' : 'They are'} kept in the catalog file; the other labs are unaffected.`
      : `${skipped.length === 1 ? 'One local lab' : `${skipped.length} local labs`} could not be loaded and ${skipped.length === 1 ? 'is' : 'are'} not shown. Ask your instructor.`);
  }
  if (instructor && status.recovered_from) {
    parts.push(`The local lab catalog was restored from ${status.recovered_from} after an interrupted save. Check the lab versions.`);
  }
  const kept = status.kept_unreadable_files ?? [];
  if (kept.length) {
    parts.push(instructor
      ? `When the app started, the local lab catalog file was missing and the ${kept.length === 1 ? 'copy' : 'copies'} left by an interrupted save could not be read, so local labs started empty. ${kept.length === 1 ? 'That copy was' : 'Those copies were'} kept for manual recovery: ${kept.join('; ')}.`
      : 'Saved local labs could not be loaded, so only the factory labs may be shown. Ask your instructor.');
  }
  const calibrationIssues = status.calibration_issues ?? [];
  if (calibrationIssues.length) {
    parts.push(instructor
      ? calibrationIssues.join(' ')
      : `${calibrationIssues.length === 1 ? 'One saved calibration file' : `${calibrationIssues.length} saved calibration files`} could not be read and ${calibrationIssues.length === 1 ? 'was' : 'were'} skipped.`);
  }
  return parts.length ? parts.join(' ') : undefined;
}
