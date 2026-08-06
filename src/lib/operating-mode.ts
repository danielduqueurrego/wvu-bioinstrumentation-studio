/** The only UI mode value; acknowledgement is intentionally separate. */
export type OperatingMode = 'student' | 'instructor_authoring';

export function isInstructorMode(mode: OperatingMode) {
  return mode === 'instructor_authoring';
}

export function modeBadge(mode: OperatingMode) {
  return isInstructorMode(mode) ? 'Instructor authoring' : 'Student mode';
}
