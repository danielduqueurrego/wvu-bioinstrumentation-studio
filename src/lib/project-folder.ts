/**
 * Student-facing preview validation for the recording destination. Rust repeats
 * this validation before a session is started; keeping this helper pure makes
 * the form immediately explain why Start is unavailable. The rules mirror
 * `project_paths::checked_relative_output_folder`.
 */
export const MAX_OUTPUT_FOLDER_CHARS = 160;

const RESERVED_DEVICE_NAME = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/i;
// Characters Windows does not allow in a folder name, plus control characters.
const INVALID_NAME_CHARACTER = /[<>:"|?*\u0000-\u001f\u007f-\u009f]/;

/** Why one Output-folder name cannot be created on Windows, if it cannot. */
export function outputFolderNameError(name: string): string | undefined {
  if (INVALID_NAME_CHARACTER.test(name)) {
    return `Output folder name "${name}" cannot contain < > : " | ? * or control characters.`;
  }
  if (/[. ]$/.test(name)) return `Output folder name "${name}" cannot end with a dot or a space.`;
  const stem = name.split('.')[0].replace(/ +$/, '');
  if (RESERVED_DEVICE_NAME.test(stem)) return `Output folder name "${name}" is reserved by Windows; choose another name.`;
  return undefined;
}

export function relativeOutputFolderError(outputFolder: string): string | undefined {
  const value = outputFolder.trim();
  if (!value) return undefined;
  if (value.includes('\0')) return 'Output folder contains an invalid character.';
  if ([...value].length > MAX_OUTPUT_FOLDER_CHARS) {
    return `Output folder must be ${MAX_OUTPUT_FOLDER_CHARS} characters or fewer.`;
  }
  // Any drive prefix ("C:\x", "C:x", "a:b") or a rooted path is absolute.
  if (/^[a-zA-Z]:/.test(value) || /^[\\/]/.test(value)) {
    return 'Output folder must be relative to the Project folder.';
  }
  const names = value.split(/[\\/]+/).filter((part) => part !== '' && part !== '.');
  if (names.some((part) => part === '..')) {
    return 'Output folder must stay inside the selected Project folder.';
  }
  for (const name of names) {
    const error = outputFolderNameError(name);
    if (error) return error;
  }
  return undefined;
}

export function effectiveRecordingFolder(projectFolder: string, outputFolder: string): string {
  const root = projectFolder.replace(/[\\/]+$/, '');
  if (!root) return '';
  const relative = outputFolder.trim();
  return relative ? `${root}\\${relative}` : root;
}
