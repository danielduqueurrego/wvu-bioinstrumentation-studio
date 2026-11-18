/**
 * The Source default. The page starts on Simulator (practice). The first
 * time WVU firmware becomes ready on the selected board (its
 * `verified_port`) while nothing is recording, Source switches to Hardware,
 * once. It never switches during or after a recording, and never again
 * once the student has chosen a Source themselves.
 */
export type RecordingSource = 'hardware' | 'simulator';

export type SourceAutoSelectInput = {
  source: RecordingSource;
  /** The switch already happened once. */
  autoSelected: boolean;
  /** The student picked a Source in the recording bar. */
  userChose: boolean;
  /** A recording has run or been started (this one or an earlier one). */
  recordingSeen: boolean;
  /** A recording runs or a Start is being sent right now. */
  sessionActive: boolean;
  /** Firmware was ready on the selected port at the previous check. */
  wasReady: boolean;
  /** Firmware is ready on the selected port now. */
  ready: boolean;
};

export function shouldAutoSelectHardware(input: SourceAutoSelectInput): boolean {
  return input.ready
    && !input.wasReady
    && input.source !== 'hardware'
    && !input.autoSelected
    && !input.userChose
    && !input.recordingSeen
    && !input.sessionActive;
}
