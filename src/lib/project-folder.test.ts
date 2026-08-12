import { describe, expect, it } from 'vitest';
import { effectiveRecordingFolder, relativeOutputFolderError } from './project-folder';

describe('Project-folder recording destinations', () => {
  it('shows a nested trial beneath the selected project folder', () => {
    expect(effectiveRecordingFolder('C:\\Users\\Student\\Documents\\BMEG 420L', 'Participant01\\Trial03'))
      .toBe('C:\\Users\\Student\\Documents\\BMEG 420L\\Participant01\\Trial03');
  });

  it('uses the project folder itself when Output folder is blank', () => {
    expect(effectiveRecordingFolder('C:\\Users\\Student\\Documents\\BMEG 420L\\', '  '))
      .toBe('C:\\Users\\Student\\Documents\\BMEG 420L');
  });

  it('does not show a misleading rooted trial path before the Project folder loads', () => {
    expect(effectiveRecordingFolder('', 'Trial01')).toBe('');
  });

  it('rejects absolute and traversal output folders before recording starts', () => {
    expect(relativeOutputFolderError('C:\\outside')).toMatch(/relative/i);
    expect(relativeOutputFolderError('..\\outside')).toMatch(/inside/i);
    expect(relativeOutputFolderError('Participant01\\Trial03')).toBeUndefined();
  });
});
