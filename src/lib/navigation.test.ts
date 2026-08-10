import { describe, expect, it } from 'vitest';
import { PRIMARY_NAVIGATION } from './navigation';

describe('class-workflow navigation', () => {
  it('exposes only the class application pages', () => {
    expect(PRIMARY_NAVIGATION).toEqual(['Home', 'Firmware', 'Acquisition', 'Diagnostics']);
    expect(PRIMARY_NAVIGATION).not.toContain('Validation');
  });
});
