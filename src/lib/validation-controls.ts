export type ValidationControlInput = {
  instructor: boolean;
  profileCategory?: string;
  safetyAcknowledged: boolean;
  validationDraftSelected: boolean;
  sessionDisconnected: boolean;
  evidenceStatus?: string;
};

/** UI policy mirror; Rust independently enforces every instructor-only action. */
export function validationControls(input: ValidationControlInput) {
  const moduleProfile = input.profileCategory === 'ecg' || input.profileCategory === 'emg';
  const canAuthor = input.instructor && moduleProfile && input.sessionDisconnected;
  return {
    canCreateDraft: canAuthor,
    canStartRun: canAuthor && input.safetyAcknowledged && input.validationDraftSelected,
    canCompleteRun: input.instructor && input.validationDraftSelected,
    canFinalize: input.instructor && input.sessionDisconnected && input.evidenceStatus === 'draft',
    canExportOrRetire: input.instructor && input.sessionDisconnected && input.evidenceStatus === 'finalized',
    blockedExplanation: !input.instructor
      ? 'Student mode may review validation status but cannot author validation evidence.'
      : !moduleProfile
        ? 'Select the locked ECG or EMG profile for analog-interface bench validation.'
        : !input.safetyAcknowledged
          ? 'Acknowledge bench-only use with no person or electrode system connected.'
          : ''
  };
}
