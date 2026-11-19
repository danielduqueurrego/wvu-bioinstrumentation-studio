/**
 * Student-facing text for a rejected Tauri command.
 *
 * Firmware commands reject with a structured `FirmwareFailure`
 * (`{ title, explanation, recommended_action, ... }`); older builds sent the
 * same object as a JSON string. Other commands reject with a plain string.
 * Technical details stay out of the student message.
 */
export type CommandFailure = {
  title?: string;
  explanation?: string;
  recommended_action?: string;
};

const FALLBACK_MESSAGE = 'The operation could not be completed. Open Advanced details if the problem continues.';

function structuredFailure(error: unknown): Record<string, unknown> | undefined {
  let candidate: unknown = error;
  if (typeof error === 'string') {
    const text = error.trim();
    if (!text.startsWith('{')) return undefined;
    try {
      candidate = JSON.parse(text);
    } catch {
      return undefined;
    }
  }
  if (!candidate || typeof candidate !== 'object' || candidate instanceof Error) return undefined;
  return candidate as Record<string, unknown>;
}

function sentence(text: string): string {
  const trimmed = text.trim();
  return /[.!?…]$/.test(trimmed) ? trimmed : `${trimmed}.`;
}

export function formatCommandError(error: unknown): string {
  const failure = structuredFailure(error);
  if (failure) {
    const parts = [failure.title, failure.explanation, failure.recommended_action]
      .filter((part): part is string => typeof part === 'string' && part.trim() !== '')
      .map(sentence);
    if (parts.length) return [...new Set(parts)].join(' ');
    if (typeof failure.userMessage === 'string' && failure.userMessage.trim()) return failure.userMessage.trim();
    if (typeof failure.message === 'string' && failure.message.trim()) return failure.message.trim();
    return FALLBACK_MESSAGE;
  }
  if (error instanceof Error && error.message.trim()) return error.message.trim();
  const text = typeof error === 'string' ? error.trim() : '';
  return text || FALLBACK_MESSAGE;
}

/**
 * Whether Repair Arduino tools can fix a rejected tool preparation or repair.
 * Only an explicit `repairable: false` (a missing or damaged installation,
 * which needs a reinstall) says no; any other failure may be repaired.
 */
export function commandErrorRepairable(error: unknown): boolean {
  return structuredFailure(error)?.repairable !== false;
}

/**
 * Whether a failed Repair Arduino tools left the tools already in use in
 * place (`ArduinoToolsFailure.previous_runtime_kept`). Only an explicit
 * `true` says so; anything else is treated as tools that need attention.
 */
export function commandErrorKeptPreviousTools(error: unknown): boolean {
  return structuredFailure(error)?.previous_runtime_kept === true;
}

/**
 * The `category` of a structured failure (`ArduinoToolsFailure.category`,
 * for example `runtime_path_too_long`), or '' when there is none. Messages
 * map it to student text (`arduinoToolsFailureMessage`); it is never shown.
 */
export function commandErrorCategory(error: unknown): string {
  const category = structuredFailure(error)?.category;
  return typeof category === 'string' ? category.trim() : '';
}

/** The technical detail of a structured failure, for Advanced details only. */
export function commandErrorDetail(error: unknown): string {
  const failure = structuredFailure(error);
  if (failure) {
    return typeof failure.technical_details === 'string' ? failure.technical_details.trim() : '';
  }
  if (error instanceof Error) return error.message.trim();
  return typeof error === 'string' ? error.trim() : '';
}
