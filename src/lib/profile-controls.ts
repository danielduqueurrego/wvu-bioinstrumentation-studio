export type ProfileControlInput = {
  category: string;
  locked: boolean;
  benchAcknowledged: boolean;
  firmwareCompatible: boolean;
  source: 'simulator' | 'hardware';
};

/** UI policy only; Rust independently validates every profile start request. */
export function profileControls(input: ProfileControlInput) {
  // Legacy bench profiles retain their explicit acknowledgement. Course
  // capture uses its profile safety notice and firmware compatibility instead.
  const requiresBenchAcknowledgement = input.category === 'ecg' || input.category === 'emg';
  const acknowledgementSatisfied = !requiresBenchAcknowledgement || input.benchAcknowledged;
  return {
    protectedSettings: input.locked,
    requiresBenchAcknowledgement,
    acknowledgementSatisfied,
    canStart: input.locked && acknowledgementSatisfied
      && (input.source === 'simulator' || input.firmwareCompatible),
    firmwareMessage: input.source === 'hardware' && !input.firmwareCompatible
      ? 'Profile requires the controlled WVU firmware. Selected firmware identity does not match.'
      : ''
  };
}
