/** Pure reconciliation used by the root-level Arduino-board cache. */
export type CachedBoard = { port: string; name: string; fqbn: string; serial_number?: string };

export type BoardCacheResult = {
  boards: CachedBoard[];
  selectedPort: string;
  verificationPort?: string;
  selectionCleared: boolean;
};

export type BoardCacheOptions = {
  /**
   * Verify a board that becomes selected because it is the only one listed
   * (default true). The refresh after a failed or canceled restore passes
   * false: a Verify then would replace the restore's failure, its log and
   * advice, on the Board card with a Verify result of a board that is
   * probably still in upload mode.
   */
  verifyNewSelection?: boolean;
};

export function reconcileBoardCache(
  currentPort: string,
  discovered: CachedBoard[],
  { verifyNewSelection = true }: BoardCacheOptions = {}
): BoardCacheResult {
  if (currentPort && discovered.some((board) => board.port === currentPort)) {
    return { boards: discovered, selectedPort: currentPort, selectionCleared: false };
  }
  if (discovered.length === 1) {
    return {
      boards: discovered,
      selectedPort: discovered[0].port,
      verificationPort: verifyNewSelection ? discovered[0].port : undefined,
      selectionCleared: Boolean(currentPort)
    };
  }
  return { boards: discovered, selectedPort: '', selectionCleared: Boolean(currentPort) };
}
