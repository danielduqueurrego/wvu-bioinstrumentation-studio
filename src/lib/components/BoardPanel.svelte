<script lang="ts">
  import CopyButton from './CopyButton.svelte';
  import { BOARD_ACTION_LABELS, type BoardAction, type BoardStatus, type LastFirmwareOperation } from '$lib/board-panel';
  import type { Board } from '$lib/session-types';

  /**
   * One row: board list, Refresh Board, the state chip, and the single
   * action that fits the state. Verify Firmware and Restore WVU Firmware are
   * always present (as the fitting action or under "More board actions"),
   * enabled by `boardControls` alone, so recovery never depends on firmware
   * or recording readiness. Every button calls a page handler; this
   * component never invokes a command itself.
   */
  type Props = {
    boards: Board[];
    selectedPort: string;
    status: BoardStatus;
    controls: { canSelectBoard: boolean; canRefreshBoards: boolean; canVerifyFirmware: boolean; canRestoreFirmware: boolean };
    toolsState: 'preparing' | 'ready' | 'failed';
    toolsProblem: string;
    toolsRepairable: boolean;
    /**
     * Show the Arduino-tools problem here. False while the message center's
     * banner already shows it, so the same text never appears twice.
     */
    toolsWarningShown?: boolean;
    canRepairTools: boolean;
    lastOperation?: LastFirmwareOperation;
    /** Compatible firmware is proven on the selected port now. */
    firmwareReady: boolean;
    onSelectChange: () => void;
    onRefresh: () => void;
    onVerify: () => void;
    onRestore: () => void;
    onRepairTools: () => void;
    /** Short feedback for the page's information line. */
    onFeedback: (message: string) => void;
  };

  let {
    boards,
    selectedPort = $bindable(),
    status,
    controls,
    toolsState,
    toolsProblem,
    toolsRepairable,
    toolsWarningShown = true,
    canRepairTools,
    lastOperation,
    firmwareReady,
    onSelectChange,
    onRefresh,
    onVerify,
    onRestore,
    onRepairTools,
    onFeedback
  }: Props = $props();

  // "Refresh" is already in the row; it is highlighted rather than repeated.
  const primary = $derived<BoardAction | undefined>(status.action === 'refresh' ? undefined : status.action);
  const primaryIsVerify = $derived(primary === 'verify' || primary === 'verify_after_close');

  function run(action: BoardAction) {
    if (action === 'refresh') onRefresh();
    else if (action === 'verify' || action === 'verify_after_close') onVerify();
    else if (action === 'restore') onRestore();
    else onRepairTools();
  }

  function disabled(action: BoardAction): boolean {
    if (action === 'refresh') return !controls.canRefreshBoards;
    if (action === 'verify' || action === 'verify_after_close') return !controls.canVerifyFirmware;
    if (action === 'restore') return !controls.canRestoreFirmware;
    return !canRepairTools;
  }

  const SYMBOL = { ready: '✓', idle: '–', busy: '↻', problem: '!' } as const;
</script>

<div class="board-panel">
  <div class="board-row">
    <label class="board-select">Board
      <select id="board-select" bind:value={selectedPort} onchange={onSelectChange} disabled={!controls.canSelectBoard}>
        <option value="">Select an Arduino UNO R4 WiFi</option>
        {#each boards as board (board.port)}<option value={board.port}>{board.name} — {board.port}</option>{/each}
      </select>
    </label>
    <button type="button" class="refresh" class:primary={status.action === 'refresh'} onclick={onRefresh} disabled={!controls.canRefreshBoards}>Refresh Board</button>
    <span class="board-chip {status.tone}" data-testid="board-state"><span class="chip-symbol" aria-hidden="true">{SYMBOL[status.tone]}</span>{status.label}</span>
    {#if primary}
      {@const action = primary}
      <button
        type="button"
        id={primaryIsVerify ? 'verify-firmware-button' : 'board-primary-action'}
        class="primary-action"
        class:gold={action === 'restore'}
        onclick={() => run(action)}
        disabled={disabled(action)}
      >{BOARD_ACTION_LABELS[action]}</button>
    {/if}
  </div>
  <p class="board-hint">{status.hint}</p>

  <div class="more-actions" role="group" aria-labelledby="more-board-actions-label">
    <span id="more-board-actions-label" class="more-label">More board actions</span>
    {#if !primaryIsVerify}
      <button type="button" id="verify-firmware-button" onclick={onVerify} disabled={!controls.canVerifyFirmware}>Verify Firmware</button>
    {/if}
    {#if primary !== 'restore'}
      <button type="button" onclick={onRestore} disabled={!controls.canRestoreFirmware}>Restore WVU Firmware</button>
    {/if}
    {#if toolsState === 'failed' && toolsRepairable && primary !== 'repair_tools'}
      <button type="button" onclick={onRepairTools} disabled={!canRepairTools}>Repair Arduino tools</button>
    {/if}
  </div>
  <p class="help">Verify only reads from the board. Restore replaces the Arduino sketch with the WVU firmware and asks first.</p>

  {#if toolsState === 'failed' && toolsWarningShown}<p class="tools-warning"><strong>Arduino tools need attention.</strong> {toolsProblem}</p>{/if}

  {#if lastOperation}
    <section class="last-operation {lastOperation.outcome}" aria-labelledby="last-firmware-operation-title">
      <h4 id="last-firmware-operation-title">Last firmware operation</h4>
      <p class="operation-title">
        <span class="op-symbol" aria-hidden="true">{lastOperation.outcome === 'success' ? '✓' : '✗'}</span>
        <strong>{lastOperation.kind === 'restore' ? 'Restore' : 'Verify'} {lastOperation.outcome === 'success' ? 'succeeded' : 'failed'}:</strong>
        {lastOperation.title}
      </p>
      <p>{lastOperation.explanation}</p>
      {#if lastOperation.recommendedAction}<p><strong>What to do:</strong> {lastOperation.recommendedAction}</p>{/if}
      {#if lastOperation.outcome === 'failure' && firmwareReady && selectedPort}<p class="help">The board has since been verified on {selectedPort}.</p>{/if}
      <dl class="operation-facts">
        {#if lastOperation.finalPort}<div><dt>Final port</dt><dd>{lastOperation.finalPort}</dd></div>{/if}
        {#if lastOperation.finishedAt}<div><dt>Finished</dt><dd>{new Date(lastOperation.finishedAt).toLocaleString()}</dd></div>{/if}
        {#if lastOperation.logPath}<div class="log"><dt>Log file</dt><dd class="path">{lastOperation.logPath}</dd></div>{/if}
      </dl>
      <div class="operation-actions">
        <CopyButton text={lastOperation.diagnostics} label="Copy diagnostics" copiedMessage="Firmware diagnostics copied. Paste them into a message to your instructor." multiline onCopied={onFeedback} />
      </div>
    </section>
  {/if}
</div>

<style>
  .board-panel { min-width: 0; }
  .board-row { display: flex; flex-wrap: wrap; align-items: flex-end; gap: .55rem .75rem; min-width: 0; margin-top: .4rem; }
  .board-select { display: grid; flex: 1 1 16rem; gap: .3rem; min-width: 0; margin: 0; font-weight: 600; }
  button {
    min-height: 2.5rem;
    padding: .5rem .8rem;
    border: 1px solid var(--wvu-navy, #002855);
    border-radius: .3rem;
    background: var(--surface, #fff);
    color: var(--wvu-navy, #002855);
    font: inherit;
    font-weight: 650;
    cursor: pointer;
  }
  button:not(:disabled):hover { background: var(--wvu-navy, #002855); color: #fff; }
  button:disabled { cursor: not-allowed; opacity: .55; }
  .primary-action, .refresh.primary { background: var(--wvu-navy, #002855); color: #fff; }
  .primary-action:not(:disabled):hover, .refresh.primary:not(:disabled):hover { background: #001b3a; }
  .primary-action.gold { background: var(--wvu-gold, #EEAA00); color: var(--ink, #17222e); }
  .primary-action.gold:not(:disabled):hover { background: #d99a00; color: var(--ink, #17222e); }
  .board-chip {
    display: inline-flex;
    align-items: center;
    gap: .35rem;
    min-height: 2.5rem;
    padding: .3rem .75rem;
    border: 2px solid;
    border-radius: 999px;
    font-weight: 800;
    white-space: nowrap;
  }
  .board-chip.ready { border-color: var(--ok, #176c33); color: var(--ok, #176c33); background: var(--ok-bg, #eaf6ee); }
  .board-chip.idle { border-color: var(--border-strong, #8493a0); color: var(--muted, #4b5965); background: var(--surface, #fff); }
  .board-chip.busy { border-color: var(--wvu-navy, #002855); color: var(--wvu-navy, #002855); background: var(--surface-muted, #f2f5f8); }
  .board-chip.problem { border-color: var(--danger, #9d2424); color: var(--danger, #9d2424); background: var(--danger-bg, #fdf0f0); }
  .chip-symbol { font-weight: 900; }
  .board-hint { margin: .5rem 0 0; color: var(--ink, #17222e); }
  .more-actions { display: flex; flex-wrap: wrap; align-items: center; gap: .45rem .6rem; margin-top: .6rem; }
  .more-actions button { min-height: 2.25rem; padding: .35rem .7rem; }
  .more-label { color: var(--muted, #4b5965); font-size: .8rem; font-weight: 700; text-transform: uppercase; letter-spacing: .03em; }
  .help { margin: .45rem 0 0; color: var(--muted, #4b5965); }
  .tools-warning { margin: .6rem 0 0; padding: .6rem .75rem; border-left: 5px solid var(--warn, #9b6700); background: var(--warn-bg, #fff8e8); }
  .last-operation {
    min-width: 0;
    margin-top: .8rem;
    padding: .7rem .8rem;
    border: 1px solid var(--border, #d7dde2);
    border-left: 5px solid var(--ok, #176c33);
    border-radius: .3rem;
    background: #f9fbfc;
  }
  .last-operation.failure { border-left-color: var(--danger, #9d2424); }
  .last-operation h4 { margin: 0 0 .35rem; color: var(--muted, #4b5965); font-size: .8rem; text-transform: uppercase; letter-spacing: .03em; }
  .last-operation p { margin: .3rem 0; overflow-wrap: anywhere; }
  .op-symbol { font-weight: 900; }
  .success .op-symbol { color: var(--ok, #176c33); }
  .failure .op-symbol { color: var(--danger, #9d2424); }
  .operation-facts { display: flex; flex-wrap: wrap; gap: .3rem 1.2rem; margin: .4rem 0 0; }
  .operation-facts div { min-width: 0; }
  .operation-facts .log { flex: 1 1 100%; }
  .operation-facts dt { color: var(--muted, #4b5965); font-size: .78rem; font-weight: 700; text-transform: uppercase; letter-spacing: .03em; }
  .operation-facts dd { margin: 0; overflow-wrap: anywhere; }
  .path { font-family: ui-monospace, Consolas, monospace; font-size: .88rem; user-select: text; }
  .operation-actions { display: flex; flex-wrap: wrap; gap: .5rem; margin-top: .55rem; }
</style>
