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
