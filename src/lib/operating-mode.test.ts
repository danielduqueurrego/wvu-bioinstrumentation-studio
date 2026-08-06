import { describe, expect, it } from 'vitest';
import { isInstructorMode, modeBadge, type OperatingMode } from './operating-mode';

describe('acquisition operating-mode state', () => {
  it('uses one two-state operating-mode value', () => {
    const student: OperatingMode = 'student';
    expect(isInstructorMode(student)).toBe(false);
    expect(modeBadge(student)).toBe('Student mode');
    expect(isInstructorMode('instructor_authoring')).toBe(true);
    expect(modeBadge('instructor_authoring')).toBe('Instructor authoring');
  });
});
