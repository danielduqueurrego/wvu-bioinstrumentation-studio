/**
 * Which setup cards are expanded. Pure state transitions, so the page only
 * wires them to its session state:
 *
 * - Student mode starts with Calibration & Units collapsed; Instructor mode
 *   expands it.
 * - While a recording runs every card collapses (the controls are read-only
 *   anyway) so the live plot fills the screen; the previous layout returns
 *   when the recording ends.
 * - A readiness item opens the card that holds its control.
 *
 * Display-only: none of this issues an acquisition command.
 */
export type SetupCardId = 'board' | 'project' | 'lab' | 'session' | 'duration' | 'calibration';

export type CardOpenState = Record<SetupCardId, boolean>;

export const SETUP_CARD_IDS: SetupCardId[] = ['board', 'project', 'lab', 'session', 'duration', 'calibration'];

export function initialCardState(instructor = false): CardOpenState {
  return { board: true, project: true, lab: true, session: true, duration: true, calibration: instructor };
}

/** Applies a mode's default for the advanced card; other cards are kept. */
export function withModeDefaults(state: CardOpenState, instructor: boolean): CardOpenState {
  return state.calibration === instructor ? state : { ...state, calibration: instructor };
}

export function collapseAll(state: CardOpenState): CardOpenState {
  return SETUP_CARD_IDS.every((id) => !state[id])
    ? state
    : Object.fromEntries(SETUP_CARD_IDS.map((id) => [id, false])) as CardOpenState;
}

export function openCard(state: CardOpenState, card: SetupCardId): CardOpenState {
  return state[card] ? state : { ...state, [card]: true };
}

export type RecordingDisclosure = {
  open: CardOpenState;
  /** The layout to restore once the recording ends (set while recording). */
  saved?: CardOpenState;
};

/**
 * Collapses the cards when a recording becomes active and restores the
 * saved layout when it ends. Repeated calls with the same `active` value
 * change nothing, so a 40 ms status poll cannot fight the student.
 */
export function recordingDisclosure(current: RecordingDisclosure, active: boolean): RecordingDisclosure {
  if (active && !current.saved) return { open: collapseAll(current.open), saved: current.open };
  if (!active && current.saved) return { open: current.saved };
  return current;
}
