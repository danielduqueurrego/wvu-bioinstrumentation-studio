/** Pure reconciliation used by the root-level Arduino-board cache. */
export type CachedBoard = { port: string; name: string; fqbn: string; serial_number?: string };

export type BoardCacheResult = {
  boards: CachedBoard[];
  selectedPort: string;
  verificationPort?: string;
  selectionCleared: boolean;
};

export function reconcileBoardCache(currentPort: string, discovered: CachedBoard[]): BoardCacheResult {
  if (currentPort && discovered.some((board) => board.port === currentPort)) {
    return { boards: discovered, selectedPort: currentPort, selectionCleared: false };
  }
  if (discovered.length === 1) {
    return {
      boards: discovered,
      selectedPort: discovered[0].port,
      verificationPort: discovered[0].port,
      selectionCleared: Boolean(currentPort)
    };
  }
  return { boards: discovered, selectedPort: '', selectionCleared: Boolean(currentPort) };
}
