// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import BoardPanel from './BoardPanel.svelte';
import { boardStatus, lastFirmwareOperation, type BoardStatusInput } from '$lib/board-panel';
import type { FirmwareWorkflowStatus } from '$lib/session-types';

type PanelState = {
  status: Partial<BoardStatusInput>;
  controls?: { canSelectBoard: boolean; canRefreshBoards: boolean; canVerifyFirmware: boolean; canRestoreFirmware: boolean };
  workflow?: Pick<FirmwareWorkflowStatus, 'last_restore' | 'last_failure'>;
};

const idleControls = { canSelectBoard: true, canRefreshBoards: true, canVerifyFirmware: true, canRestoreFirmware: true };

function render(initial: PanelState) {
  const calls = { refresh: 0, verify: 0, restore: 0, repair: 0, select: 0, feedback: [] as string[] };
  const state = $state<PanelState>(initial);
  let port = $state('COM5');
  const component = mount(BoardPanel, {
    target: document.body,
    props: {
      boards: [{ port: 'COM5', name: 'Arduino UNO R4 WiFi', fqbn: 'arduino:renesas_uno:unor4wifi' }],
      get selectedPort() { return port; },
      set selectedPort(value: string) { port = value; },
      get status() {
        return boardStatus({ selectedPort: port, compatibility: 'unknown', firmwareReady: false, checking: false, firmwareJobActive: false, ...state.status });
      },
      get controls() { return state.controls ?? idleControls; },
      toolsState: 'ready' as const,
      toolsProblem: '',
      toolsRepairable: true,
      canRepairTools: true,
      get lastOperation() { return state.workflow ? lastFirmwareOperation(state.workflow) : undefined; },
      get firmwareReady() { return state.status.firmwareReady === true; },
      onSelectChange: () => { calls.select += 1; },
      onRefresh: () => { calls.refresh += 1; },
      onVerify: () => { calls.verify += 1; },
      onRestore: () => { calls.restore += 1; },
      onRepairTools: () => { calls.repair += 1; },
      onFeedback: (message: string) => { calls.feedback.push(message); }
    }
  });
  flushSync();
  return { component, calls, state };
}

const buttons = () => [...document.querySelectorAll<HTMLButtonElement>('button')];
const named = (text: string) => buttons().filter((button) => button.textContent?.trim() === text);

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

describe('BoardPanel', () => {
  it('offers Verify as the fitting action for an unchecked board, with Restore still reachable', () => {
    const panel = render({ status: {} });
    expect(document.querySelector('[data-testid="board-state"]')?.textContent).toContain('Not checked');
    expect(named('Verify Firmware')).toHaveLength(1);
    expect(named('Verify Firmware')[0].id).toBe('verify-firmware-button');
    expect(named('Restore WVU Firmware')).toHaveLength(1);
    expect(named('Restore WVU Firmware')[0].closest('[role="group"]')?.getAttribute('aria-labelledby')).toBe('more-board-actions-label');
    named('Verify Firmware')[0].click();
    named('Restore WVU Firmware')[0].click();
    expect(panel.calls).toMatchObject({ verify: 1, restore: 1 });
    unmount(panel.component);
  });

  it('makes a gold Restore the fitting action when the board has no WVU firmware', () => {
    const panel = render({ status: { compatibility: 'wvu_protocol_incompatible' } });
    expect(document.querySelector('[data-testid="board-state"]')?.textContent).toContain('No WVU firmware');
    const restore = named('Restore WVU Firmware');
    expect(restore).toHaveLength(1);
    expect(restore[0].id).toBe('board-primary-action');
    expect(restore[0].classList.contains('gold')).toBe(true);
    // Verify is still there, under More board actions.
    expect(named('Verify Firmware')[0].id).toBe('verify-firmware-button');
    unmount(panel.component);
  });

  it('asks to close other serial apps when the port is busy', () => {
    const panel = render({ status: { lastFailureCategory: 'port_busy' } });
    expect(document.querySelector('[data-testid="board-state"]')?.textContent).toContain('Port busy');
    const action = named('Close other serial apps, then Verify')[0];
    expect(action.id).toBe('verify-firmware-button');
    action.click();
    expect(panel.calls.verify).toBe(1);
    expect(named('Verify Firmware')).toHaveLength(0);
    unmount(panel.component);
  });

  it('keeps Verify and Restore reachable when the firmware is ready', () => {
    const panel = render({ status: { compatibility: 'wvu_protocol_compatible', firmwareReady: true } });
    expect(document.querySelector('[data-testid="board-state"]')?.textContent).toContain('Ready on COM5');
    expect(named('Verify Firmware')[0].disabled).toBe(false);
    expect(named('Restore WVU Firmware')[0].disabled).toBe(false);
    unmount(panel.component);
  });

  it('highlights Refresh Board when no board is selected and keeps recovery buttons visible', () => {
    const panel = render({ status: {}, controls: { ...idleControls, canVerifyFirmware: false, canRestoreFirmware: false } });
    const select = document.querySelector<HTMLSelectElement>('#board-select')!;
    select.value = '';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();
    expect(panel.calls.select).toBe(1);
    expect(document.querySelector('[data-testid="board-state"]')?.textContent).toContain('No board selected');
    expect(named('Refresh Board')[0].classList.contains('primary')).toBe(true);
    expect(named('Verify Firmware')).toHaveLength(1);
    expect(named('Restore WVU Firmware')).toHaveLength(1);
    unmount(panel.component);
  });

  it('shows the last firmware operation and copies its diagnostics', async () => {
    const writeText = vi.fn(async () => undefined);
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const panel = render({
      status: { compatibility: 'verification_failed', lastFailureCategory: 'manual_reset_required' },
      workflow: {
        last_restore: {
          id: 4, kind: 'restore_reference', stage: 'failed', active: false, message: 'Restore failed',
          original_port: 'COM5', final_port: null, log_path: 'C:\\logs\\firmware_job_4.json',
          failure: {
            category: 'manual_reset_required', stage: 'waiting_for_bootloader', title: 'Manual reset required',
            explanation: 'The board did not switch to its upload interface in time.', recommended_action: 'Press Reset twice.', technical_details: 'reset window expired'
          }
        },
        last_failure: null
      }
    });
    const card = document.querySelector('[aria-labelledby="last-firmware-operation-title"]')!;
    expect(card.textContent).toContain('Restore failed:');
    expect(card.textContent).toContain('The board needs a double reset');
    expect(card.textContent).toContain('C:\\logs\\firmware_job_4.json');
    expect(card.textContent).not.toContain('manual_reset_required');
    named('Copy diagnostics')[0].click();
    await new Promise((done) => setTimeout(done, 0));
    flushSync();
    expect(writeText).toHaveBeenCalledWith(expect.stringContaining('Category: manual_reset_required'));
    expect(panel.calls.feedback[0]).toContain('diagnostics copied');
    unmount(panel.component);
  });

  it('shows the diagnostics selected for Ctrl+C when the clipboard is unavailable', async () => {
    vi.stubGlobal('navigator', {});
    const panel = render({
      status: {},
      workflow: { last_failure: { category: 'port_busy', stage: 'verifying_firmware', title: 'Port busy', explanation: 'COM5 is open elsewhere.', recommended_action: '', technical_details: 'access denied' } }
    });
    named('Copy diagnostics')[0].click();
    await new Promise((done) => setTimeout(done, 0));
    flushSync();
    await new Promise((done) => setTimeout(done, 0));
    const fallback = document.querySelector<HTMLTextAreaElement>('.copy-fallback textarea');
    expect(fallback?.value).toContain('Technical details: access denied');
    expect(document.activeElement).toBe(fallback);
    expect(document.querySelector(`label[for="${fallback?.id}"]`)?.textContent).toBe('Press Ctrl+C to copy');
    expect(panel.calls.feedback[0]).toContain('Ctrl+C');
    unmount(panel.component);
  });
});
