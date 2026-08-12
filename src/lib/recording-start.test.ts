import { describe, expect, it } from 'vitest';
import { hardwareStartInvokePayload, recordingStartFailure, recordingStartReadiness, studentRecordingStartError } from './recording-start';

const readyHardware = {
  source: 'hardware' as const,
  selectedBoard: true,
  firmwareReady: true,
  sessionState: 'Disconnected',
  boardOperationBusy: false,
  startInFlight: false,
  activeProfile: true,
  projectFolder: true,
  durationValid: true,
  acknowledgementSatisfied: true
};

describe('recording start readiness', () => {
  it('permits exactly one start for a ready, fresh hardware session', () => {
    expect(recordingStartReadiness(readyHardware)).toEqual({ canStart: true });
  });

  it.each([
    [{ ...readyHardware, selectedBoard: false }, /No Arduino is selected/i],
    [{ ...readyHardware, firmwareReady: false }, /firmware is not ready/i],
    [{ ...readyHardware, projectFolder: false }, /Project folder/i],
    [{ ...readyHardware, outputFolderError: 'The Output folder must stay inside the Project folder.' }, /Output folder/i],
    [{ ...readyHardware, durationValid: false }, /valid timed duration/i],
    [{ ...readyHardware, boardOperationBusy: true }, /Arduino is busy/i],
    [{ ...readyHardware, startInFlight: true }, /Arduino is busy/i],
    [{ ...readyHardware, sessionState: 'Faulted' }, /previous Arduino connection/i],
    [{ ...readyHardware, sessionState: 'Acquiring' }, /already active/i]
  ])('explains why a blocked start cannot begin', (input, message) => {
    const result = recordingStartReadiness(input);
    expect(result.canStart).toBe(false);
    expect(result.message).toMatch(message);
  });

  it('does not use raw backend errors as the primary student message', () => {
    expect(studentRecordingStartError('only one session may be active; disconnect first')).toMatch(/Arduino is busy/i);
    expect(studentRecordingStartError('firmware compatibility is not verified')).toMatch(/firmware is not ready/i);
  });

  it('retains structured backend stage and detail for Advanced details', () => {
    const failure = recordingStartFailure({
      stage: 'CONFIG_ACK',
      code: 'configuration_timeout',
      userMessage: 'The Arduino did not accept the recording setup. Verify the firmware and try again.',
      technicalDetail: 'timed out waiting for CONFIG_ACK after 1500 ms'
    });
    expect(failure.stage).toBe('CONFIG_ACK');
    expect(failure.technicalDetail).toMatch(/1500 ms/);
    expect(failure.userMessage).toMatch(/did not accept/i);
  });

  it('keeps the hardware Start request under the named Tauri command argument', () => {
    const request = { port: 'COM5', project_folder: 'C:\\Users\\Student\\Documents\\BMEG 420L' };
    expect(hardwareStartInvokePayload(request)).toEqual({ request });
  });
});
