import { describe, expect, it } from 'vitest';
import { boardStatus, lastFirmwareOperation, BOARD_ACTION_LABELS, type BoardStatusInput } from './board-panel';
import type { FirmwareFailure, FirmwareJob } from './session-types';

const base: BoardStatusInput = {
  selectedPort: 'COM5',
  compatibility: 'unknown',
  firmwareReady: false,
  checking: false,
  firmwareJobActive: false,
  lastFailureCategory: null
};

const failure = (category: string, overrides: Partial<FirmwareFailure> = {}): FirmwareFailure => ({
  category,
  stage: 'waiting_for_bootloader',
  title: 'Backend title',
  explanation: 'The board did not switch to its upload interface in time.',
  recommended_action: 'Backend action.',
  technical_details: 'native USB reset window expired',
  ...overrides
});

const job = (overrides: Partial<FirmwareJob> = {}): FirmwareJob => ({
  id: 12,
  kind: 'restore_reference',
  stage: 'complete',
  active: false,
  message: 'Done',
  original_port: 'COM34',
  final_port: 'COM35',
  log_path: 'C:\\Users\\student\\AppData\\Local\\WVU\\logs\\firmware_job_12.json',
  completed_utc: '2026-09-30T15:18:55Z',
  failure: null,
  ...overrides
});

describe('Board panel state chip and fitting action', () => {
  it('names each state in words and picks one fitting action', () => {
    expect(boardStatus({ ...base, selectedPort: '' })).toMatchObject({ state: 'none', label: 'No board selected', action: 'refresh' });
    expect(boardStatus({ ...base, checking: true })).toMatchObject({ state: 'checking', label: 'Checking…' });
    expect(boardStatus({ ...base, checking: true }).action).toBeUndefined();
    expect(boardStatus(base)).toMatchObject({ state: 'not_checked', label: 'Not checked', action: 'verify' });
    const ready = boardStatus({ ...base, compatibility: 'wvu_protocol_compatible', firmwareReady: true });
    expect(ready).toMatchObject({ state: 'ready', label: 'Ready on COM5', tone: 'ready' });
    expect(ready.action).toBeUndefined();
    expect(boardStatus({ ...base, lastFailureCategory: 'port_busy' })).toMatchObject({ state: 'busy', label: 'Port busy', action: 'verify_after_close' });
    expect(boardStatus({ ...base, compatibility: 'wvu_protocol_incompatible' })).toMatchObject({ state: 'no_firmware', label: 'No WVU firmware', action: 'restore' });
    expect(boardStatus({ ...base, compatibility: 'verification_failed', lastFailureCategory: 'protocol_verification_failed' })).toMatchObject({ state: 'no_firmware', action: 'restore' });
    expect(BOARD_ACTION_LABELS.verify_after_close).toBe('Close other serial apps, then Verify');
  });

  it('shows a running restore instead of any action, even when a board is selected', () => {
    const restoring = boardStatus({ ...base, firmwareJobActive: true });
    expect(restoring.state).toBe('restoring');
    expect(restoring.action).toBeUndefined();
    expect(boardStatus({ ...base, compatibility: 'upload_in_progress' })).toMatchObject({ state: 'restoring' });
  });

  it('says which side is out of date when the board has another WVU firmware build', () => {
    const older = boardStatus({ ...base, compatibility: 'verification_failed', lastFailureCategory: 'older_wvu_firmware' });
    expect(older).toMatchObject({ state: 'older_firmware', label: 'Older WVU firmware', tone: 'problem', action: 'restore' });
    expect(older.hint).toContain('Restore the WVU firmware to update it');
    const newer = boardStatus({ ...base, compatibility: 'verification_failed', lastFailureCategory: 'newer_wvu_firmware' });
    expect(newer).toMatchObject({ state: 'newer_firmware', label: 'Newer WVU firmware', tone: 'problem', action: 'restore' });
    expect(newer.hint).toContain('Update the app on this computer');
    expect(newer.hint).toContain('COM5');
    // A Retry connection or Start that proved it withdraws readiness without a Verify failure.
    expect(boardStatus({ ...base, compatibility: 'verification_failed', connectionFailureCategory: 'newer_wvu_firmware' }))
      .toMatchObject({ state: 'newer_firmware' });
    expect(boardStatus({ ...base, compatibility: 'verification_failed', lastFailureCategory: 'protocol_verification_failed', connectionFailureCategory: 'older_wvu_firmware' }))
      .toMatchObject({ state: 'older_firmware' });
    // Ready firmware and a busy port still come first; a connection result alone does not override "not checked".
    expect(boardStatus({ ...base, compatibility: 'wvu_protocol_compatible', firmwareReady: true, connectionFailureCategory: 'newer_wvu_firmware' }).state).toBe('ready');
    expect(boardStatus({ ...base, lastFailureCategory: 'port_busy', connectionFailureCategory: 'older_wvu_firmware' }).state).toBe('busy');
    expect(boardStatus({ ...base, connectionFailureCategory: 'older_wvu_firmware' }).state).toBe('not_checked');
  });

  it('treats firmware verified on another port as not checked on this one', () => {
    expect(boardStatus({ ...base, compatibility: 'wvu_protocol_compatible', firmwareReady: false })).toMatchObject({ state: 'not_checked', action: 'verify' });
  });

  it('follows the plain-language mapping after a failed restore', () => {
    expect(boardStatus({ ...base, compatibility: 'verification_failed', lastFailureCategory: 'manual_reset_required' })).toMatchObject({ state: 'needs_attention', action: 'restore' });
    expect(boardStatus({ ...base, compatibility: 'verification_failed', lastFailureCategory: 'application_port_not_found' })).toMatchObject({ action: 'refresh' });
    expect(boardStatus({ ...base, compatibility: 'verification_failed', lastFailureCategory: 'compile_failed' })).toMatchObject({ action: 'repair_tools' });
    const hint = boardStatus({ ...base, compatibility: 'verification_failed', lastFailureCategory: 'upload_failed' }).hint;
    expect(hint).toContain('The firmware upload failed');
    expect(hint).not.toContain('upload_failed');
  });
});

describe('Last firmware operation card', () => {
  it('is empty before any firmware operation and while a restore runs', () => {
    expect(lastFirmwareOperation({})).toBeUndefined();
    expect(lastFirmwareOperation({ last_restore: job({ active: true, stage: 'uploading' }) })).toBeUndefined();
  });

  it('reports a completed restore with the final port, log, and a COM change explained', () => {
    const operation = lastFirmwareOperation({ last_restore: job(), last_failure: null })!;
    expect(operation).toMatchObject({ kind: 'restore', outcome: 'success', title: 'WVU Firmware restored', finalPort: 'COM35', logPath: job().log_path });
    expect(operation.explanation).toContain('verified on COM35');
    expect(operation.explanation).toContain('moved from COM34 to COM35');
    expect(operation.diagnostics).toContain('Final port: COM35');
    expect(operation.diagnostics).toContain('Log: C:\\Users\\student');
  });

  it('explains a failed restore in plain language and keeps the raw codes for diagnostics', () => {
    const restoreFailure = failure('manual_reset_required');
    const operation = lastFirmwareOperation({
      last_restore: job({ stage: 'failed', final_port: null, failure: restoreFailure }),
      last_failure: restoreFailure
    })!;
    expect(operation).toMatchObject({ kind: 'restore', outcome: 'failure', title: 'The board needs a double reset' });
    expect(operation.explanation).toBe(restoreFailure.explanation);
    expect(operation.recommendedAction).toContain('RESET button twice quickly');
    expect(operation.diagnostics).toContain('Category: manual_reset_required');
    expect(operation.diagnostics).toContain('Technical details: native USB reset window expired');
    expect(operation.finalPort).toBeUndefined();
  });

  it('shows a later Verify failure instead of an older restore', () => {
    const busy = failure('port_busy', { stage: 'verifying_firmware', explanation: 'COM35 is open in another program.' });
    const afterSuccess = lastFirmwareOperation({ last_restore: job(), last_failure: busy })!;
    expect(afterSuccess).toMatchObject({ kind: 'verify', outcome: 'failure', title: 'Another program is using the board' });
    const afterFailedRestore = lastFirmwareOperation({
      last_restore: job({ stage: 'failed', failure: failure('upload_failed') }),
      last_failure: busy
    })!;
    expect(afterFailedRestore.kind).toBe('verify');
  });

  it('reports a canceled restore without a failure record', () => {
    const operation = lastFirmwareOperation({ last_restore: job({ stage: 'canceled', failure: null }) })!;
    expect(operation).toMatchObject({ outcome: 'failure', title: 'The firmware restore was canceled' });
    expect(operation.diagnostics).toContain('Result: canceled');
  });
});
