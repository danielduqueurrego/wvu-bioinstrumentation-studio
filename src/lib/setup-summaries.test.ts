import { describe, expect, it } from 'vitest';
import {
  boardSummary,
  calibrationSummary,
  durationLabel,
  durationSummary,
  labSummary,
  plotArrangementSummary,
  projectSummary,
  sessionSummary
} from './setup-summaries';

describe('collapsed card summaries', () => {
  const board = { port: 'COM5', boardName: 'Arduino UNO R4 WiFi', firmwareReady: true, compatibility: 'wvu_protocol_compatible', firmwareJobActive: false, toolsState: 'ready' as const };

  it('summarizes the board and its firmware in words', () => {
    expect(boardSummary(board)).toBe('Arduino UNO R4 WiFi · COM5 · Firmware ready');
    expect(boardSummary({ ...board, firmwareReady: false, compatibility: 'unknown' })).toBe('Arduino UNO R4 WiFi · COM5 · Firmware not checked');
    expect(boardSummary({ ...board, firmwareReady: false, compatibility: 'wvu_protocol_incompatible' })).toMatch(/No WVU firmware$/);
    expect(boardSummary({ ...board, firmwareReady: false, compatibility: 'verification_failed' })).toMatch(/Firmware not verified$/);
    expect(boardSummary({ ...board, firmwareReady: false, firmwareJobActive: true })).toMatch(/Restoring firmware…$/);
    expect(boardSummary({ ...board, port: '', toolsState: 'failed' })).toBe('No board selected · Arduino tools need attention');
  });

  it('summarizes the folder, lab, and session', () => {
    expect(projectSummary('C:\\BMEG 420L')).toBe('C:\\BMEG 420L');
    expect(projectSummary('', '')).toBe('Choose a Project folder');
    expect(projectSummary('E:\\Lab', 'E:\\ is not connected')).toBe('Needs attention');
    expect(labSummary({ display_name: 'Op Amp Lab', profile_version: '2.0.1' }, false)).toBe('Op Amp Lab 2.0.1 · Student mode');
    expect(labSummary(undefined, true)).toBe('Choose a lab · Instructor mode');
    expect(sessionSummary({ outputFolder: 'Participant01\\Trial03', note: '', rateText: '1000 frames/s' })).toBe('Output: Participant01\\Trial03 · 1000 frames/s');
    expect(sessionSummary({ outputFolder: '  ', note: 'warm-up' })).toBe('Output: Project folder · Note added');
  });

  it('summarizes the duration', () => {
    expect(durationLabel(30)).toBe('30 seconds');
    expect(durationLabel(60)).toBe('1 minute');
    expect(durationLabel(300)).toBe('5 minutes');
    expect(durationLabel(90)).toBe('90 seconds');
    expect(durationSummary('timed', 60, true)).toBe('Timed · 1 minute');
    expect(durationSummary('timed', 3, false)).toBe('Timed · choose a valid duration');
    expect(durationSummary('until_stopped', 60, true)).toBe('Until stopped');
  });

  it('summarizes units and the applied calibration', () => {
    expect(calibrationSummary({ channelIds: ['a0', 'a1'], units: { a0: 'volts', a1: 'volts' } })).toBe('Units: volts · Calibration: none');
    expect(calibrationSummary({ channelIds: ['a0', 'a1'], units: { a0: 'counts' }, calibrationLabel: 'Team XGZP' }))
      .toBe('Units: ADC counts · Calibration: Team XGZP');
    expect(calibrationSummary({ channelIds: ['a0', 'a1'], units: { a0: 'counts', a1: 'mmhg' } })).toBe('Units: mixed (ADC counts, mmHg) · Calibration: none');
  });

  it('summarizes the plot arrangement', () => {
    expect(plotArrangementSummary(3, 3, 3)).toBe('Layout: 3 plots · all 3 signals shown');
    expect(plotArrangementSummary(1, 2, 4)).toBe('Layout: 1 plot · 2 of 4 signals shown');
  });
});
