/**
 * Copies text with the asynchronous Clipboard API. Returns false when the
 * API is unavailable or refuses (the caller then shows the text selected,
 * so it can be copied with Ctrl+C).
 */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (typeof navigator === 'undefined' || !navigator.clipboard?.writeText) return false;
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
