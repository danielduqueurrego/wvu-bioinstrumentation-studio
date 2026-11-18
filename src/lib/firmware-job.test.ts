import { describe, expect, it } from 'vitest';
import {
  DOUBLE_RESET_SECONDS,
  doubleResetSecondsLeft,
  FIRMWARE_STEPS,
  FIRMWARE_WRITE_NOTICE,
  firmwareJobCancelable,
  firmwareJobStageLabel,
  firmwareJobTitle,
  firmwareJobView,
  firmwareStepAnnouncement,
  firmwareStepIndex,
  firmwareStepper
} from './firmware-job';

// Every stage the Rust `FirmwareJobStage` enum serializes.
const BACKEND_STAGES = [
  'idle', 'preparing', 'closing_serial_session', 'compiling', 'touch_reset',
  'waiting_for_bootloader', 'uploading', 'waiting_for_application_port',
  'verifying_firmware', 'complete', 'failed', 'canceled'
];

describe('firmware job labels', () => {
  it('never shows the raw job kind as the modal title', () => {
    expect(firmwareJobTitle({ kind: 'restore_reference' })).toBe('Restoring WVU Firmware…');
    expect(firmwareJobTitle({ kind: 'some_future_kind' })).toBe('Firmware operation in progress…');
    expect(firmwareJobTitle(undefined)).toBe('Firmware operation in progress…');
  });

  it('gives every backend stage a plain label', () => {
    for (const stage of BACKEND_STAGES) {
      const label = firmwareJobStageLabel(stage);
      expect(label).not.toContain('_');
      expect(label).not.toBe('Working');
    }
    expect(firmwareJobStageLabel('unknown_stage')).toBe('Working');
  });

  it('offers Cancel only before the board is written', () => {
    for (const stage of ['preparing', 'closing_serial_session', 'compiling', 'touch_reset', 'waiting_for_bootloader']) {
      expect(firmwareJobCancelable(stage)).toBe(true);
    }
    for (const stage of ['uploading', 'waiting_for_application_port', 'verifying_firmware']) {
      expect(firmwareJobCancelable(stage)).toBe(false);
    }
    expect(FIRMWARE_WRITE_NOTICE).toBe('Writing firmware — do not unplug the board');
  });

  it('maps every backend stage onto the six restore steps in order', () => {
    expect(FIRMWARE_STEPS.map((step) => step.label)).toEqual([
      'Prepare', 'Compile', 'Switch board to upload mode', 'Upload', 'Reconnect', 'Verify'
    ]);
    const indices = BACKEND_STAGES.map((stage) => firmwareStepIndex(stage));
    expect(indices).toEqual([0, 0, 0, 1, 2, 2, 3, 4, 5, 6, undefined, undefined]);
    // The order never goes backwards along a restore.
    const running = indices.slice(0, 9) as number[];
    expect([...running].sort((a, b) => a - b)).toEqual(running);
  });

  it('marks finished, current, and later steps', () => {
    const steps = firmwareStepper({ stage: 'uploading' });
    expect(steps.map((step) => step.status)).toEqual(['done', 'done', 'done', 'current', 'pending', 'pending']);
    expect(firmwareStepper({ stage: 'complete' }).every((step) => step.status === 'done')).toBe(true);
    expect(firmwareStepper({ stage: 'some_future_stage' }).every((step) => step.status === 'pending')).toBe(true);
  });

  it('marks the step a failed or canceled restore stopped in', () => {
    const failed = firmwareStepper({ stage: 'failed', failure: { stage: 'waiting_for_bootloader' } });
    expect(failed.map((step) => step.status)).toEqual(['done', 'done', 'failed', 'pending', 'pending', 'pending']);
    const canceled = firmwareStepper({ stage: 'canceled', failure: { stage: 'compiling' } });
    expect(canceled[1].status).toBe('failed');
  });

  it('announces a step as words, never as a stage code', () => {
    expect(firmwareStepAnnouncement({ stage: 'waiting_for_application_port' })).toBe('Restoring WVU Firmware, step 5 of 6: Reconnect.');
    expect(firmwareStepAnnouncement({ stage: 'complete' })).toBe('');
  });

  it('counts down the double-reset wait only while the backend waits for it', () => {
    const now = Date.parse('2026-11-18T10:00:00Z');
    const waiting = { active: true, stage: 'waiting_for_bootloader', manual_reset_deadline_utc: '2026-11-18T10:01:00Z' };
    expect(doubleResetSecondsLeft(waiting, now)).toBe(60);
    expect(doubleResetSecondsLeft(waiting, now + 59_500)).toBe(1);
    expect(doubleResetSecondsLeft(waiting, now + 120_000)).toBe(0);
    // A clock far behind the backend never shows more than the real wait.
    expect(doubleResetSecondsLeft(waiting, now - 600_000)).toBe(DOUBLE_RESET_SECONDS);
    // The CLI's own bootloader transition (no deadline) is not a double reset.
    expect(doubleResetSecondsLeft({ ...waiting, manual_reset_deadline_utc: null }, now)).toBeUndefined();
    expect(doubleResetSecondsLeft({ ...waiting, stage: 'uploading' }, now)).toBeUndefined();
    expect(doubleResetSecondsLeft({ ...waiting, active: false }, now)).toBeUndefined();
  });

  it('uses the backend message as detail and falls back to the step label', () => {
    const uploading = firmwareJobView({ kind: 'restore_reference', stage: 'uploading', message: '' });
    expect(uploading).toEqual({
      title: 'Restoring WVU Firmware…',
      step: 'Writing the firmware to the board',
      detail: 'Writing the firmware to the board…',
      cancelable: false,
      cancelLabel: 'Cancel restore'
    });
    const compiling = firmwareJobView({ kind: 'restore_reference', stage: 'compiling', message: 'Compiling the controlled WVU reference firmware.' });
    expect(compiling.detail).toBe('Compiling the controlled WVU reference firmware.');
    expect(compiling.cancelable).toBe(true);
    expect(compiling.cancelLabel).not.toContain('restore_reference');
  });
});
