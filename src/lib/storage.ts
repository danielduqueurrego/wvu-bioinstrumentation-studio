/**
 * Storage display before and during a recording. The projection comes from
 * the backend `estimate_recording_storage` command (the same BMEG and CSV
 * sizes the storage guard uses, including the pulse-ox cycle rate set by the
 * state dwell), so the page keeps no second size formula.
 */
import type { RecordingCalibration } from './calibration';
import type { RecordingDurationRequest } from './duration';

/** Wire shape of the Rust `EstimateRecordingStorageRequest` (snake_case). */
export type StorageEstimateRequest = {
  profile_id: string;
  duration: RecordingDurationRequest;
  calibration: RecordingCalibration | null;
  project_folder: string | null;
};

/** The Rust response: `RecordingStorageEstimate` plus the folder's free space. */
export type RecordingStorageEstimate = {
  records_per_second: number;
  bmeg_bytes_per_second: number;
  csv_bytes_per_second: number;
  /** BMEG plus CSV for a timed recording; null when it runs until stopped. */
  projected_bytes: number | null;
  /** Projected files plus the critical reserve; null until stopped. */
  required_free_bytes: number | null;
  critical_free_bytes: number;
  /** Free space in the Project folder now; null when unknown. */
  available_free_bytes: number | null;
};

/** The command takes one named `request` argument, like Start. */
export function storageEstimateInvokePayload(request: StorageEstimateRequest): { request: StorageEstimateRequest } {
  return { request };
}

const MIB = 1024 * 1024;
const GIB = 1024 * MIB;

function mebibytes(bytes: number): string {
  const value = bytes / MIB;
  return value >= 100 ? value.toFixed(0) : value.toFixed(value >= 10 ? 1 : 2);
}

/**
 * Free space for the Storage tile. Missing values arrive from Rust as `null`
 * (not `undefined`), and an unknown value is "not yet checked", never 0.
 */
export function formatStorage(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes)) return 'not yet checked';
  return `${(bytes / GIB).toFixed(2)} GiB free`;
}

/**
 * The free space to show: the live value the recording measures, falling
 * back to the Project-folder check made before Start (and the other way
 * round while idle, when the pre-Start check is the newer one).
 */
export function displayedFreeBytes(
  active: boolean,
  sessionBytes: number | null | undefined,
  estimateBytes: number | null | undefined
): number | null {
  const first = active ? sessionBytes : estimateBytes;
  const second = active ? estimateBytes : sessionBytes;
  return first ?? second ?? null;
}

/** The size line under Recording duration. */
export function storageEstimateText(
  estimate: RecordingStorageEstimate | undefined,
  mode: RecordingDurationRequest['mode']
): string {
  if (!estimate) return 'Storage estimate not available yet.';
  if (mode === 'until_stopped' || estimate.projected_bytes === null) {
    const perMinute = (estimate.bmeg_bytes_per_second + estimate.csv_bytes_per_second) * 60;
    return `Approx. ${mebibytes(perMinute)} MiB per minute (raw BMEG + CSV); free space is checked every 15 seconds.`;
  }
  return `Approx. ${mebibytes(estimate.projected_bytes)} MiB raw BMEG + CSV for this timed run.`;
}

/** The thresholds line; the stop threshold also keeps room for the CSV. */
export function storageThresholdText(estimate: RecordingStorageEstimate | undefined): string {
  const critical = estimate ? mebibytes(estimate.critical_free_bytes) : '250';
  return `Warning below 1 GiB free; controlled stop below ${critical} MiB free plus room for the recording's CSV.`;
}

/**
 * A warning when a timed recording would not fit: the storage guard would
 * stop it early. Not shown when either value is unknown.
 */
export function storageShortfallWarning(estimate: RecordingStorageEstimate | undefined): string | undefined {
  if (!estimate || estimate.required_free_bytes === null || estimate.available_free_bytes === null) return undefined;
  if (estimate.available_free_bytes >= estimate.required_free_bytes) return undefined;
  return `This timed recording needs about ${mebibytes(estimate.required_free_bytes)} MiB free, but the Project folder's drive has ${mebibytes(estimate.available_free_bytes)} MiB. `
    + 'Free some space, choose another Project folder, or choose a shorter duration; otherwise the recording stops early to protect the drive.';
}
