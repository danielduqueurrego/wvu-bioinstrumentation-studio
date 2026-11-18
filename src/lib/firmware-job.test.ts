import { describe, expect, it } from 'vitest';
import {
  FIRMWARE_WRITE_NOTICE,
  firmwareJobCancelable,
  firmwareJobStageLabel,
  firmwareJobTitle,
  firmwareJobView
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
