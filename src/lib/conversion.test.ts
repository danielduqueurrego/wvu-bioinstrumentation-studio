import { describe, expect, it } from 'vitest';
import { countsToVolts } from './conversion';

describe('direct ADC volts conversion', () => {
  it('maps the documented 12-bit 0–5 V endpoints', () => {
    expect(countsToVolts(0)).toBe(0);
    expect(countsToVolts(4095)).toBe(5);
  });
});
