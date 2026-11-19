import { describe, expect, it } from 'vitest';
import { CHECKLIST_TARGETS, firstFailingItem, readinessChecklist, type ChecklistInput } from './readiness';
import { recordingStartReadiness, type RecordingStartReadinessInput } from './recording-start';

const ready: ChecklistInput = {
  source: 'hardware',
  selectedPort: 'COM5',
  firmwareReady: true,
  blockingFault: false,
  labName: 'ECG — Course Capture',
  acknowledgementSatisfied: true,
  projectFolder: 'C:\\BMEG 420L'
};

const byId = (input: ChecklistInput) => Object.fromEntries(readinessChecklist(input).map((item) => [item.id, item]));

describe('readiness checklist', () => {
  it('lists Board · Firmware · Lab · Folder · Source in that order', () => {
    expect(readinessChecklist(ready).map((item) => item.label)).toEqual(['Board', 'Firmware', 'Lab', 'Folder', 'Source']);
    expect(readinessChecklist(ready).every((item) => item.status === 'ok')).toBe(true);
    expect(byId(ready).source.detail).toBe('Hardware · COM5');
  });

  it('skips Board and Firmware for the Simulator', () => {
    const items = byId({ ...ready, source: 'simulator', selectedPort: '', firmwareReady: false });
    expect(items.board.status).toBe('skip');
    expect(items.firmware.status).toBe('skip');
    expect(items.source.detail).toBe('Simulator (practice)');
  });

  it('points a missing board and unverified firmware at the board controls', () => {
    expect(byId({ ...ready, selectedPort: '' }).board).toMatchObject({ status: 'fail', target: 'board-select' });
    expect(byId({ ...ready, selectedPort: '' }).firmware).toMatchObject({ status: 'fail', target: 'board-select' });
    expect(byId({ ...ready, firmwareReady: false }).firmware).toMatchObject({ status: 'fail', target: 'verify-firmware' });
  });

  it('names the fix for an older or newer WVU firmware build', () => {
    expect(byId({ ...ready, firmwareReady: false, firmwareMismatch: 'older' }).firmware)
      .toMatchObject({ status: 'fail', detail: 'restore the WVU firmware', target: 'verify-firmware' });
    expect(byId({ ...ready, firmwareReady: false, firmwareMismatch: 'newer' }).firmware)
      .toMatchObject({ status: 'fail', detail: 'update this app or restore', target: 'verify-firmware' });
    // Ready firmware wins over a stale mismatch.
    expect(byId({ ...ready, firmwareMismatch: 'newer' }).firmware.status).toBe('ok');
  });

  it('points each lab-setting problem at its own control', () => {
    expect(byId({ ...ready, labName: undefined }).lab.target).toBe('lab-select');
    expect(byId({ ...ready, durationError: 'too short' }).lab.target).toBe('duration');
    expect(byId({ ...ready, adcReferenceError: 'bad' }).lab.target).toBe('adc-reference');
    expect(byId({ ...ready, mpxvSupplyError: 'bad' }).lab.target).toBe('mpxv-supply');
    expect(byId({ ...ready, acknowledgementSatisfied: false }).lab.target).toBe('bench-acknowledgement');
  });

  it('points folder problems at Browse or the Output folder field', () => {
    expect(byId({ ...ready, projectFolder: '' }).folder.target).toBe('project-folder');
    expect(byId({ ...ready, outputFolderError: 'bad name' }).folder.target).toBe('output-folder');
  });

  it('names the button that clears a fault blocking a hardware Start, and focuses that button', () => {
    expect(byId({ ...ready, blockingFault: true, faultAction: 'restore_firmware' }).source)
      .toMatchObject({ status: 'fail', detail: 'select Restore WVU Firmware first', target: 'message-action' });
    expect(byId({ ...ready, blockingFault: true, faultAction: 'verify_firmware' }).source.detail).toBe('select Verify Firmware first');
    // No banner action: the recording bar’s Clear connection.
    expect(byId({ ...ready, blockingFault: true }).source).toMatchObject({ detail: 'select Clear connection first', target: 'clear-connection' });
    expect(byId({ ...ready, source: 'simulator', blockingFault: true }).source.status).toBe('ok');
  });

  it('gives every target an element and a card when it lives in one', () => {
    for (const [target, location] of Object.entries(CHECKLIST_TARGETS)) {
      expect(location.elementIds.length, target).toBeGreaterThan(0);
    }
    expect(CHECKLIST_TARGETS['message-action'].card).toBeUndefined();
  });

  it('explains every setup reason that blocks Start', () => {
    const base: RecordingStartReadinessInput = {
      source: 'hardware', selectedBoard: true, firmwareReady: true, sessionState: 'Disconnected', boardOperationBusy: false,
      startInFlight: false, activeProfile: true, projectFolder: true, durationValid: true, acknowledgementSatisfied: true
    };
    const cases: Array<[Partial<RecordingStartReadinessInput>, Partial<ChecklistInput>]> = [
      [{ selectedBoard: false }, { selectedPort: '' }],
      [{ firmwareReady: false }, { firmwareReady: false }],
      [{ activeProfile: false }, { labName: undefined }],
      [{ projectFolder: false }, { projectFolder: '' }],
      [{ outputFolderError: 'bad' }, { outputFolderError: 'bad' }],
      [{ durationValid: false }, { durationError: 'bad' }],
      [{ calibrationInputError: 'bad' }, { adcReferenceError: 'bad' }],
      [{ acknowledgementSatisfied: false }, { acknowledgementSatisfied: false }],
      [{ sessionState: 'Faulted', faultOrigin: 'recording' }, { blockingFault: true }]
    ];
    for (const [blocked, checklist] of cases) {
      expect(recordingStartReadiness({ ...base, ...blocked }).canStart).toBe(false);
      expect(firstFailingItem(readinessChecklist({ ...ready, ...checklist })), JSON.stringify(blocked)).toBeDefined();
    }
  });
});
