import { describe, expect, it } from 'vitest';
import { effectiveRecordingFolder, MAX_OUTPUT_FOLDER_CHARS, relativeOutputFolderError } from './project-folder';

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

  it('rejects drive-relative paths such as C:foo, as Rust does', () => {
    expect(relativeOutputFolderError('C:foo')).toMatch(/relative/i);
    expect(relativeOutputFolderError('a:b')).toMatch(/relative/i);
  });

  it('rejects names Windows cannot create, mirroring the Rust rules', () => {
    for (const invalid of [
      'NUL', 'con', 'Trial\\COM1', 'lpt9.txt', 'AUX .data', 'Trial:1', 'Trial1.', 'Trial1 \\Run',
      'Run<1>', 'Run|1', 'Run?', 'Run*', 'Run"1"', 'Run\u0007', '...'
    ]) {
      expect(relativeOutputFolderError(invalid), invalid).toBeDefined();
    }
    for (const valid of ['Participant 01\\Trial.03', 'COM10', 'CONSOLE', 'nul_run', '.\\Trial', 'Lab6/Trial1']) {
      expect(relativeOutputFolderError(valid), valid).toBeUndefined();
    }
    expect(relativeOutputFolderError('Trial1.')).toMatch(/dot or a space/);
    expect(relativeOutputFolderError('NUL')).toMatch(/reserved/);
    expect(relativeOutputFolderError('a'.repeat(MAX_OUTPUT_FOLDER_CHARS + 1))).toMatch(/characters or fewer/);
  });
});
