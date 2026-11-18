/**
 * One-line summaries shown on collapsed setup cards (for example
 * "Arduino UNO R4 WiFi · COM5 · Firmware ready"). Text only; they read
 * the same state the cards edit and never change it.
 */
import type { DisplayUnit } from './calibration';

export type BoardSummaryInput = {
  port: string;
  boardName?: string;
  firmwareReady: boolean;
  compatibility: string;
  firmwareJobActive: boolean;
  toolsState: 'preparing' | 'ready' | 'failed';
};

export function boardSummary(input: BoardSummaryInput): string {
  const tools = input.toolsState === 'failed' ? ' · Arduino tools need attention' : '';
  if (!input.port) return `No board selected${tools}`;
  const firmware = input.firmwareReady
    ? 'Firmware ready'
    : input.firmwareJobActive || input.compatibility === 'upload_in_progress'
      ? 'Restoring firmware…'
      : input.compatibility === 'wvu_protocol_incompatible'
        ? 'No WVU firmware'
        : input.compatibility === 'verification_failed'
          ? 'Firmware not verified'
          : 'Firmware not checked';
  return `${input.boardName || 'Arduino UNO R4 WiFi'} · ${input.port} · ${firmware}${tools}`;
}

export function projectSummary(projectFolder: string, notice = ''): string {
  if (notice.trim()) return 'Needs attention';
  return projectFolder.trim() || 'Choose a Project folder';
}

export function labSummary(lab: { display_name: string; profile_version: string } | undefined, instructor: boolean): string {
  const mode = instructor ? 'Instructor mode' : 'Student mode';
  return lab ? `${lab.display_name} ${lab.profile_version} · ${mode}` : `Choose a lab · ${mode}`;
}

export function sessionSummary(input: { outputFolder: string; note: string; rateText?: string }): string {
  const parts = [`Output: ${input.outputFolder.trim() || 'Project folder'}`];
  if (input.note.trim()) parts.push('Note added');
  if (input.rateText) parts.push(input.rateText);
  return parts.join(' · ');
}

/** "30 seconds", "1 minute", "90 seconds", "5 minutes". */
export function durationLabel(seconds: number): string {
  if (Number.isFinite(seconds) && seconds >= 60 && seconds % 60 === 0) {
    const minutes = seconds / 60;
    return `${minutes} minute${minutes === 1 ? '' : 's'}`;
  }
  return `${seconds} second${seconds === 1 ? '' : 's'}`;
}

export function durationSummary(mode: 'timed' | 'until_stopped', seconds: number, valid: boolean): string {
  if (mode === 'until_stopped') return 'Until stopped';
  return valid ? `Timed · ${durationLabel(seconds)}` : 'Timed · choose a valid duration';
}

const SHORT_UNIT: Record<DisplayUnit, string> = {
  counts: 'ADC counts',
  volts: 'volts',
  kpa: 'kPa',
  mmhg: 'mmHg',
  calibrated: 'calibrated units'
};

export function calibrationSummary(input: {
  channelIds: string[];
  units: Record<string, DisplayUnit | undefined>;
  calibrationLabel?: string;
}): string {
  const names = [...new Set(input.channelIds.map((id) => SHORT_UNIT[input.units[id] ?? 'counts']))];
  const units = names.length === 0 ? 'Units: —' : names.length === 1 ? `Units: ${names[0]}` : `Units: mixed (${names.join(', ')})`;
  return `${units} · Calibration: ${input.calibrationLabel?.trim() || 'none'}`;
}

export function plotArrangementSummary(plots: number, visibleSignals: number, totalSignals: number): string {
  const layout = `Layout: ${plots} plot${plots === 1 ? '' : 's'}`;
  return visibleSignals === totalSignals
    ? `${layout} · all ${totalSignals} signal${totalSignals === 1 ? '' : 's'} shown`
    : `${layout} · ${visibleSignals} of ${totalSignals} signals shown`;
}
