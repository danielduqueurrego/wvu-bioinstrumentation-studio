/**
 * Student-facing preview validation for the recording destination. Rust repeats
 * this validation before a session is started; keeping this helper pure makes
 * the form immediately explain why Start is unavailable.
 */
export function relativeOutputFolderError(outputFolder: string): string | undefined {
  const value = outputFolder.trim();
  if (!value) return undefined;
  if (value.includes('\0')) return 'Output folder contains an invalid character.';
  if (/^[a-zA-Z]:[\\/]/.test(value) || /^[\\/]/.test(value)) {
    return 'Output folder must be relative to the Project folder.';
  }
  if (value.split(/[\\/]+/).some((part) => part === '..')) {
    return 'Output folder must stay inside the selected Project folder.';
  }
  return undefined;
}

export function effectiveRecordingFolder(projectFolder: string, outputFolder: string): string {
  const root = projectFolder.replace(/[\\/]+$/, '');
  if (!root) return '';
  const relative = outputFolder.trim();
  return relative ? `${root}\\${relative}` : root;
}
