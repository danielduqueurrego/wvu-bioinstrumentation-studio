import { describe, expect, it } from 'vitest';
import { profileControls } from './profile-controls';

describe('profile acquisition controls', () => {
  it('requires a bench-only acknowledgement for ECG and EMG', () => {
    expect(profileControls({ category: 'ecg', locked: true, benchAcknowledged: false, firmwareCompatible: true, source: 'simulator' }).canStart).toBe(false);
    expect(profileControls({ category: 'emg', locked: true, benchAcknowledged: true, firmwareCompatible: true, source: 'simulator' }).canStart).toBe(true);
  });

  it('explains a hardware firmware block without blocking simulator teaching data', () => {
    const hardware = profileControls({ category: 'ecg', locked: true, benchAcknowledged: true, firmwareCompatible: false, source: 'hardware' });
    expect(hardware.canStart).toBe(false);
    expect(hardware.firmwareMessage).toContain('controlled WVU firmware');
    expect(profileControls({ category: 'development', locked: true, benchAcknowledged: false, firmwareCompatible: false, source: 'simulator' }).canStart).toBe(true);
  });
});
