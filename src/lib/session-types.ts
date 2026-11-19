/**
 * Shapes of the Tauri command results the Acquisition page reads. Field
 * names are the Rust serde (snake_case) names. Rust sends every missing
 * `Option` as `null`, so optional fields are typed `T | null` where the page
 * checks them, and compared with `== null`, never `=== undefined`.
 */
import type { RecordingCalibration } from './calibration';
import type { FirmwareCompatibility } from './board-controls';
import type { RecordingDurationRequest } from './duration';
import type { LabProfile } from './labs';
import type { FaultOrigin } from './messages';

export type { FaultOrigin } from './messages';

export type Point = { sequence: number; timestamp_us: number; values: number[]; status_flags: number };

export type Board = {
  port: string; name: string; fqbn: string; serial_number?: string;
  usb_vid?: number; usb_pid?: number;
};

export type Integrity = {
  received_packets: number; crc_failures: number; invalid_frames: number; unsupported_versions: number;
  missing_packet_sequences: number; duplicate_packets: number; out_of_order_packets: number;
  missing_sample_sequences: number; duplicate_sample_sequences: number; out_of_order_sample_sequences: number;
  firmware_overflows: number; host_channel_overflows: number; reconnects: number; disconnect_events: number;
  /**
   * Records in batches flagged 0x0004. Firmware build 0x00010007 sets it only
   * on the batch after a dropped batch (already counted as missing samples
   * and `firmware_overflows`); builds up to 0x00010006 also set it after a
   * skipped sampling deadline. Not part of `firmware_overflows`. Absent from
   * older backends.
   */
  overrun_flagged_records?: number;
  /**
   * Records in batches flagged 0x0010 (build 0x00010007 and later): the
   * firmware skipped one or more sampling deadlines before them. Absent from
   * older backends.
   */
  deadline_skip_flagged_records?: number;
  /** LED-sequenced records whose LED state is not `sequence % 4`. Absent from older backends. */
  led_state_mismatches?: number;
};

export const emptyIntegrity: Integrity = {
  received_packets: 0, crc_failures: 0, invalid_frames: 0, unsupported_versions: 0,
  missing_packet_sequences: 0, duplicate_packets: 0, out_of_order_packets: 0,
  missing_sample_sequences: 0, duplicate_sample_sequences: 0, out_of_order_sample_sequences: 0,
  firmware_overflows: 0, host_channel_overflows: 0, reconnects: 0, disconnect_events: 0
};

export type ProfileSnapshot = { bench_notice_acknowledged: boolean; profile: LabProfile };

/** Per LED state: RED, DARK 1, IR, DARK 2. */
export type PerLedState = [number, number, number, number];

/** Rust `AcquisitionTimingDiagnostics`: the timing checks written to a recording's metadata. */
export type AcquisitionTiming = {
  requested_rate_hz: number; measured_rate_hz: number; interval_count: number;
  median_interval_us: number; minimum_interval_us: number; maximum_interval_us: number; p99_interval_us: number;
  interval_tolerance_percent: number; intervals_outside_tolerance: number; acquisition_queue_overflows: number;
  timestamp_source: string;
  /** Pulse-ox labs: firmware-observed state intervals outside ±5% of the dwell. */
  pulse_state_intervals_outside_tolerance?: PerLedState | null;
  /** Pulse-ox labs: the largest firmware-observed dwell error (µs) per state. */
  pulse_state_maximum_error_us?: PerLedState | null;
  /** Sampling deadlines the board skipped (firmware build 0x00010007 and later). */
  firmware_skipped_deadlines?: number | null;
  /** Pulse-ox labs: the shortest time (µs) from an LED edge to that state's sample. */
  minimum_pulse_settle_us?: number | null;
};

/** Rust `SessionSummary`: the last finalized recording. */
export type Summary = {
  state: string; samples: number; packets: number; measured_rate_hz: number;
  board_elapsed_seconds: number; host_elapsed_seconds: number; bmeg_path: string; csv_path: string;
  metadata_path: string; recording_status: string; duration: RecordingDurationRequest; stop_reason: string;
  completion_status: string; initial_free_disk_bytes?: number; final_free_disk_bytes?: number;
  integrity: Integrity; error?: string; profile?: ProfileSnapshot; calibration?: RecordingCalibration;
  active_digital_output_mask?: number; final_digital_output_mask?: number;
  csv_error?: string; finalize_error?: string;
  /** The recording's timing checks (Advanced details). Absent from older backends. */
  timing_diagnostics?: AcquisitionTiming | null;
};

export type ConnectionDiagnostics = {
  selected_port: string; board: string; fqbn: string; port_opened: boolean;
  bytes_received: number; valid_frames: number; crc_failures: number; skipped_noise_bytes: number;
  hello_received: boolean; capabilities_received: boolean; pong_received: boolean;
  protocol_version?: string; firmware_build?: number; firmware_board_id?: number; raw_byte_classification: string; ping_attempts: number; handshake_elapsed_ms: number;
  reset_attempted: boolean; original_port?: string; final_port?: string;
  disappearance_observed: boolean; reappearance_observed: boolean; bootloader_observed: boolean;
  failure_category?: string; recommended_action: string;
  terminal_error_classification?: string; terminal_error_stage?: string;
  terminal_error_kind?: string; terminal_error_raw_os_error?: number;
  terminal_error_detail?: string; terminal_error_elapsed_ms?: number;
  last_valid_packet_utc?: string; last_valid_sample_utc?: string;
  last_successful_ping_utc?: string; last_pong_or_status_utc?: string;
  selected_port_present_after_error?: boolean; same_vid_pid_present_after_error?: boolean;
  same_serial_present_after_error?: boolean; uno_r4_present_after_error?: boolean;
  port_enumeration_error?: string;
};

/** Rust `SessionStatus` (`get_session_status`). */
export type SessionStatus = {
  state: string;
  /**
   * A recording (a Start) owns the session, Connecting through Stopping.
   * False while Verify Firmware, Retry connection, or a restore's
   * verification passes through Connecting/Connected (see `recordingActive`).
   */
  recording_active?: boolean;
  board: string; port: string; protocol_version: string; simulator: boolean;
  samples: number; packets: number; measured_rate_hz: number; integrity: Integrity;
  duration?: RecordingDurationRequest | null; elapsed_seconds: number; remaining_seconds?: number | null;
  available_disk_bytes?: number | null; storage_warning?: string | null; stop_reason?: string | null;
  connection_diagnostics?: ConnectionDiagnostics | null;
  last_error?: string | null; fault_origin?: FaultOrigin | null;
  last_summary?: Summary | null; calibration?: RecordingCalibration;
  /** The lab snapshot the current recording uses (set while a session runs). */
  profile?: ProfileSnapshot | null;
  digital_output_mask?: number | null;
  display_origin_timestamp_us?: number | null;
};

export type HandshakeRetryResult = { handshake_succeeded: boolean; diagnostics: ConnectionDiagnostics };

export type FirmwareFailure = {
  category: string; stage: string; title: string; explanation: string;
  recommended_action: string; technical_details: string;
};

export type FirmwareJob = {
  id: number; kind: string; stage: string; active: boolean; message: string;
  original_port?: string | null; final_port?: string | null;
  bootloader_port?: string | null;
  /** Set only while a native-USB restore waits for the double press of RESET. */
  manual_reset_deadline_utc?: string | null;
  started_utc?: string | null; completed_utc?: string | null;
  source_hash?: string | null;
  failure?: FirmwareFailure | null; log_path?: string | null;
};

export type FirmwareWorkflowStatus = {
  compatibility: FirmwareCompatibility; job?: FirmwareJob | null;
  last_restore?: FirmwareJob | null; last_failure?: FirmwareFailure | null;
  verified_port?: string | null;
};

export type FirmwareVerification = {
  declared_kind: string; compatible: boolean; protocol_version?: string;
  identity?: { protocol_version: string; firmware_build: number; device_id: number };
  bytes_received?: number; valid_frames?: number; crc_failures?: number; explanation: string;
};

export type FirmwareEnvironment = {
  cli_path?: string; cli_version?: string; uno_r4_core_version?: string; expected_fqbn: string;
  boards: Board[]; ready: boolean; problem?: string;
};

export type ArduinoRuntimeStatus = { ready: boolean; cli_version: string; core_version: string; message: string };

/** A busy operation shown in the operation modal. */
export type ActiveOperation = { title: string; stage: string; cancelable: boolean };

export type StartFailureRecord = {
  stage: string; code: string; userMessage: string; technicalDetail: string;
  timestamp: string; port: string; lab: string;
};

export type PlotErrorRecord = { timestamp: string; stage: string; detail: string };
