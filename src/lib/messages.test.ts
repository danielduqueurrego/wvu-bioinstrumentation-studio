import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  CONNECTION_FAILURE_MESSAGES,
  FAULT_ORIGIN_MESSAGES,
  FIRMWARE_FAILURE_MESSAGES,
  NEXT_ACTION_LABELS,
  VERIFY_INSTEAD_OF_RETRY,
  connectionFailureMessage,
  faultBanner,
  faultConcernsSource,
  firmwareFailureMessage,
  problemBanner,
  SIMULATOR_PORT,
  simulatorRecordingFault,
  STORAGE_REFUSED_BEFORE_START,
  type ProblemInput
} from './messages';

/** The snake_case serde names of a Rust enum's unit variants. */
function rustEnumVariants(file: string, enumName: string): string[] {
  const source = readFileSync(resolve(process.cwd(), 'src-tauri', 'src', file), 'utf8');
  const start = source.indexOf(`pub enum ${enumName} {`);
  if (start < 0) throw new Error(`${enumName} not found in ${file}`);
  const body = source.slice(start, source.indexOf('\n}', start));
  return body
    .split('\n')
    .slice(1)
    .map((line) => line.trim())
    .filter((line) => /^[A-Z][A-Za-z]*,$/.test(line))
    .map((line) => line.slice(0, -1).replace(/([a-z])([A-Z])/g, '$1_$2').toLowerCase());
}

const RAW_CODE = /[a-z]+_[a-z_]+/;

describe('plain-language mapping covers every backend code', () => {
  it.each([
    ['session.rs', 'ConnectionFailureCategory', CONNECTION_FAILURE_MESSAGES],
    ['session.rs', 'FaultOrigin', FAULT_ORIGIN_MESSAGES],
    ['firmware_workflow.rs', 'FirmwareErrorCategory', FIRMWARE_FAILURE_MESSAGES]
  ])('%s %s', (file, enumName, table) => {
    const variants = rustEnumVariants(file, enumName);
    expect(variants.length).toBeGreaterThan(3);
    expect(Object.keys(table).sort()).toEqual([...variants].sort());
  });

  it('gives every entry a headline, a full sentence, and no raw codes', () => {
    for (const table of [CONNECTION_FAILURE_MESSAGES, FAULT_ORIGIN_MESSAGES, FIRMWARE_FAILURE_MESSAGES, { storage_refused_before_start: STORAGE_REFUSED_BEFORE_START }]) {
      for (const [code, message] of Object.entries(table)) {
        expect(message.headline.trim(), code).not.toBe('');
        expect(message.sentence, code).toMatch(/[.!?]$/);
        expect(`${message.headline} ${message.sentence}`, code).not.toMatch(RAW_CODE);
        if (message.nextAction) expect(NEXT_ACTION_LABELS[message.nextAction], code).toBeTruthy();
      }
    }
  });

  it('never suggests the retired Reset board action', () => {
    for (const table of [CONNECTION_FAILURE_MESSAGES, FAULT_ORIGIN_MESSAGES, FIRMWARE_FAILURE_MESSAGES]) {
      for (const message of Object.values(table)) expect(message.sentence).not.toMatch(/Reset board/);
    }
  });

  it('returns undefined for an unknown code instead of throwing', () => {
    expect(connectionFailureMessage('something_new')).toBeUndefined();
    expect(firmwareFailureMessage(undefined)).toBeUndefined();
    expect(connectionFailureMessage('toString')).toBeUndefined();
  });
});

const fault = (overrides: Partial<Parameters<typeof faultBanner>[0]> = {}) =>
  faultBanner({ faultOrigin: 'recording', failureCategory: null, lastError: 'x', simulatorSession: false, recordingRan: true, source: 'hardware', ...overrides });

describe('fault banner next action', () => {
  it('clears a save problem instead of sending the student to firmware recovery', () => {
    const banner = fault({ faultOrigin: 'storage', failureCategory: 'recording_storage_error' });
    expect(banner.headline).toBe('The recording was not saved completely');
    expect(banner.nextAction).toBe('clear_connection');
    expect(banner.sentence).not.toMatch(/firmware/i);
  });

  it('says a recording the folder or drive refused before START could not start, and why', () => {
    for (const source of ['hardware', 'simulator'] as const) {
      for (const failureCategory of ['recording_storage_error', null]) {
        const banner = fault({
          faultOrigin: 'storage', failureCategory, recordingRan: false, source, simulatorSession: source === 'simulator',
          lastError: 'storage: only 200 MiB free in E:\\BMEG; recording requires at least 250 MiB'
        });
        expect(banner.headline).toBe('The recording could not start');
        expect(banner.sentence).toContain('Only 200 MiB free in E:\\BMEG; recording requires at least 250 MiB.');
        expect(banner.sentence).toContain('Nothing was recorded.');
        expect(banner.sentence).not.toMatch(/saved completely|already saved|firmware/i);
        expect(banner.nextAction).toBe('clear_connection');
      }
    }
    // Without a detail the plain sentence stands alone.
    expect(fault({ faultOrigin: 'storage', recordingRan: false, lastError: null }).sentence).toBe(STORAGE_REFUSED_BEFORE_START.sentence);
    // A save that failed after a recording keeps its own text and id.
    const saved = fault({ faultOrigin: 'storage', failureCategory: 'recording_storage_error', recordingRan: true });
    expect(saved.id).toBe('fault:storage:recording_storage_error');
    expect(fault({ faultOrigin: 'storage', failureCategory: 'recording_storage_error', recordingRan: false }).id).not.toBe(saved.id);
  });

  it('shows the backend’s student text for a refused request', () => {
    const banner = fault({ faultOrigin: 'request', lastError: 'This lab is too large for one recording header' });
    expect(banner.sentence).toContain('This lab is too large for one recording header.');
    expect(banner.nextAction).toBe('clear_connection');
  });

  it('restores the firmware when the board runs something else', () => {
    expect(fault({ faultOrigin: 'connection', failureCategory: 'non_protocol_bytes' }).nextAction).toBe('restore_firmware');
    expect(fault({ faultOrigin: 'verification', failureCategory: 'wrong_protocol_version' }).nextAction).toBe('restore_firmware');
  });

  it('refreshes the board list after an unplug, whatever raised the fault', () => {
    expect(fault({ faultOrigin: 'recording', failureCategory: 'device_disconnected' }).nextAction).toBe('refresh_board');
    expect(fault({ faultOrigin: 'verification', failureCategory: 'device_disconnected' }).nextAction).toBe('refresh_board');
  });

  it('repeats the failed operation otherwise', () => {
    expect(fault({ faultOrigin: 'verification', failureCategory: 'port_busy' }).nextAction).toBe('verify_firmware');
    expect(fault({ faultOrigin: 'connection', failureCategory: 'port_busy' }).nextAction).toBe('retry_connection');
    expect(fault({ faultOrigin: 'recording', failureCategory: 'no_data_timeout' }).nextAction).toBe('verify_firmware');
  });

  it('names a board that never answered Verify as having no WVU firmware, like the Board card', () => {
    for (const category of ['port_open_no_bytes', 'handshake_incomplete']) {
      const banner = fault({ faultOrigin: 'verification', failureCategory: category, port: 'COM5' });
      expect(banner.headline, category).toBe('No WVU firmware detected on COM5');
      expect(banner.nextAction).toBe('restore_firmware');
      expect(banner.secondary).toEqual({ action: 'verify_firmware', label: 'Verify again' });
      expect(banner.sentence).not.toMatch(/retry/i);
    }
    // The same silence during Retry connection is still a connection problem.
    expect(fault({ faultOrigin: 'connection', failureCategory: 'port_open_no_bytes' }).nextAction).toBe('retry_connection');
  });

  it('agrees with the Board card about every Verify that opened the board but got no WVU answer', () => {
    // firmware_workflow records these as protocol_verification_failed: the Board card says "No WVU firmware".
    for (const category of ['protocol_crc_failure', 'serial_transport_error', 'firmware_protocol_error', 'configuration_rejected']) {
      const banner = fault({ faultOrigin: 'verification', failureCategory: category, port: 'COM5' });
      expect(banner.headline, category).toBe('No WVU firmware detected on COM5');
      expect(banner.nextAction, category).toBe('restore_firmware');
      expect(banner.sentence, category).not.toMatch(/retry/i);
    }
    // Another WVU firmware version keeps its own text, with the same Restore step.
    const version = fault({ faultOrigin: 'verification', failureCategory: 'wrong_protocol_version', port: 'COM5' });
    expect(version.headline).toBe('The board has a different WVU firmware version');
    expect(version.nextAction).toBe('restore_firmware');
  });

  it('asks to verify again, not to retry the connection, when Verify found the port busy', () => {
    const banner = fault({ faultOrigin: 'verification', failureCategory: 'port_busy', port: 'COM5' });
    expect(banner.headline).toBe(FIRMWARE_FAILURE_MESSAGES.port_busy.headline);
    expect(banner.sentence).toBe(FIRMWARE_FAILURE_MESSAGES.port_busy.sentence);
    expect(banner.sentence).not.toMatch(/retry/i);
    expect(NEXT_ACTION_LABELS[banner.nextAction!]).toBe('Verify Firmware');
  });

  it('never tells the student to retry the connection beside a Verify Firmware button', () => {
    const retrying = Object.entries(CONNECTION_FAILURE_MESSAGES)
      .filter(([, message]) => message.nextAction === 'retry_connection')
      .map(([category]) => category);
    expect(retrying.length).toBeGreaterThan(3);
    for (const category of retrying) {
      expect(VERIFY_INSTEAD_OF_RETRY, category).toHaveProperty(category);
      for (const input of [
        { faultOrigin: 'recording', failureCategory: category },
        { faultOrigin: 'connection', failureCategory: category, source: 'simulator' as const },
        { faultOrigin: 'verification', failureCategory: category }
      ]) {
        const banner = fault(input);
        if (banner.nextAction !== 'verify_firmware') continue;
        expect(banner.sentence, JSON.stringify(input)).not.toMatch(/retry/i);
        expect(banner.sentence, JSON.stringify(input)).toMatch(/verify/i);
      }
    }
    // A connection fault while Source is Hardware still offers Retry connection.
    expect(fault({ faultOrigin: 'connection', failureCategory: 'protocol_crc_failure' })).toMatchObject({
      nextAction: 'retry_connection',
      sentence: CONNECTION_FAILURE_MESSAGES.protocol_crc_failure.sentence
    });
  });

  it('never offers Retry connection while Source is Simulator', () => {
    expect(fault({ faultOrigin: 'connection', failureCategory: 'handshake_incomplete', source: 'simulator' }).nextAction).toBe('verify_firmware');
  });

  it('clears a simulator fault', () => {
    const banner = fault({ simulatorSession: true, faultOrigin: 'recording' });
    expect(banner.headline).toMatch(/simulator/i);
    expect(banner.nextAction).toBe('clear_connection');
    expect(fault({ simulatorSession: true, faultOrigin: 'connection', source: 'simulator' }).headline).toBe('The simulator recording could not start');
  });

  it('describes a failed Verify after a Simulator recording as a board problem, never as the simulator recording', () => {
    for (const source of ['hardware', 'simulator'] as const) {
      const banner = fault({ simulatorSession: true, faultOrigin: 'verification', failureCategory: 'non_protocol_bytes', source, port: 'COM5' });
      expect(banner.headline).not.toMatch(/simulator/i);
      expect(banner.nextAction).toBe('restore_firmware');
      const silent = fault({ simulatorSession: true, faultOrigin: 'verification', failureCategory: 'port_open_no_bytes', source, port: 'COM5' });
      expect(silent.headline).toBe('No WVU firmware detected on COM5');
      const busy = fault({ simulatorSession: true, faultOrigin: 'verification', failureCategory: 'port_busy', source, port: 'COM5' });
      expect(busy.headline).toBe('Another program is using the board');
      expect(busy.nextAction).toBe('verify_firmware');
    }
  });

  it('names a fault a Simulator recording raised, not a later board check that kept the simulator flag', () => {
    expect(simulatorRecordingFault({ simulator: true, port: 'SIM', fault_origin: 'connection' })).toBe(true);
    expect(simulatorRecordingFault({ simulator: true, port: 'SIM', fault_origin: 'recording' })).toBe(true);
    // Verify, Retry connection, and a restore's verification set the board's COM port.
    expect(simulatorRecordingFault({ simulator: true, port: 'COM5', fault_origin: 'connection' })).toBe(false);
    expect(simulatorRecordingFault({ simulator: true, port: 'SIM', fault_origin: 'verification' })).toBe(false);
    expect(simulatorRecordingFault({ simulator: false, port: 'COM5', fault_origin: 'recording' })).toBe(false);
    // The backend's simulator session uses the same port name.
    const session = readFileSync(resolve(process.cwd(), 'src-tauri', 'src', 'session.rs'), 'utf8');
    expect(session).toMatch(new RegExp(`begin_session\\(\\s*true,\\s*"Simulator",\\s*"${SIMULATOR_PORT}",`));
  });

  it('uses the category text and keeps one id per problem', () => {
    const banner = fault({ faultOrigin: 'recording', failureCategory: 'no_data_timeout' });
    expect(banner.headline).toBe('The Arduino stopped sending samples');
    expect(banner.id).toBe('fault:recording:no_data_timeout');
  });
});

const quiet: ProblemInput = {
  sessionState: 'Disconnected',
  active: false,
  fault: { simulatorSession: false, recordingRan: false, source: 'simulator' },
  arduinoTools: { state: 'ready', problem: '', repairable: true },
  board: { source: 'simulator', selectedPort: '', firmwareReady: false, compatibility: 'unknown', firmwareJobActive: false }
};

describe('problem banner priority', () => {
  it('shows nothing when nothing is wrong', () => {
    expect(problemBanner(quiet)).toBeUndefined();
  });

  it('shows the fault first, even with other problems present', () => {
    const banner = problemBanner({
      ...quiet,
      sessionState: 'Faulted',
      fault: { faultOrigin: 'recording', failureCategory: 'no_data_timeout', simulatorSession: false, recordingRan: true, source: 'hardware' },
      arduinoTools: { state: 'failed', problem: 'Broken', repairable: true },
      catalogNotice: 'Local labs could not be loaded.'
    });
    expect(banner?.id).toBe('fault:recording:no_data_timeout');
  });

  it('offers Repair only for repairable Arduino tools, while Source is Hardware', () => {
    const hardware = { ...quiet.board, source: 'hardware' as const };
    expect(problemBanner({ ...quiet, board: hardware, arduinoTools: { state: 'failed', problem: 'Broken', repairable: true } })?.nextAction).toBe('repair_arduino_tools');
    expect(problemBanner({ ...quiet, board: hardware, arduinoTools: { state: 'failed', problem: 'Reinstall', repairable: false } })?.nextAction).toBeUndefined();
  });

  it('keeps Arduino-tools and board problems out of the banner during Simulator practice', () => {
    expect(problemBanner({ ...quiet, arduinoTools: { state: 'failed', problem: 'Arduino tools missing.', repairable: false } })).toBeUndefined();
    expect(problemBanner({ ...quiet, board: { ...quiet.board, selectedPort: 'COM5', compatibility: 'wvu_protocol_incompatible' } })).toBeUndefined();
    // A failed board check does not concern Simulator practice either …
    const verification = { faultOrigin: 'verification', failureCategory: 'port_open_no_bytes', simulatorSession: false, recordingRan: false, source: 'simulator' as const };
    expect(faultConcernsSource(verification)).toBe(false);
    expect(problemBanner({ ...quiet, sessionState: 'Faulted', fault: verification })).toBeUndefined();
    // … even right after a Simulator recording …
    expect(faultConcernsSource({ ...verification, simulatorSession: true, recordingRan: true })).toBe(false);
    expect(problemBanner({ ...quiet, sessionState: 'Faulted', fault: { ...verification, simulatorSession: true } })).toBeUndefined();
    // … but the student's own recording fault, or a simulator fault, does.
    expect(faultConcernsSource({ ...verification, faultOrigin: 'recording' })).toBe(true);
    expect(faultConcernsSource({ ...verification, faultOrigin: 'connection', simulatorSession: true })).toBe(true);
    expect(faultConcernsSource({ ...verification, faultOrigin: 'connection' })).toBe(false);
    expect(problemBanner({ ...quiet, sessionState: 'Faulted', fault: { ...verification, source: 'hardware' } })?.id).toBe('fault:verification:port_open_no_bytes');
  });

  it('asks for a board only when Source is Hardware', () => {
    expect(problemBanner(quiet)).toBeUndefined();
    const banner = problemBanner({ ...quiet, board: { ...quiet.board, source: 'hardware' } });
    expect(banner?.nextAction).toBe('refresh_board');
  });

  it('names a board without WVU firmware as the Board card does, with Restore and Verify again', () => {
    const board = { ...quiet.board, source: 'hardware' as const, selectedPort: 'COM5' };
    for (const state of [
      { compatibility: 'wvu_protocol_incompatible' },
      { compatibility: 'verification_failed', lastFailureCategory: 'protocol_verification_failed' },
      { compatibility: 'verification_failed', lastFailureCategory: null }
    ]) {
      const banner = problemBanner({ ...quiet, board: { ...board, ...state } });
      expect(banner?.headline, JSON.stringify(state)).toBe('No WVU firmware detected on COM5');
      expect(banner?.nextAction).toBe('restore_firmware');
      expect(banner?.secondary).toEqual({ action: 'verify_firmware', label: 'Verify again' });
    }
    // A busy port is not wrong firmware.
    const busy = problemBanner({ ...quiet, board: { ...board, compatibility: 'verification_failed', lastFailureCategory: 'port_busy' } });
    expect(busy?.headline).toBe('Another program is using the board');
    expect(busy?.nextAction).toBe('verify_firmware');
  });

  it('says nothing about an unchecked board while a scan or Verify is still running', () => {
    const board = { ...quiet.board, source: 'hardware' as const, selectedPort: 'COM5' };
    expect(problemBanner({ ...quiet, board })?.id).toBe('board:unchecked:COM5');
    expect(problemBanner({ ...quiet, board: { ...board, checking: true } })).toBeUndefined();
  });

  it('asks to verify a board that has not been checked, and uses the last firmware failure when there is one', () => {
    const hardware = { ...quiet.board, source: 'hardware' as const, selectedPort: 'COM5' };
    expect(problemBanner({ ...quiet, board: hardware })?.nextAction).toBe('verify_firmware');
    const failed = problemBanner({ ...quiet, board: { ...hardware, compatibility: 'verification_failed', lastFailureCategory: 'upload_failed' } });
    expect(failed?.headline).toBe('The firmware upload failed');
    expect(failed?.nextAction).toBe('restore_firmware');
  });

  it('shows no board problem while the firmware is ready or a restore runs', () => {
    const board = { ...quiet.board, source: 'hardware' as const, selectedPort: 'COM5' };
    expect(problemBanner({ ...quiet, board: { ...board, firmwareReady: true, compatibility: 'wvu_protocol_compatible' } })).toBeUndefined();
    expect(problemBanner({ ...quiet, board: { ...board, firmwareJobActive: true } })).toBeUndefined();
  });

  it('warns about low storage during a recording without an action', () => {
    const banner = problemBanner({ ...quiet, sessionState: 'Acquiring', active: true, storageWarning: 'Less than 1 GiB free' });
    expect(banner).toMatchObject({ id: 'storage:low', headline: 'Storage is running low', sentence: 'Less than 1 GiB free.' });
    expect(banner?.nextAction).toBeUndefined();
  });

  it('offers another folder for an unavailable folder or a planned recording that will not fit', () => {
    expect(problemBanner({ ...quiet, projectFolderNotice: 'E:\\ is not connected' })?.nextAction).toBe('choose_folder');
    expect(problemBanner({ ...quiet, storageShortfall: 'This recording needs 2 GiB' })?.nextAction).toBe('choose_folder');
  });

  it('shows a catalog problem last, with no action', () => {
    const banner = problemBanner({ ...quiet, catalogNotice: 'Local labs could not be loaded, so only the factory labs are shown. Ask your instructor.' });
    expect(banner?.id).toBe('catalog:problem');
    expect(banner?.nextAction).toBeUndefined();
  });
});
