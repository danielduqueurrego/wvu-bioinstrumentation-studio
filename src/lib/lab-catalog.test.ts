import { describe, expect, it } from 'vitest';
import {
  allowedDurationMode, beginEditSession, beginNewLabSession, catalogNotice, labDurationModes, labDurationRefusal, labMinimumCustomSeconds,
  labSaveErrorMessage, labSourceLabel, labTimedDurationAllowed, resetConfirmationText, resetOutcomeText, saveArguments, type CatalogStatus
} from './lab-catalog';
import type { LabProfile } from './labs';

const lab = {
  schema_version: 1, profile_id: 'wvu.bmeg420l.emg.force.course.capture.v1', profile_version: '1.0.0',
  display_name: 'EMG + Force — Course Capture', category: 'course_emg_force', status: 'draft', source: 'instructor',
  description: 'Draft', target_board: 'Arduino UNO R4 WiFi', fqbn: 'arduino:renesas_uno:unor4wifi',
  required_firmware: { protocol_major: 0, protocol_minor_min: 3, build: '0x00010003', device: '0x554E4F34' },
  acquisition: { analog_pin: 'A0', adc_resolution_bits: 12, sample_rate_hz: 1000, allowed_duration_modes: ['timed'], timed_presets_seconds: [10], minimum_custom_duration_seconds: 10, acquisition_mode: 'simultaneous', channels: [] },
  display: { primary_quantity: 'arduino_input_volts', channel_label: 'EMG', raw_units_label: 'ADC counts', voltage_units_label: 'V', voltage_reference_v: 5, plot_min_v: 0, plot_max_v: 5 },
  safety: { bench_only: false, human_connection_authorized: false, not_medical_device: true, notices: [] },
  export: { signal_name: 'emg', include_profile_snapshot: true }, integrity: { canonical_hash_algorithm: 'SHA-256', canonical_hash: '' }
} as LabProfile;

describe('Lab Manager detached editor sessions', () => {
  it('keeps the edit base and operation ID stable across a save retry', () => {
    const session = beginEditSession(lab, 'save-emg-0001');
    expect(saveArguments(session)).toEqual({ draft: lab, baseVersion: '1.0.0', requestId: 'save-emg-0001' });
    expect(saveArguments(session)).toEqual({ draft: lab, baseVersion: '1.0.0', requestId: 'save-emg-0001' });
  });

  it('marks duplicate and blank drafts as new IDs without a base revision', () => {
    const session = beginNewLabSession({ ...lab, profile_id: 'team2.emg' }, 'save-team-0001');
    expect(saveArguments(session).baseVersion).toBeNull();
  });

  it('makes factory and local source labels unambiguous', () => {
    expect(labSourceLabel('built_in')).toBe('Factory');
    expect(labSourceLabel('instructor')).toBe('Instructor');
    expect(labSourceLabel('imported')).toBe('Imported');
  });
});

const withDurations = (modes: string[], minimum: number, presets: number[]) => ({
  ...lab,
  acquisition: { ...lab.acquisition, allowed_duration_modes: modes, minimum_custom_duration_seconds: minimum, timed_presets_seconds: presets }
}) as LabProfile;

describe('lab duration rules', () => {
  it('offers only the duration modes the lab allows and falls back to an allowed one', () => {
    expect(labDurationModes(undefined)).toEqual(['timed', 'until_stopped']);
    expect(labDurationModes(withDurations(['until_stopped', 'timed'], 10, [10]))).toEqual(['timed', 'until_stopped']);
    const timedOnly = withDurations(['timed'], 10, [10]);
    expect(labDurationModes(timedOnly)).toEqual(['timed']);
    expect(allowedDurationMode(timedOnly, 'until_stopped')).toBe('timed');
    expect(allowedDurationMode(withDurations(['until_stopped'], 10, [10]), 'timed')).toBe('until_stopped');
    expect(labDurationRefusal(timedOnly, 'until_stopped', 0)).toMatch(/does not allow Until-stopped/);
  });

  it('applies the lab minimum to custom durations but always allows a preset', () => {
    const lab30 = withDurations(['timed', 'until_stopped'], 30, [10, 60]);
    expect(labMinimumCustomSeconds(lab30)).toBe(30);
    expect(labMinimumCustomSeconds(withDurations(['timed'], 5, [10]))).toBe(10);
    expect(labTimedDurationAllowed(lab30, 10)).toBe(true);
    expect(labTimedDurationAllowed(lab30, 20)).toBe(false);
    expect(labTimedDurationAllowed(lab30, 45)).toBe(true);
    expect(labTimedDurationAllowed(lab30, 45.5)).toBe(false);
    expect(labDurationRefusal(lab30, 'timed', 20)).toMatch(/at least 30 seconds/);
    expect(labDurationRefusal(lab30, 'until_stopped', 0)).toBeUndefined();
    // Rust's AcquisitionSettings::duration_refusal text, word for word.
    expect(labDurationRefusal(lab30, 'timed', 20)).toBe(
      'This lab requires timed recordings of at least 30 seconds, or one of its preset durations. Choose a longer duration, then start again.'
    );
    expect(labDurationRefusal(lab30, 'timed', 45.5)).toMatch(/whole seconds/);
  });
});

describe('Reset local customizations wording', () => {
  it('counts listed and unloadable labs and reports what Rust removed', () => {
    expect(resetConfirmationText(1, 0)).toMatch(/currently 1 local instructor\/imported lab revision\)/);
    expect(resetConfirmationText(3, 2)).toMatch(/currently 3 local instructor\/imported lab revisions, plus 2 stored labs that could not be loaded\)/);
    expect(resetOutcomeText({ backup_path: 'C:\\x\\lab_state.reset-1.json', removed_local_revisions: 4 }))
      .toMatch(/Removed 4 local instructor\/imported lab revisions\..*saved to C:\\x\\lab_state\.reset-1\.json\./);
    expect(resetOutcomeText(null)).toMatch(/^Reset local lab customizations\. Shipped/);
  });
});

describe('lab catalog notices', () => {
  const healthy: CatalogStatus = { writable: true, load_error: null, skipped_labs: [], recovered_from: null, calibration_issues: [] };

  it('says nothing when the catalog is healthy', () => {
    expect(catalogNotice(undefined, false)).toBeUndefined();
    expect(catalogNotice(healthy, true)).toBeUndefined();
  });

  it('tells students to ask the instructor and gives instructors the detail', () => {
    const skipped: CatalogStatus = {
      ...healthy,
      skipped_labs: [{ profile_id: 'team.lab', profile_version: '1.0.2', display_name: 'Team Lab', reason: 'hash mismatch' }]
    };
    expect(catalogNotice(skipped, false)).toBe('One local lab could not be loaded and is not shown. Ask your instructor.');
    expect(catalogNotice(skipped, true)).toMatch(/Team Lab 1\.0\.2 \(hash mismatch\)/);
    const unavailable: CatalogStatus = { ...healthy, writable: false, load_error: 'invalid profile JSON' };
    expect(catalogNotice(unavailable, false)).toMatch(/only the factory labs are shown/);
    expect(catalogNotice(unavailable, true)).toMatch(/invalid profile JSON/);
    expect(catalogNotice({ ...healthy, recovered_from: 'lab_state.json.backup' }, false)).toBeUndefined();
    expect(catalogNotice({ ...healthy, recovered_from: 'lab_state.json.backup' }, true)).toMatch(/restored from lab_state\.json\.backup/);
    expect(catalogNotice({ ...healthy, calibration_issues: ['Calibration file a.json failed validation and was skipped.'] }, false))
      .toBe('One saved calibration file could not be read and was skipped.');
    const kept: CatalogStatus = { ...healthy, kept_unreadable_files: ['C:\\p\\lab_state.corrupt-1-1.json'] };
    expect(catalogNotice(kept, true)).toMatch(/kept for manual recovery: C:\\p\\lab_state\.corrupt-1-1\.json/);
    expect(catalogNotice(kept, false)).toMatch(/Ask your instructor/);
  });

  it('uses a neutral prefix for a failed save', () => {
    expect(labSaveErrorMessage('the lab catalog is busy')).toBe('Could not save lab: the lab catalog is busy');
  });
});
