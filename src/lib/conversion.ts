export const ADC_FULL_SCALE_COUNTS = 4095;
export const ADC_REFERENCE_VOLTS = 5;

/** Direct Phase 1 conversion; no calibration or filtering is applied. */
export function countsToVolts(counts: number): number {
  return counts * ADC_REFERENCE_VOLTS / ADC_FULL_SCALE_COUNTS;
}
