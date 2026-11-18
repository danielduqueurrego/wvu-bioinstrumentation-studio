import { describe, expect, it } from 'vitest';
import { connectionActions } from './connection-actions';

describe('connection recovery actions', () => {
  it('offers Retry connection for an idle hardware connection failure', () => {
    for (const failureCategory of ['port_busy', 'port_open_no_bytes', 'handshake_incomplete', 'wrong_protocol_version']) {
      expect(connectionActions({
        source: 'hardware', active: false, selectedPort: 'COM12', failureCategory
      })).toEqual({ canRetryHandshake: true });
    }
  });

  it('never permits retry during a recording, without a selected UNO, or without a failure', () => {
    expect(connectionActions({
      source: 'hardware', active: true, selectedPort: 'COM12', failureCategory: 'port_open_no_bytes'
    })).toEqual({ canRetryHandshake: false });
    expect(connectionActions({
      source: 'hardware', active: false, selectedPort: '', failureCategory: 'port_open_no_bytes'
    })).toEqual({ canRetryHandshake: false });
    expect(connectionActions({
      source: 'hardware', active: false, selectedPort: 'COM12', failureCategory: null
    })).toEqual({ canRetryHandshake: false });
  });

  it('does not offer a board action for the simulator or for a storage problem', () => {
    expect(connectionActions({
      source: 'simulator', active: false, selectedPort: 'SIM', failureCategory: 'handshake_incomplete'
    })).toEqual({ canRetryHandshake: false });
    expect(connectionActions({
      source: 'hardware', active: false, selectedPort: 'COM12', failureCategory: 'recording_storage_error'
    })).toEqual({ canRetryHandshake: false });
  });

  it('does not offer Retry connection for a lab the firmware cannot run', () => {
    expect(connectionActions({
      source: 'hardware', active: false, selectedPort: 'COM12', failureCategory: 'lab_not_supported_by_firmware'
    })).toEqual({ canRetryHandshake: false });
  });

  it('no longer offers the retired board reset', () => {
    expect(connectionActions({
      source: 'hardware', active: false, selectedPort: 'COM12', failureCategory: 'port_open_no_bytes'
    })).not.toHaveProperty('canReset');
  });
});
