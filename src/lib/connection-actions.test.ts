import { describe, expect, it } from 'vitest';
import { connectionActions } from './connection-actions';

describe('connection recovery actions', () => {
  it('offers an explicit reset only for a supported idle hardware failure', () => {
    expect(connectionActions({
      source: 'hardware', active: false, selectedPort: 'COM12', failureCategory: 'port_open_no_bytes'
    })).toEqual({ canRetryHandshake: true, canReset: true });
  });

  it('never permits retry or reset during a recording or without a selected UNO', () => {
    expect(connectionActions({
      source: 'hardware', active: true, selectedPort: 'COM12', failureCategory: 'port_open_no_bytes'
    })).toEqual({ canRetryHandshake: false, canReset: false });
    expect(connectionActions({
      source: 'hardware', active: false, selectedPort: '', failureCategory: 'port_open_no_bytes'
    })).toEqual({ canRetryHandshake: false, canReset: false });
  });

  it('does not offer board reset for simulator or unsupported diagnostic categories', () => {
    expect(connectionActions({
      source: 'simulator', active: false, selectedPort: 'SIM', failureCategory: 'handshake_incomplete'
    })).toEqual({ canRetryHandshake: false, canReset: false });
    expect(connectionActions({
      source: 'hardware', active: false, selectedPort: 'COM12', failureCategory: 'wrong_protocol_version'
    })).toEqual({ canRetryHandshake: true, canReset: false });
  });
});
