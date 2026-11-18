export type ConnectionActionState = {
  source: 'hardware' | 'simulator';
  active: boolean;
  selectedPort: string;
  failureCategory?: string | null;
};

/**
 * Problems a new handshake cannot fix: a folder or disk problem, and a lab
 * the connected firmware cannot run (Restore WVU Firmware or the instructor
 * can help instead).
 */
const NON_CONNECTION_FAILURES = new Set(['recording_storage_error', 'lab_not_supported_by_firmware']);

/**
 * Keeps recovery controls unavailable while a controller owns a session.
 * Recovery is Retry connection, then Verify Firmware, then Restore WVU
 * Firmware; the retired "Reset board and retry" is never offered.
 */
export function connectionActions(state: ConnectionActionState) {
  const canRetryHandshake = state.source === 'hardware'
    && !state.active
    && Boolean(state.selectedPort)
    && Boolean(state.failureCategory)
    && !NON_CONNECTION_FAILURES.has(state.failureCategory ?? '');
  return { canRetryHandshake };
}
