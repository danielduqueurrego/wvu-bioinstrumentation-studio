import { describe, expect, it } from 'vitest';
import { commandErrorDetail, commandErrorRepairable, formatCommandError } from './errors';

const portBusy = {
  category: 'port_busy',
  stage: 'verifying_firmware',
  title: 'Serial port is busy',
  explanation: 'COM12 could not be opened. Another program is probably using it, so the firmware was not checked.',
  recommended_action: 'Close Arduino IDE, Serial Monitor, or Arduino Cloud Agent, then select Verify Firmware again.',
  technical_details: 'Access is denied.'
};

describe('command error text', () => {
  it('renders a structured firmware failure as title, explanation, and next step', () => {
    expect(formatCommandError(portBusy)).toBe(
      'Serial port is busy. COM12 could not be opened. Another program is probably using it, so the firmware was not checked. Close Arduino IDE, Serial Monitor, or Arduino Cloud Agent, then select Verify Firmware again.'
    );
  });

  it('never shows technical details, raw JSON, or snake_case categories', () => {
    for (const message of [formatCommandError(portBusy), formatCommandError(JSON.stringify(portBusy))]) {
      expect(message).not.toContain('Access is denied');
      expect(message).not.toContain('port_busy');
      expect(message).not.toContain('{');
    }
  });

  it('accepts the same failure sent as a JSON string', () => {
    expect(formatCommandError(JSON.stringify(portBusy))).toBe(formatCommandError(portBusy));
  });

  it('passes plain string errors through and gives empty errors a fallback', () => {
    expect(formatCommandError('WVU Firmware is being restored. Wait for the restore to finish, then try again.'))
      .toBe('WVU Firmware is being restored. Wait for the restore to finish, then try again.');
    expect(formatCommandError(new Error('Task failed'))).toBe('Task failed');
    expect(formatCommandError('')).toMatch(/could not be completed/);
    expect(formatCommandError(undefined)).toMatch(/could not be completed/);
    expect(formatCommandError('{not json')).toBe('{not json');
  });

  it('does not repeat a sentence and uses a start failure message when present', () => {
    expect(formatCommandError({ title: 'Upload failed', explanation: 'Upload failed.' })).toBe('Upload failed.');
    expect(formatCommandError({ stage: 'CLAIM_SERIAL', userMessage: 'The firmware is being verified.' }))
      .toBe('The firmware is being verified.');
  });
});

describe('Arduino tools repairability', () => {
  it('offers Repair only when the failure does not require a reinstall', () => {
    const missing = {
      title: 'Arduino tools missing',
      explanation: 'The Arduino tools included with WVU Bioinstrumentation Studio were not found.',
      recommended_action: 'Reinstall WVU Bioinstrumentation Studio or contact your instructor.',
      technical_details: 'missing',
      repairable: false
    };
    expect(commandErrorRepairable(missing)).toBe(false);
    expect(commandErrorRepairable(JSON.stringify(missing))).toBe(false);
    expect(commandErrorRepairable({ ...missing, repairable: true })).toBe(true);
    // Failures without the field (task errors, plain strings) stay repairable.
    expect(commandErrorRepairable({ title: 'Arduino tools need attention' })).toBe(true);
    expect(commandErrorRepairable('Arduino tool repair task failed')).toBe(true);
    expect(commandErrorRepairable(undefined)).toBe(true);
  });
});

describe('command error detail', () => {
  it('keeps the technical detail for Advanced details', () => {
    expect(commandErrorDetail(portBusy)).toBe('Access is denied.');
    expect(commandErrorDetail(JSON.stringify(portBusy))).toBe('Access is denied.');
    expect(commandErrorDetail({ title: 'Arduino tools missing' })).toBe('');
    expect(commandErrorDetail('plain failure')).toBe('plain failure');
  });
});
