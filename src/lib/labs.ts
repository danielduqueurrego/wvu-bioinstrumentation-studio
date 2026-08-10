/** Shared frontend shape for the versioned acquisition-lab API. */
export type LabChannel = {
  pin: string; id: string; label: string; csv_name: string; units: string;
  allowed_conversions?: string[]; default_display_unit?: string; default_visible?: boolean;
};

export type LabDigitalOutput = {
  pin: string;
  label: string;
  behavior: 'always_low' | 'high_while_recording' | 'acquisition_sequenced';
};

export type LabPlotGroup = { channel_ids: string[] };

export type LabAcquisition = {
  analog_pin: string; adc_resolution_bits: number; sample_rate_hz: number;
  allowed_duration_modes: string[]; timed_presets_seconds: number[]; minimum_custom_duration_seconds: number;
  acquisition_mode: 'simultaneous' | 'pulseox_4state'; channels?: LabChannel[];
  analog_inputs?: { tx: string; rx: string };
  led_outputs?: { green?: string | null; red?: string | null; ir?: string | null };
  state_dwell_us?: number;
  digital_outputs?: LabDigitalOutput[];
};

export type LabProfile = {
  schema_version: number; profile_id: string; profile_version: string; display_name: string; category: string;
  status: 'locked' | 'draft' | 'retired'; source: 'built_in' | 'instructor'; description: string;
  target_board: string; fqbn: string;
  required_firmware: { protocol_major: number; protocol_minor_min: number; build: string; device: string };
  acquisition: LabAcquisition;
  display: { primary_quantity: string; channel_label: string; raw_units_label: string; voltage_units_label: string; voltage_reference_v: number; plot_min_v: number; plot_max_v: number };
  safety: { bench_only: boolean; human_connection_authorized: boolean; not_medical_device: boolean; notices: string[] };
  export: { signal_name: string; include_profile_snapshot: boolean };
  integrity: { canonical_hash_algorithm: string; canonical_hash: string };
  plot_defaults?: { groups: LabPlotGroup[] };
  associated_sketch?: { name: string; relative_path?: string; source_hash?: string; is_wvu_reference: boolean };
};

export type LabListEntry = { profile: LabProfile; active: boolean; retired: boolean };
