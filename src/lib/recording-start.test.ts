import { describe, expect, it } from 'vitest';
import {
  hardwareStartInvokePayload,
  recordingStartFailure,
  recordingStartReadiness,
  simulatorStartInvokePayload,
  startFeedbackView,
  type HardwareStartRequest,
  type SimulatorStartRequest
} from './recording-start';

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

  it('does not send a student to firmware recovery after a storage fault', () => {
    const result = recordingStartReadiness({ ...readyHardware, sessionState: 'Faulted', faultOrigin: 'storage' });
    expect(result.canStart).toBe(false);
    expect(result.message).toMatch(/not saved completely/i);
    expect(result.message).not.toMatch(/firmware/i);
  });

  it('does not send a student to firmware recovery after a refused recording request', () => {
    const result = recordingStartReadiness({ ...readyHardware, sessionState: 'Faulted', faultOrigin: 'request' });
    expect(result.canStart).toBe(false);
    expect(result.message).toMatch(/did not start/i);
    expect(result.message).not.toMatch(/firmware/i);
  });

  it('lets a Simulator start clear any previous fault, while a hardware start still waits', () => {
    const simulator = { ...readyHardware, source: 'simulator' as const, selectedBoard: false, firmwareReady: false };
    for (const faultOrigin of [undefined, 'verification', 'connection', 'recording', 'storage', 'request']) {
      expect(recordingStartReadiness({ ...simulator, sessionState: 'Faulted', faultOrigin })).toEqual({ canStart: true });
      expect(recordingStartReadiness({ ...readyHardware, sessionState: 'Faulted', faultOrigin }).canStart).toBe(false);
    }
    // A running session still blocks the Simulator.
    expect(recordingStartReadiness({ ...simulator, sessionState: 'Acquiring' }).canStart).toBe(false);
  });

  it('uses the lab duration rule as the reason when it has one', () => {
    const message = 'This lab requires timed recordings of at least 30 seconds, or one of its preset durations. Choose a longer duration, then start again.';
    expect(recordingStartReadiness({ ...readyHardware, durationValid: false, durationMessage: message }).message).toBe(message);
    expect(recordingStartReadiness({ ...readyHardware, durationValid: false }).message).toMatch(/valid timed duration/i);
  });

  it('blocks Start while an ADC reference or MPXV supply field is invalid', () => {
    const result = recordingStartReadiness({ ...readyHardware, calibrationInputError: 'Enter the ADC reference voltage between 0.1 and 10 V.' });
    expect(result.canStart).toBe(false);
    expect(result.message).toMatch(/ADC reference/);
  });

  it('does not use raw backend errors as the primary student message', () => {
    expect(recordingStartFailure('only one session may be active; disconnect first').userMessage).toMatch(/Arduino is busy/i);
    expect(recordingStartFailure('firmware compatibility is not verified').userMessage).toMatch(/firmware is not ready/i);
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

  it('keeps the simulator Start request under the named Tauri command argument', () => {
    const request = {
      project_folder: 'C:\\Users\\Student\\Documents\\BMEG 420L',
      output_folder: 'Test1',
      duration: { mode: 'until_stopped' },
      profile_id: 'general-analog',
      bench_notice_acknowledged: true,
      recording_note: 'Bench test',
      calibration: null
    };
    expect(simulatorStartInvokePayload(request)).toEqual({ request });
  });

  it('sends the shown lab revision as profile_version in both Start requests', () => {
    const simulator: SimulatorStartRequest = {
      project_folder: 'C:\\BMEG',
      output_folder: '',
      duration: { mode: 'timed', seconds: 60 },
      profile_id: 'wvu.bmeg420l.general.analog.development.v2',
      profile_version: '2.0.1',
      bench_notice_acknowledged: false,
      recording_note: '',
      calibration: null
    };
    const hardware: HardwareStartRequest = { ...simulator, port: 'COM5' };
    expect(simulatorStartInvokePayload(simulator).request.profile_version).toBe('2.0.1');
    expect(hardwareStartInvokePayload(hardware).request).toMatchObject({ port: 'COM5', profile_id: simulator.profile_id, profile_version: '2.0.1' });
    // The Rust field names, exactly (serde does not rename them).
    expect(Object.keys(hardwareStartInvokePayload(hardware).request).sort()).toEqual([
      'bench_notice_acknowledged', 'calibration', 'duration', 'output_folder', 'port',
      'profile_id', 'profile_version', 'project_folder', 'recording_note'
    ]);
  });
});

describe('feedback under Start', () => {
  const ready = { canStart: true };
  const blocked = { canStart: false, message: 'Choose a writable Project folder before recording.' };

  it('shows a running recording as a normal status, not a warning', () => {
    const view = startFeedbackView({
      sessionState: 'Acquiring',
      startInFlight: false,
      readiness: { canStart: false, message: 'A recording or connection is already active. Stop it before starting another recording.' },
      feedback: ''
    });
    expect(view.tone).toBe('status');
    expect(view.text).toMatch(/Stop recording/);
    expect(startFeedbackView({ sessionState: 'Stopping', startInFlight: false, readiness: blocked, feedback: '' }).text).toMatch(/saving/i);
  });

  it('never shows "Configuring recording…" after the recording has finished', () => {
    // The page clears its Start text once Start returns.
    expect(startFeedbackView({ sessionState: 'Disconnected', startInFlight: false, readiness: ready, feedback: '' }))
      .toEqual({ tone: 'none', text: '' });
    expect(startFeedbackView({ sessionState: 'Configured', startInFlight: false, readiness: blocked, feedback: '' }).text)
      .toBe('Configuring recording…');
  });

  it('treats a Verify or Retry handshake as a board check, not as a running recording', () => {
    const checking = recordingStartReadiness({ ...readyHardware, sessionState: 'Connected', sessionActive: false });
    expect(checking.canStart).toBe(false);
    expect(checking.message).toMatch(/being checked/);
    const view = startFeedbackView({ sessionState: 'Connecting', sessionActive: false, startInFlight: false, readiness: checking, feedback: '' });
    expect(view).toEqual({ tone: 'warning', text: checking.message });
    expect(startFeedbackView({ sessionState: 'Connecting', sessionActive: true, startInFlight: false, readiness: checking, feedback: '' }).text)
      .toBe('Connecting to the Arduino…');
  });

  it('shows the Start progress while Start runs, and readiness problems only when idle', () => {
    expect(startFeedbackView({ sessionState: 'Disconnected', startInFlight: true, readiness: { canStart: false, message: 'The Arduino is busy.' }, feedback: 'Connecting to Arduino…' }))
      .toEqual({ tone: 'status', text: 'Connecting to Arduino…' });
    expect(startFeedbackView({ sessionState: 'Disconnected', startInFlight: false, readiness: blocked, feedback: '' }))
      .toEqual({ tone: 'warning', text: blocked.message });
    expect(startFeedbackView({ sessionState: 'Faulted', startInFlight: false, readiness: ready, feedback: 'The selected lab changed.' }))
      .toEqual({ tone: 'status', text: 'The selected lab changed.' });
  });
});
