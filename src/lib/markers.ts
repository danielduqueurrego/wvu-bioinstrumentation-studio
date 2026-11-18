/**
 * Recording markers as students see them: an optional short label and the
 * time in elapsed recording seconds (the plot's time axis), never board
 * microseconds since power-up. The backend stores the raw timestamp.
 */

/** The backend refuses a trimmed label longer than 80 UTF-8 bytes. */
export const MAX_MARKER_LABEL_BYTES = 80;

export type RecordingMarker = { timestamp_us: number; label: string };

export function markerLabelError(label: string): string | undefined {
  const trimmed = label.trim();
  if (trimmed.includes('\0')) return 'A marker label cannot contain that character.';
  if (new TextEncoder().encode(trimmed).length > MAX_MARKER_LABEL_BYTES) {
    return `Keep the marker label to ${MAX_MARKER_LABEL_BYTES} characters or fewer.`;
  }
  return undefined;
}

/** Seconds since the first recorded frame, or undefined if unknown. */
export function markerElapsedSeconds(timestampUs: number, originUs: number | null | undefined): number | undefined {
  if (originUs == null || !Number.isFinite(originUs) || !Number.isFinite(timestampUs) || timestampUs < originUs) return undefined;
  return (timestampUs - originUs) / 1_000_000;
}

/** A marker placed on the plots' elapsed-time axis. */
export type PlacedMarker = { id: string; number: number; seconds: number; text: string };

/** The text drawn beside a marker line: its label, or "Marker 2". */
export function markerLineText(label: string, number: number): string {
  return label.trim() || `Marker ${number}`;
}

/**
 * Places the markers of the current recording on the plots' time axis. The
 * elapsed time is computed once, when the marker is added, from the
 * recording's own display origin, so it stays correct after the recording
 * ends. Markers without a known time are left off the plots.
 */
export function placeMarkers(markers: ReadonlyArray<{ seconds?: number; label: string }>): PlacedMarker[] {
  return markers.flatMap((marker, index) => marker.seconds === undefined || !Number.isFinite(marker.seconds)
    ? []
    : [{ id: `marker-${index + 1}`, number: index + 1, seconds: marker.seconds, text: markerLineText(marker.label, index + 1) }]);
}

/** Markers inside the plotted time range (inclusive). */
export function markersInRange<T extends { seconds: number }>(markers: ReadonlyArray<T>, min: number | undefined, max: number | undefined): T[] {
  if (min === undefined || max === undefined || !Number.isFinite(min) || !Number.isFinite(max)) return [];
  return markers.filter((marker) => marker.seconds >= min && marker.seconds <= max);
}

/** "Marker added at 12.3 s" or "Marker “baseline” added at 12.3 s". */
export function markerFeedback(marker: RecordingMarker, originUs: number | null | undefined): string {
  const elapsed = markerElapsedSeconds(marker.timestamp_us, originUs);
  const label = marker.label.trim();
  const name = label ? `Marker “${label}”` : 'Marker';
  return elapsed === undefined ? `${name} added.` : `${name} added at ${elapsed.toFixed(1)} s.`;
}
