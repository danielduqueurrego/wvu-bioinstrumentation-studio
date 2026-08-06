import { describe, expect, it } from 'vitest';
import { validationControls } from './validation-controls';

describe('validation controls', () => {
  it('keeps Student mode review-only', () => {
    const controls = validationControls({ instructor: false, profileCategory: 'ecg', safetyAcknowledged: true, validationDraftSelected: true, sessionDisconnected: true, evidenceStatus: 'finalized' });
    expect(controls.canCreateDraft).toBe(false);
    expect(controls.canStartRun).toBe(false);
    expect(controls.canExportOrRetire).toBe(false);
    expect(controls.blockedExplanation).toContain('Student mode');
  });

  it('requires ECG or EMG profile, safety acknowledgement, and a draft', () => {
    expect(validationControls({ instructor: true, profileCategory: 'development', safetyAcknowledged: true, validationDraftSelected: true, sessionDisconnected: true }).canStartRun).toBe(false);
    expect(validationControls({ instructor: true, profileCategory: 'emg', safetyAcknowledged: false, validationDraftSelected: true, sessionDisconnected: true }).canStartRun).toBe(false);
    expect(validationControls({ instructor: true, profileCategory: 'emg', safetyAcknowledged: true, validationDraftSelected: false, sessionDisconnected: true }).canStartRun).toBe(false);
    expect(validationControls({ instructor: true, profileCategory: 'emg', safetyAcknowledged: true, validationDraftSelected: true, sessionDisconnected: true }).canStartRun).toBe(true);
  });

  it('does not allow authoring while a session owns acquisition', () => {
    const controls = validationControls({ instructor: true, profileCategory: 'ecg', safetyAcknowledged: true, validationDraftSelected: true, sessionDisconnected: false, evidenceStatus: 'draft' });
    expect(controls.canCreateDraft).toBe(false);
    expect(controls.canStartRun).toBe(false);
    expect(controls.canFinalize).toBe(false);
  });

  it('keeps finalization and package actions status-specific', () => {
    expect(validationControls({ instructor: true, profileCategory: 'ecg', safetyAcknowledged: true, validationDraftSelected: true, sessionDisconnected: true, evidenceStatus: 'draft' }).canFinalize).toBe(true);
    expect(validationControls({ instructor: true, profileCategory: 'ecg', safetyAcknowledged: true, validationDraftSelected: true, sessionDisconnected: true, evidenceStatus: 'draft' }).canExportOrRetire).toBe(false);
    expect(validationControls({ instructor: true, profileCategory: 'ecg', safetyAcknowledged: true, validationDraftSelected: true, sessionDisconnected: true, evidenceStatus: 'finalized' }).canExportOrRetire).toBe(true);
  });
});
