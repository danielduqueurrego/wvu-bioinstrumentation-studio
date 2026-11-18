import { describe, expect, it } from 'vitest';
import { markerElapsedSeconds, markerFeedback, markerLabelError, markerLineText, markersInRange, placeMarkers } from './markers';

describe('recording markers', () => {
  it('reports the marker in elapsed recording seconds, not board microseconds', () => {
    expect(markerElapsedSeconds(48_213_377, 35_913_377)).toBeCloseTo(12.3);
    expect(markerFeedback({ timestamp_us: 48_213_377, label: '' }, 35_913_377)).toBe('Marker added at 12.3 s.');
    expect(markerFeedback({ timestamp_us: 48_213_377, label: ' baseline ' }, 35_913_377)).toBe('Marker “baseline” added at 12.3 s.');
  });

  it('does not invent a time when the origin is unknown or later than the marker', () => {
    expect(markerElapsedSeconds(10, null)).toBeUndefined();
    expect(markerElapsedSeconds(0, 5_000)).toBeUndefined();
    expect(markerFeedback({ timestamp_us: 0, label: '' }, 5_000)).toBe('Marker added.');
  });

  it('accepts an empty or short label and refuses what the backend would refuse', () => {
    expect(markerLabelError('')).toBeUndefined();
    expect(markerLabelError('x'.repeat(80))).toBeUndefined();
    expect(markerLabelError(`  ${'x'.repeat(80)}  `)).toBeUndefined();
    expect(markerLabelError('x'.repeat(81))).toMatch(/80 characters/);
    // The backend counts UTF-8 bytes: 41 two-byte characters are 82 bytes.
    expect(markerLabelError('é'.repeat(41))).toMatch(/80 characters/);
    expect(markerLabelError('a\0b')).toBeDefined();
  });

  it('labels each plotted marker line with its label or its number', () => {
    expect(markerLineText(' baseline ', 1)).toBe('baseline');
    expect(markerLineText('', 2)).toBe('Marker 2');
    const placed = placeMarkers([
      { seconds: 2.5, label: 'baseline' },
      { seconds: undefined, label: 'no time' },
      { seconds: 12.3, label: '' }
    ]);
    // A marker without a known time is not drawn; numbering follows the order added.
    expect(placed).toEqual([
      { id: 'marker-1', number: 1, seconds: 2.5, text: 'baseline' },
      { id: 'marker-3', number: 3, seconds: 12.3, text: 'Marker 3' }
    ]);
  });

  it('draws only the markers inside the plotted time window', () => {
    const placed = placeMarkers([{ seconds: 1, label: 'a' }, { seconds: 7, label: 'b' }, { seconds: 12, label: 'c' }]);
    expect(markersInRange(placed, 5, 12).map((marker) => marker.text)).toEqual(['b', 'c']);
    expect(markersInRange(placed, undefined, 12)).toEqual([]);
  });
});
