export type ConnectionActionState = {
  source: 'hardware' | 'simulator';
  active: boolean;
  selectedPort: string;
  failureCategory?: string;
};

const RESETTABLE_FAILURES = new Set([
  'port_open_no_bytes',
  'protocol_crc_failure',
  'handshake_incomplete'
]);

/** Keeps recovery controls unavailable while a controller owns a session. */
export function connectionActions(state: ConnectionActionState) {
  const canRetryHandshake = state.source === 'hardware'
    && !state.active
    && Boolean(state.selectedPort)
    && Boolean(state.failureCategory);
  return {
    canRetryHandshake,
    canReset: canRetryHandshake && RESETTABLE_FAILURES.has(state.failureCategory ?? '')
  };
}
