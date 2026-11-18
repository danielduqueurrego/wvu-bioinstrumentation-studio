import { describe, expect, it } from 'vitest';
import {
  displayedFreeBytes,
  formatStorage,
  storageEstimateInvokePayload,
  storageEstimateText,
  storageShortfallWarning,
  storageThresholdText,
  type RecordingStorageEstimate
} from './storage';

const MIB = 1024 * 1024;

const timed: RecordingStorageEstimate = {
  records_per_second: 1000,
  bmeg_bytes_per_second: 20_000,
  csv_bytes_per_second: 30_000,
  projected_bytes: 60 * 50_000 + 2 * MIB,
  required_free_bytes: 60 * 50_000 + 2 * MIB + 250 * MIB,
  critical_free_bytes: 250 * MIB,
  available_free_bytes: 8 * 1024 * MIB
};

describe('storage display', () => {
  it('shows an unknown free space as not yet checked, never as 0.00 GiB', () => {
    // Rust sends a missing Option as null.
    expect(formatStorage(null)).toBe('not yet checked');
    expect(formatStorage(undefined)).toBe('not yet checked');
    expect(formatStorage(0)).toBe('0.00 GiB free');
    expect(formatStorage(8 * 1024 * MIB)).toBe('8.00 GiB free');
  });

  it('prefers the live value while recording and the pre-Start check while idle', () => {
    expect(displayedFreeBytes(true, 5, 9)).toBe(5);
    expect(displayedFreeBytes(true, null, 9)).toBe(9);
    expect(displayedFreeBytes(false, 5, 9)).toBe(9);
    expect(displayedFreeBytes(false, 5, null)).toBe(5);
    expect(displayedFreeBytes(false, null, undefined)).toBeNull();
  });

  it('describes the backend projection, including the CSV', () => {
    expect(storageEstimateText(timed, 'timed')).toBe('Approx. 4.86 MiB raw BMEG + CSV for this timed run.');
    expect(storageEstimateText({ ...timed, projected_bytes: null, required_free_bytes: null }, 'until_stopped'))
      .toBe('Approx. 2.86 MiB per minute (raw BMEG + CSV); free space is checked every 15 seconds.');
    expect(storageEstimateText(undefined, 'timed')).toMatch(/not available/);
    expect(storageThresholdText(timed)).toMatch(/controlled stop below 250 MiB free plus room for the recording's CSV/);
  });

  it('warns before Start when a timed recording would not fit', () => {
    expect(storageShortfallWarning(timed)).toBeUndefined();
    const low = { ...timed, available_free_bytes: 200 * MIB };
    expect(storageShortfallWarning(low)).toMatch(/needs about 255 MiB free, but the Project folder's drive has 200 MiB/);
    expect(storageShortfallWarning({ ...low, available_free_bytes: null })).toBeUndefined();
    expect(storageShortfallWarning({ ...low, required_free_bytes: null })).toBeUndefined();
  });

  it('sends the estimate request under the Rust field names', () => {
    const request = {
      profile_id: 'lab',
      duration: { mode: 'timed' as const, seconds: 60 },
      calibration: null,
      project_folder: 'C:\\BMEG'
    };
    expect(storageEstimateInvokePayload(request)).toEqual({ request });
    expect(Object.keys(storageEstimateInvokePayload(request).request).sort())
      .toEqual(['calibration', 'duration', 'profile_id', 'project_folder']);
  });
});
