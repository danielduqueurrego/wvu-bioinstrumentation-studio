/** Primary class-workflow navigation. Formal validation is intentionally not a runtime feature. */
export const PRIMARY_NAVIGATION = ['Home', 'Firmware', 'Acquisition', 'Diagnostics'] as const;

export type PrimaryView = typeof PRIMARY_NAVIGATION[number];
