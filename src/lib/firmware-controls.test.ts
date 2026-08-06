import { describe, expect, it } from 'vitest';
import { firmwareCompatibilityMessage, firmwareControls } from './firmware-controls';

describe('firmware workspace controls', () => {
  it('shows unsaved state by disabling compile and upload until Save completes', () => {
    expect(firmwareControls({
      hasProject: true,
      unsavedChanges: true,
      activeJob: false,
      hasCurrentCompile: true,
      selectedPort: 'COM12'
    })).toEqual({ saveEnabled: true, compileEnabled: false, uploadEnabled: false, restoreEnabled: true });
  });

  it('requires a saved compile, a selected port, and no active job before upload', () => {
    expect(firmwareControls({
      hasProject: true,
      unsavedChanges: false,
      activeJob: false,
      hasCurrentCompile: false,
      selectedPort: 'COM12'
    }).uploadEnabled).toBe(false);
    expect(firmwareControls({
      hasProject: true,
      unsavedChanges: false,
      activeJob: false,
      hasCurrentCompile: true,
      selectedPort: 'COM12'
    }).uploadEnabled).toBe(true);
    expect(firmwareControls({
      hasProject: true,
      unsavedChanges: false,
      activeJob: true,
      hasCurrentCompile: true,
      selectedPort: 'COM12'
    }).restoreEnabled).toBe(false);
  });

  it('makes the non-WVU consequence explicit rather than calling upload a hardware failure', () => {
    expect(firmwareCompatibilityMessage('non_wvu_sketch')).toContain('upload succeeded');
    expect(firmwareCompatibilityMessage('non_wvu_sketch')).toContain('Acquisition remains unavailable');
  });
});
