// @vitest-environment happy-dom
/**
 * Mounts the Acquisition page against a fake Tauri backend to check the
 * behaviour that depends on Svelte's legacy `$:` ordering, which the pure
 * helper tests cannot reach: the calibrations loaded after a lab change, the
 * setup controls while Faulted, the Storage tile, and the text under Start.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import type { CalibrationPreset } from '$lib/calibration';
import type { LabProfile } from '$lib/labs';

const backend = vi.hoisted(() => ({
  handlers: {} as Record<string, (args: Record<string, unknown>) => unknown>,
  calls: [] as Array<{ command: string; args: Record<string, unknown> }>,
  confirmAnswer: true,
  confirmMessages: [] as string[],
  confirmOptions: [] as unknown[],
  openCalls: 0,
  openResult: null as string | null
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (command: string, args: Record<string, unknown> = {}) => {
    backend.calls.push({ command, args });
    const handler = backend.handlers[command];
    if (!handler) throw new Error(`unexpected command ${command}`);
    return handler(args);
  }
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => undefined }));
// happy-dom has no canvas; the plots are not under test here.
vi.mock('uplot', () => ({
  default: class FakePlot {
    static pxRatio = 1;
    bbox = { left: 0, top: 0, width: 300, height: 200 };
    setData() {}
    setSize() {}
    destroy() {}
    valToPos() { return 0; }
  }
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({
  confirm: async (message: string, options?: unknown) => {
    backend.confirmOptions.push(options);
    backend.confirmMessages.push(message);
    return backend.confirmAnswer;
  },
  open: async () => {
    backend.openCalls += 1;
    return backend.openResult;
  },
  save: async () => null
}));

const profile = (file: string) =>
  JSON.parse(readFileSync(resolve(process.cwd(), 'profiles', file), 'utf8')) as LabProfile;
const general = profile('general_analog_development.profile.json');
const bp = profile('blood_pressure_ppg_course_capture.profile.json');

const preset = (lab: LabProfile, channelId: string, id: string, label: string): CalibrationPreset => ({
  schema_version: 1, calibration_id: id, profile_id: lab.profile_id, channel_id: channelId, calibration_type: 'linear',
  input_quantity: 'volts', output_quantity: 'force', output_units: 'N', parameters: { slope: 2, offset: 0 },
  created_at: '2026-11-18T00:00:00Z', label
});

const idleSession = (overrides: Record<string, unknown> = {}) => ({
  state: 'Disconnected', board: '', port: '', protocol_version: '0.4', simulator: false,
  samples: 0, packets: 0, measured_rate_hz: 0,
  integrity: {
    received_packets: 0, crc_failures: 0, invalid_frames: 0, unsupported_versions: 0,
    missing_packet_sequences: 0, duplicate_packets: 0, out_of_order_packets: 0,
    missing_sample_sequences: 0, duplicate_sample_sequences: 0, out_of_order_sample_sequences: 0,
    firmware_overflows: 0, host_channel_overflows: 0, reconnects: 0, disconnect_events: 0
  },
  // Rust sends missing values as null.
  duration: null, elapsed_seconds: 0, remaining_seconds: null, available_disk_bytes: null,
  storage_warning: null, stop_reason: null, connection_diagnostics: null, last_error: null,
  fault_origin: null, last_summary: null, profile: null,
  calibration: { adc_reference_v: 5, mpxv_sensor_supply_v: 5, channel_units: {}, active_calibrations: [] },
  digital_output_mask: null, display_origin_timestamp_us: null,
  // Left out unless a test sets it: the page then follows the state alone.
  recording_active: undefined as boolean | undefined,
  ...overrides
});

let session = idleSession();
let generalListDelay = 0;
let firmwareStatus: Record<string, unknown> = { compatibility: 'unknown', job: null, verified_port: null };

const board = { port: 'COM5', name: 'Arduino UNO R4 WiFi', fqbn: 'arduino:renesas_uno:unor4wifi', usb_vid: 0x2341, usb_pid: 0x006d };

function installBackend() {
  backend.calls = [];
  backend.confirmMessages = [];
  backend.confirmOptions = [];
  backend.confirmAnswer = true;
  backend.openCalls = 0;
  backend.openResult = null;
  session = idleSession();
  generalListDelay = 0;
  firmwareStatus = { compatibility: 'unknown', job: null, verified_port: null };
  backend.handlers = {
    get_project_folder: () => ({ project_folder: 'C:\\BMEG 420L', available: true, notice: null }),
    prepare_arduino_runtime: () => ({ ready: true, cli_version: '1.6.0', core_version: '1.6.0', message: '' }),
    firmware_environment: () => ({ ready: true, expected_fqbn: 'arduino:renesas_uno:unor4wifi', boards: [] }),
    list_boards: () => [],
    get_session_status: () => session,
    get_recent_display_data: () => [],
    get_firmware_workflow_status: () => firmwareStatus,
    list_acquisition_profiles: () => [general, bp],
    get_profile_mode: () => 'student',
    get_catalog_status: () => ({ writable: true, skipped_labs: [], calibration_issues: [] }),
    list_calibrations: async ({ profileId, channelId }) => {
      if (profileId === general.profile_id) {
        // The previous lab's answer can arrive after the new lab's.
        await new Promise((done) => setTimeout(done, generalListDelay));
        return [preset(general, 'general_a0', 'general.a0.1', 'General load cell')];
      }
      if (profileId === bp.profile_id && channelId === 'xgzp') return [preset(bp, 'xgzp', 'bp.xgzp.1', 'Team XGZP')];
      return [];
    },
    estimate_recording_storage: () => ({
      records_per_second: 1000, bmeg_bytes_per_second: 20_000, csv_bytes_per_second: 30_000,
      projected_bytes: 5_000_000, required_free_bytes: 300_000_000, critical_free_bytes: 262_144_000,
      available_free_bytes: 8 * 1024 ** 3
    }),
    delete_calibration: () => null,
    record_frontend_plot_error: () => null
  };
}

async function settle(ms = 20) {
  for (let round = 0; round < 6; round += 1) {
    await new Promise((done) => setTimeout(done, ms));
    flushSync();
  }
}

function labSelect(): HTMLSelectElement {
  const select = [...document.querySelectorAll('label')]
    .find((label) => label.textContent?.startsWith('Selected lab'))
    ?.querySelector('select');
  if (!select) throw new Error('lab select not found');
  return select;
}

function calibrationOptions(): string[] {
  const select = [...document.querySelectorAll('label')]
    .find((label) => /XGZP calibration|Channel calibration/.test(label.textContent ?? ''))
    ?.querySelector('select');
  return [...(select?.options ?? [])].map((option) => option.value);
}

function buttonNamed(text: string): HTMLButtonElement | undefined {
  return [...document.querySelectorAll<HTMLButtonElement>('button')].find((button) => button.textContent?.trim() === text);
}

function expanded(card: string): boolean {
  return document.getElementById(`${card}-toggle`)?.getAttribute('aria-expanded') === 'true';
}

function chooseLab(lab: LabProfile) {
  const select = labSelect();
  select.value = lab.profile_id;
  select.dispatchEvent(new Event('change', { bubbles: true }));
  flushSync();
}

let page: Record<string, unknown> | undefined;

async function renderPage() {
  const { default: Page } = await import('./+page.svelte');
  page = mount(Page, { target: document.body }) as Record<string, unknown>;
  await settle();
}

beforeEach(() => {
  vi.useRealTimers();
  installBackend();
  // uPlot needs layout APIs happy-dom does not provide; the page isolates plot errors.
  globalThis.ResizeObserver ??= class { observe() {} disconnect() {} unobserve() {} } as unknown as typeof ResizeObserver;
  window.matchMedia ??= (() => ({ matches: false, addEventListener() {}, removeEventListener() {} })) as unknown as typeof window.matchMedia;
});

afterEach(() => {
  if (page) unmount(page);
  page = undefined;
  document.body.innerHTML = '';
});

describe('Acquisition page against a fake backend', () => {
  it('loads the new lab’s calibrations after a lab change, never the previous lab’s', async () => {
    await renderPage();
    expect(calibrationOptions()).toEqual(['', 'general.a0.1']);

    generalListDelay = 80;
    backend.calls = [];
    chooseLab(bp);
    await settle(30);

    const requests = backend.calls.filter((call) => call.command === 'list_calibrations').map((call) => call.args);
    expect(requests).toEqual([{ profileId: bp.profile_id, channelId: 'xgzp' }]);
    expect(calibrationOptions()).toEqual(['', 'bp.xgzp.1']);
  });

  it('drops a slow answer for the previous lab that arrives after the lab changed', async () => {
    await renderPage();
    generalListDelay = 120;
    // Re-request General Analog's list (slow), then switch to BP before it answers.
    chooseLab(bp);
    chooseLab(general);
    chooseLab(bp);
    await settle(50);
    expect(calibrationOptions()).toEqual(['', 'bp.xgzp.1']);
    expect(labSelect().value).toBe(bp.profile_id);
  });

  it('keeps the setup usable while the session is Faulted', async () => {
    session = idleSession({ state: 'Faulted', last_error: 'verification failed', fault_origin: 'verification' });
    await renderPage();
    expect(labSelect().disabled).toBe(false);
    const source = [...document.querySelectorAll<HTMLInputElement>('input[name="recording-source"]')];
    expect(source).toHaveLength(2);
    expect(source.every((input) => !input.matches(':disabled'))).toBe(true);
    const browse = [...document.querySelectorAll('button')].find((button) => button.textContent === 'Browse');
    expect(browse?.disabled).toBe(false);
  });

  it('shows unknown free space as not yet checked, then the Project folder’s free space', async () => {
    backend.handlers.estimate_recording_storage = async () => {
      await new Promise((done) => setTimeout(done, 400));
      return {
        records_per_second: 1000, bmeg_bytes_per_second: 20_000, csv_bytes_per_second: 30_000,
        projected_bytes: 5_000_000, required_free_bytes: 300_000_000, critical_free_bytes: 262_144_000,
        available_free_bytes: 8 * 1024 ** 3
      };
    };
    await renderPage();
    const storage = () => [...document.querySelectorAll('.metric-grid span')]
      .find((span) => span.textContent?.startsWith('Storage'))?.textContent;
    expect(storage()).toBe('Storagenot yet checked');
    expect(document.body.textContent).not.toContain('Remaining');
    await settle(150);
    expect(storage()).toBe('Storage8.00 GiB free');
    const estimate = backend.calls.find((call) => call.command === 'estimate_recording_storage');
    expect(estimate?.args).toMatchObject({ request: { profile_id: general.profile_id, project_folder: 'C:\\BMEG 420L' } });
  });

  it('explains a Faulted session with one plain-language banner and a single live region', async () => {
    session = idleSession({
      state: 'Faulted', last_error: 'no samples for 7 s', fault_origin: 'recording',
      connection_diagnostics: { failure_category: 'no_data_timeout', recommended_action: 'Select Refresh Board, then Verify Firmware.' }
    });
    backend.handlers.disconnect_session = () => {
      session = idleSession({ last_error: 'no samples for 7 s' });
      return session;
    };
    await renderPage();
    const banners = document.querySelectorAll('[data-problem]');
    expect(banners).toHaveLength(1);
    expect(banners[0].textContent).toContain('The Arduino stopped sending samples');
    expect(banners[0].textContent).not.toContain('no_data_timeout');
    expect(document.querySelector('#message-action-button')?.textContent?.trim()).toBe('Verify Firmware');
    expect(document.querySelectorAll('[aria-live]')).toHaveLength(1);
    expect(document.querySelectorAll('[role="status"]')).toHaveLength(0);
    expect(document.querySelector('[data-testid="recording-state"]')?.textContent).toBe('Needs attention');

    // The retired generic texts are gone; Clear connection replaces Disconnect.
    expect(document.body.textContent).not.toContain('Connection needs attention');
    expect(buttonNamed('Disconnect')).toBeUndefined();
    buttonNamed('Clear connection')?.click();
    await settle(60);
    expect(backend.calls.some((call) => call.command === 'disconnect_session')).toBe(true);
    // Disconnected with a stale last_error: no banner is left behind.
    expect(document.querySelectorAll('[data-problem]')).toHaveLength(0);
  });

  it('starts Calibration & Units collapsed in Student mode and collapses setup while recording', async () => {
    await renderPage();
    expect(expanded('board')).toBe(true);
    expect(expanded('calibration')).toBe(false);
    document.getElementById('calibration-toggle')?.click();
    flushSync();
    expect(expanded('calibration')).toBe(true);

    session = idleSession({
      state: 'Acquiring', simulator: true, samples: 500, measured_rate_hz: 99.96,
      duration: { mode: 'timed', seconds: 60 }, elapsed_seconds: 5, remaining_seconds: 55,
      display_origin_timestamp_us: 1_000_000, profile: { bench_notice_acknowledged: false, profile: general }
    });
    await settle(80);
    for (const card of ['board', 'project', 'lab', 'session', 'duration', 'calibration']) expect(expanded(card), card).toBe(false);
    expect(buttonNamed('Stop recording')).toBeDefined();
    expect(document.body.textContent).toContain('SIMULATED');
    expect(document.body.textContent).toContain('100.0 Hz');
    expect(document.querySelector('[data-testid="recording-state"]')?.textContent).toBe('Recording');

    session = idleSession();
    await settle(80);
    expect(expanded('board')).toBe(true);
    expect(expanded('calibration')).toBe(true);
    expect(buttonNamed('Start recording')).toBeDefined();
  });

  it('adds a labelled marker and reports it in elapsed seconds', async () => {
    session = idleSession({
      state: 'Acquiring', simulator: true, duration: { mode: 'until_stopped' }, elapsed_seconds: 12,
      display_origin_timestamp_us: 35_913_377, profile: { bench_notice_acknowledged: false, profile: general }
    });
    backend.handlers.add_recording_marker = ({ label }) => ({ timestamp_us: 48_213_377, label });
    await renderPage();
    const input = document.querySelector<HTMLInputElement>('#marker-label-input')!;
    input.value = 'baseline';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    buttonNamed('Add marker')?.click();
    await settle();
    expect(backend.calls.find((call) => call.command === 'add_recording_marker')?.args).toEqual({ label: 'baseline' });
    expect(document.querySelector('[aria-live]')?.textContent).toBe('Marker “baseline” added at 12.3 s.');
  });

  it('takes the student from a failing readiness item to the control that fixes it', async () => {
    backend.handlers.get_project_folder = () => ({ project_folder: '', available: true, notice: null });
    await renderPage();
    document.getElementById('project-toggle')?.click();
    flushSync();
    expect(expanded('project')).toBe(false);
    const folderItem = [...document.querySelectorAll<HTMLButtonElement>('.readiness button')].find((button) => button.textContent?.includes('Folder'));
    expect(folderItem?.textContent).toContain('choose a Project folder');
    folderItem?.click();
    await settle();
    expect(expanded('project')).toBe(true);
    expect(document.activeElement?.id).toBe('project-folder-browse');
  });

  it('opens the calibration dialog as a real modal that Escape closes, returning focus', async () => {
    await renderPage();
    document.getElementById('calibration-toggle')?.click();
    flushSync();
    const opener = buttonNamed('Calibrate channel')!;
    opener.focus();
    opener.click();
    flushSync();
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]');
    expect(dialog?.getAttribute('aria-modal')).toBe('true');
    expect(document.getElementById(dialog?.getAttribute('aria-labelledby') ?? '')?.textContent).toMatch(/^Calibrate /);
    expect(document.activeElement).toBe(dialog);
    dialog?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    await settle();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(document.activeElement).toBe(opener);

    // The Close button inside the moved dialog still reaches the page.
    opener.click();
    flushSync();
    [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find((button) => button.textContent === 'Close')?.click();
    flushSync();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });

  it('opens Calibration & Units in Instructor mode and shows Lab Manager as a modal dialog', async () => {
    backend.handlers.get_profile_mode = () => 'instructor_authoring';
    backend.handlers.list_instructor_labs = () => [{ profile: general, active: true, retired: false }];
    await renderPage();
    expect(expanded('calibration')).toBe(true);
    // The acknowledgement comes before the Instructor choice.
    const acknowledgement = document.querySelector('input[type="checkbox"][aria-describedby="instructor-mode-explanation"]');
    const instructor = document.querySelector('input[value="instructor_authoring"]');
    expect(acknowledgement && instructor && acknowledgement.compareDocumentPosition(instructor) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    const manage = buttonNamed('Manage Labs')!;
    manage.focus();
    manage.click();
    await settle();
    const dialog = document.querySelector<HTMLElement>('[role="dialog"][aria-labelledby="lab-manager-title"]');
    expect(dialog?.getAttribute('aria-modal')).toBe('true');
    expect(dialog?.textContent).toContain('General Analog');
    dialog?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    await settle();
    expect(document.querySelector('[aria-labelledby="lab-manager-title"]')).toBeNull();
    expect(document.activeElement).toBe(manage);
  });

  it('shows a lab catalog migration notice in the Lab card in Student mode, not as a problem', async () => {
    const notice = 'The Op Amp Lab is now built in. The older Op Amp Lab copy that replaced General Analog was retired; a backup was saved to C:\\Users\\Student\\AppData\\Local\\WVU Bioinstrumentation Studio\\profiles\\lab_state.pre-migration-20261118T120000.000Z.json.';
    backend.handlers.get_catalog_status = () => ({ writable: true, skipped_labs: [], migration_notices: [notice], calibration_issues: [] });
    await renderPage();
    const shown = document.querySelector('#lab-card .migration-notice');
    expect(shown?.textContent).toBe(notice);
    expect(expanded('lab')).toBe(true);
    expect(document.querySelectorAll('[data-problem]')).toHaveLength(0);
    expect(document.body.textContent).not.toContain('Some labs could not be loaded');
  });

  it('uses the new wording and keeps the teaching-use notice', async () => {
    await renderPage();
    const text = document.body.textContent ?? '';
    expect(document.querySelector('header img')?.getAttribute('alt')).toBe('West Virginia University Chemical and Biomedical Engineering');
    expect(document.querySelector('#live-title')?.textContent).toBe('Live signals');
    expect(text).toContain('Teaching use only — not a medical device.');
    for (const retired of ['Bounded live', 'Connect, configure, and start recording', 'Elapsed host', 'Frame / cycle rate', 'buffered rail samples']) {
      expect(text, retired).not.toContain(retired);
    }
    expect(buttonNamed('Start recording')).toBeDefined();
  });

  it('never sends an acquisition command for display and layout changes', async () => {
    await renderPage();
    backend.calls = [];
    document.getElementById('board-toggle')?.click();
    document.querySelector<HTMLElement>('.display-options > summary')?.click();
    flushSync();
    buttonNamed('One plot per signal')?.click();
    buttonNamed('Overlay all')?.click();
    const trace = document.querySelector<HTMLInputElement>('.trace-controls input[type="checkbox"]');
    trace?.click();
    await settle();
    const acquisition = ['start_profile_simulator_recording', 'start_profile_hardware_recording', 'stop_recording', 'disconnect_session', 'add_recording_marker', 'retry_hardware_handshake'];
    expect(backend.calls.filter((call) => acquisition.includes(call.command))).toEqual([]);
  });

  it('asks before deleting a saved calibration and names it', async () => {
    await renderPage();
    const select = [...document.querySelectorAll('label')]
      .find((label) => /Channel calibration/.test(label.textContent ?? ''))
      ?.querySelector('select');
    if (!select) throw new Error('calibration select not found');
    select.value = 'general.a0.1';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
    backend.confirmAnswer = false;
    const deleteButton = [...document.querySelectorAll('button')].find((button) => button.textContent === 'Delete calibration');
    deleteButton?.click();
    await settle();
    expect(backend.confirmMessages[0]).toContain('General load cell');
    expect(backend.calls.some((call) => call.command === 'delete_calibration')).toBe(false);
  });
});

const ACQUISITION_COMMANDS = ['start_profile_simulator_recording', 'start_profile_hardware_recording', 'stop_recording', 'disconnect_session', 'add_recording_marker', 'retry_hardware_handshake'];

/**
 * One UNO R4 WiFi on COM5; Verify Firmware proves (or disproves) the WVU
 * firmware. Like the backend (`retry_handshake` with the verification
 * origin), the read-only handshake clears the previous fault, names the
 * board's port and board (but keeps `simulator`, which describes the last
 * recording), and passes the session through Connecting and Connected with
 * `recording_active: false`: a board check, not a recording. A compatible
 * board ends Disconnected. A
 * board that does not answer leaves the session Faulted (verification,
 * port_open_no_bytes) and the firmware status verification_failed. A busy
 * port is never opened: the session faults with port_busy, the command
 * fails, and the board's firmware status is left as it was.
 */
function withBoard(compatible: boolean, checkMs = 25, failure: 'port_open_no_bytes' | 'port_busy' = 'port_open_no_bytes') {
  backend.handlers.list_boards = () => [board];
  backend.handlers.verify_wvu_reference_firmware = async ({ port }) => {
    const before = { ...session, last_error: null, fault_origin: null, port: port as string, board: 'Arduino UNO R4 WiFi' };
    const pause = () => new Promise((done) => setTimeout(done, checkMs));
    const faulted = (category: string, lastError: string) => idleSession({
      ...before, state: 'Faulted', recording_active: false, fault_origin: 'verification', last_error: lastError,
      connection_diagnostics: { failure_category: category, recommended_action: 'Backend text.' }
    });
    session = { ...before, state: 'Connecting', recording_active: false };
    await pause();
    if (!compatible && failure === 'port_busy') {
      const busy = {
        category: 'port_busy', stage: 'verifying_firmware', title: 'Serial port is busy',
        explanation: `${port} could not be opened. Another program is probably using it, so the firmware was not checked.`,
        recommended_action: 'Close Arduino IDE, Serial Monitor, or Arduino Cloud Agent, then select Verify Firmware again.',
        technical_details: 'Access is denied.'
      };
      session = faulted('port_busy', `could not open ${port}: Access is denied.`);
      firmwareStatus = { ...firmwareStatus, last_failure: busy };
      throw busy;
    }
    session = { ...before, state: 'Connected', recording_active: false };
    await pause();
    session = compatible
      ? { ...before, state: 'Disconnected', recording_active: false, connection_diagnostics: null }
      : faulted('port_open_no_bytes', 'firmware did not respond');
    firmwareStatus = compatible
      ? { compatibility: 'wvu_protocol_compatible', job: null, verified_port: port, last_failure: null }
      : {
          compatibility: 'verification_failed', job: null, verified_port: null,
          last_failure: { category: 'protocol_verification_failed', stage: 'verifying_firmware', title: 'Firmware verification failed', explanation: 'The board did not answer with the WVU protocol.', recommended_action: 'Restore.', technical_details: 'no HELLO' }
        };
    return { declared_kind: 'wvu_protocol_reference', compatible, explanation: compatible ? 'compatible' : 'no reply' };
  };
}

function sourceChecked(): string | undefined {
  return document.querySelector<HTMLInputElement>('input[name="recording-source"]:checked')?.value;
}

function infoLine(): string {
  return document.querySelector('[aria-live]')?.textContent ?? '';
}

describe('Acquisition page: board, firmware, plots, and the finished recording', () => {
  const faulted = (origin: string, category?: string) => idleSession({
    state: 'Faulted', last_error: 'failed', fault_origin: origin,
    connection_diagnostics: category ? { failure_category: category, recommended_action: 'Backend text.' } : null
  });

  // `afterRender` runs once the startup board check (which replaces any earlier fault) is done.
  it.each([
    {
      name: 'Verify Firmware',
      setup: () => withBoard(false),
      afterRender: () => { session = faulted('recording', 'no_data_timeout'); },
      expectCall: { command: 'verify_wvu_reference_firmware', args: { port: 'COM5' } }
    },
    {
      name: 'Retry connection',
      setup: () => {
        withBoard(false);
        backend.handlers.retry_hardware_handshake = () => ({ handshake_succeeded: true, diagnostics: { recommended_action: '' } });
      },
      afterRender: () => { session = faulted('connection', 'port_busy'); },
      chooseHardware: true,
      expectCall: { command: 'retry_hardware_handshake', args: { port: 'COM5' } }
    },
    {
      name: 'Restore WVU Firmware',
      setup: () => {
        withBoard(false);
        backend.handlers.restore_wvu_reference_firmware = () => ({ id: 1, kind: 'restore_reference', stage: 'preparing', active: true, message: 'Preparing firmware operation.' });
      },
      chooseHardware: true,
      expectCall: { command: 'restore_wvu_reference_firmware', args: { request: { port: 'COM5', confirmation: true } } }
    },
    {
      name: 'Refresh Board',
      setup: () => { session = faulted('recording', 'device_disconnected'); },
      expectCall: { command: 'list_boards', args: {} }
    },
    {
      name: 'Clear connection',
      setup: () => {
        session = faulted('storage', 'recording_storage_error');
        backend.handlers.disconnect_session = () => (session = idleSession());
      },
      expectCall: { command: 'disconnect_session', args: {} }
    },
    {
      name: 'Repair Arduino tools',
      setup: () => {
        backend.handlers.prepare_arduino_runtime = () => { throw { title: 'Arduino tools need repair', explanation: 'A tool file is damaged.', recommended_action: 'Select Repair Arduino tools.', repairable: true }; };
        backend.handlers.repair_arduino_runtime = () => ({ ready: true, cli_version: '1.6.0', core_version: '1.6.0', message: '' });
      },
      chooseHardware: true,
      expectCall: { command: 'repair_arduino_runtime', args: {} }
    }
  ])('the banner action $name runs its handler', async ({ name, setup, afterRender, chooseHardware, expectCall }) => {
    setup();
    await renderPage();
    if (afterRender) {
      afterRender();
      // The next idle status read (every 250 ms) picks it up.
      await settle(60);
    }
    if (chooseHardware) {
      document.querySelector<HTMLInputElement>('input[value="hardware"]')?.click();
      await settle();
    }
    const action = document.querySelector<HTMLButtonElement>('#message-action-button');
    expect(action?.textContent?.trim()).toBe(name);
    expect(action?.disabled).toBe(false);
    backend.calls = [];
    action?.click();
    await settle(30);
    expect(backend.calls.find((call) => call.command === expectCall.command)?.args).toEqual(expectCall.args);
  });

  it('the banner action Choose another folder opens the folder picker', async () => {
    backend.handlers.get_project_folder = () => ({ project_folder: 'E:\\BMEG', available: false, notice: 'The Project folder E:\\BMEG is not available.' });
    await renderPage();
    const action = document.querySelector<HTMLButtonElement>('#message-action-button');
    expect(action?.textContent?.trim()).toBe('Choose another folder');
    action?.click();
    await settle();
    expect(backend.openCalls).toBe(1);
  });

  it('switches Source to Hardware once when the firmware is first verified, never after the student chose', async () => {
    withBoard(true);
    await renderPage();
    expect(sourceChecked()).toBe('hardware');
    expect(infoLine()).toContain('Source is now Hardware · COM5');
    expect(document.querySelector('[data-testid="board-state"]')?.textContent).toContain('Ready on COM5');
    // Recovery stays reachable when everything is ready.
    expect(buttonNamed('Verify Firmware')?.disabled).toBe(false);
    expect(buttonNamed('Restore WVU Firmware')?.disabled).toBe(false);

    // The student picks Simulator; losing and regaining readiness changes nothing.
    document.querySelector<HTMLInputElement>('input[value="simulator"]')?.click();
    await settle();
    firmwareStatus = { compatibility: 'unknown', job: null, verified_port: null };
    await settle(150);
    expect(document.querySelector('[data-testid="board-state"]')?.textContent).toContain('Not checked');
    buttonNamed('Verify Firmware')?.click();
    await settle(150);
    expect(document.querySelector('[data-testid="board-state"]')?.textContent).toContain('Ready on COM5');
    expect(sourceChecked()).toBe('simulator');
  });

  it('never switches Source to Hardware after a recording has run', async () => {
    await renderPage();
    session = idleSession({ state: 'Acquiring', simulator: true, duration: { mode: 'until_stopped' }, profile: { bench_notice_acknowledged: false, profile: general } });
    await settle(80);
    expect(buttonNamed('Stop recording')).toBeDefined();
    session = idleSession();
    await settle(80);
    withBoard(true);
    buttonNamed('Refresh Board')?.click();
    await settle(60);
    expect(document.querySelector('[data-testid="board-state"]')?.textContent).toContain('Ready on COM5');
    expect(sourceChecked()).toBe('simulator');
  });

  it('pauses the plots without pausing the recording, and Resume jumps to the newest data', async () => {
    session = idleSession({ state: 'Acquiring', simulator: true, duration: { mode: 'until_stopped' }, display_origin_timestamp_us: 1_000_000, profile: { bench_notice_acknowledged: false, profile: general } });
    await renderPage();
    expect(document.querySelectorAll('.sim-watermark').length).toBeGreaterThan(0);
    expect(document.querySelector('.sim-watermark')?.textContent).toBe('SIMULATED DATA');
    const pause = document.querySelector<HTMLButtonElement>('#plot-pause-button')!;
    expect(pause.textContent).toContain('Pause display');
    pause.click();
    flushSync();
    expect(pause.textContent).toContain('Resume display');
    expect(document.querySelector('.paused-badge')?.textContent).toBe('Paused — recording continues');
    backend.calls = [];
    await settle(40);
    expect(backend.calls.filter((call) => call.command === 'get_session_status').length).toBeGreaterThan(0);
    expect(backend.calls.filter((call) => call.command === 'get_recent_display_data')).toEqual([]);

    pause.click();
    await settle(30);
    expect(document.querySelector('.paused-badge')).toBeNull();
    expect(backend.calls.filter((call) => call.command === 'get_recent_display_data').length).toBeGreaterThan(0);
    expect(backend.calls.filter((call) => ACQUISITION_COMMANDS.includes(call.command))).toEqual([]);
  });

  it('changes the plot time window with a chip, as a display-only query', async () => {
    await renderPage();
    backend.calls = [];
    const chip = buttonNamed('10 s')!;
    chip.click();
    await settle();
    expect(chip.getAttribute('aria-pressed')).toBe('true');
    expect(buttonNamed('5 s')?.getAttribute('aria-pressed')).toBe('false');
    expect(document.querySelector<HTMLInputElement>('#plot-time-window')?.value).toBe('10');
    expect(backend.calls.find((call) => call.command === 'get_recent_display_data')?.args).toMatchObject({ windowSeconds: 10 });
    expect(backend.calls.filter((call) => ACQUISITION_COMMANDS.includes(call.command))).toEqual([]);
  });

  it('draws a labelled marker line on the plots at its elapsed time', async () => {
    const origin = 35_913_377;
    session = idleSession({ state: 'Acquiring', simulator: true, duration: { mode: 'until_stopped' }, display_origin_timestamp_us: origin, profile: { bench_notice_acknowledged: false, profile: general } });
    backend.handlers.get_recent_display_data = () => Array.from({ length: 31 }, (_, index) => ({
      sequence: index, timestamp_us: origin + 10_000_000 + index * 100_000, values: [100, 200, 300, 400, 500, 600, 700, 800], status_flags: 0
    }));
    backend.handlers.add_recording_marker = ({ label }) => ({ timestamp_us: origin + 12_300_000, label });
    await renderPage();
    const input = document.querySelector<HTMLInputElement>('#marker-label-input')!;
    input.value = 'baseline';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    buttonNamed('Add marker')?.click();
    await settle();
    const lines = [...document.querySelectorAll('.marker-line')];
    expect(lines.length).toBeGreaterThan(0);
    expect(lines.every((line) => line.querySelector('.marker-text')?.textContent === 'baseline')).toBe(true);
    expect(document.querySelector('.plot[role="img"]')?.getAttribute('aria-label')).toBe('Live signal plot, 1 marker');
  });

  it('shows the finished recording with file checks, and Show in folder asks the backend', async () => {
    session = idleSession({
      last_summary: {
        state: 'Disconnected', samples: 6000, packets: 600, measured_rate_hz: 100, board_elapsed_seconds: 60, host_elapsed_seconds: 60.2,
        bmeg_path: 'C:\\BMEG 420L\\run.bmeg', csv_path: '', metadata_path: 'C:\\BMEG 420L\\run.metadata.json',
        recording_status: 'complete', duration: { mode: 'timed', seconds: 60 }, stop_reason: 'timed_complete', completion_status: 'complete',
        integrity: idleSession().integrity, csv_error: 'could not write the CSV'
      }
    });
    backend.handlers.reveal_recording_file = () => null;
    await renderPage();
    const card = document.querySelector('[aria-labelledby="result-title"]')!;
    expect(card.textContent).toContain('Recording complete.');
    expect(card.textContent).toContain('Not saved');
    expect(document.body.textContent).not.toContain('Finalized files');
    buttonNamed('Show in folder')?.click();
    await settle();
    expect(backend.calls.find((call) => call.command === 'reveal_recording_file')?.args).toEqual({ path: 'C:\\BMEG 420L\\run.bmeg' });
    expect(infoLine()).toBe('File Explorer opened with the file selected.');
  });

  it('shows the restore as steps; Escape never cancels it and the upload has no Cancel', async () => {
    const job = {
      id: 3, kind: 'restore_reference', stage: 'waiting_for_bootloader', active: true, message: 'Press the board Reset button twice.',
      original_port: 'COM5', manual_reset_deadline_utc: new Date(Date.now() + 90_000).toISOString()
    };
    firmwareStatus = { compatibility: 'upload_in_progress', job, verified_port: null };
    backend.handlers.cancel_firmware_job = () => firmwareStatus;
    await renderPage();
    const dialog = document.querySelector<HTMLElement>('[role="dialog"][aria-labelledby="operation-title"]')!;
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(dialog.textContent).toContain('Press the RESET button twice quickly');
    expect(dialog.querySelector('[aria-current="step"]')?.textContent).toContain('Switch board to upload mode');
    expect(document.querySelector('[data-testid="board-state"]')?.textContent).toContain('Restoring firmware…');

    dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    await settle();
    expect(document.querySelector('[role="dialog"][aria-labelledby="operation-title"]')).not.toBeNull();
    expect(backend.calls.some((call) => call.command === 'cancel_firmware_job')).toBe(false);

    firmwareStatus = { compatibility: 'upload_in_progress', job: { ...job, stage: 'uploading', manual_reset_deadline_utc: null, message: 'Writing the WVU firmware to the board.' }, verified_port: null };
    await settle(150);
    const writing = document.querySelector('[role="dialog"][aria-labelledby="operation-title"]')!;
    expect(writing.textContent).toContain('Writing firmware — do not unplug the board');
    expect([...writing.querySelectorAll('button')].some((button) => button.textContent?.includes('Cancel'))).toBe(false);
    expect(infoLine()).toBe('Restoring WVU Firmware, step 4 of 6: Upload.');
  });
});

describe('Acquisition page: board checks, Start feedback, and the recording layout', () => {
  it('treats a Verify handshake as a board check, never as a recording', async () => {
    withBoard(true, 400);
    const page = renderPage();
    const chip = () => document.querySelector('[data-testid="recording-state"]')?.textContent;
    // The startup Verify is running: Connecting/Connected, but not a recording.
    for (let round = 0; round < 40 && chip() !== 'Checking board…'; round += 1) await new Promise((done) => setTimeout(done, 20));
    expect(backend.calls.some((call) => call.command === 'verify_wvu_reference_firmware')).toBe(true);
    expect(['Connecting', 'Connected']).toContain(session.state);
    expect(expanded('board')).toBe(true);
    expect(buttonNamed('Stop recording')).toBeUndefined();
    expect(document.querySelector('[data-testid="recording-state"]')?.textContent).toBe('Checking board…');
    await page;
    await settle(150);
    // The decided auto-select still happens after the startup Verify.
    expect(sourceChecked()).toBe('hardware');

    // A later Verify keeps the setup open and the display settings as they were.
    const trace = document.querySelector<HTMLInputElement>('.trace-controls input[type="checkbox"]')!;
    trace.click();
    flushSync();
    expect(trace.checked).toBe(false);
    document.getElementById('calibration-toggle')?.click();
    flushSync();
    buttonNamed('Verify Firmware')?.click();
    for (let round = 0; round < 40 && chip() !== 'Checking board…'; round += 1) await new Promise((done) => setTimeout(done, 20));
    expect(session.state).toMatch(/Connecting|Connected/);
    expect(chip()).toBe('Checking board…');
    for (const card of ['board', 'project', 'lab', 'session', 'duration', 'calibration']) expect(expanded(card), card).toBe(true);
    await settle(150);
    expect(document.querySelector<HTMLInputElement>('.trace-controls input[type="checkbox"]')?.checked).toBe(false);
    expect(document.getElementById('live-panel')?.compareDocumentPosition(document.getElementById('setup-title')!)).toBe(Node.DOCUMENT_POSITION_PRECEDING);
  });

  it('never flashes "firmware not checked" while a newly found board is being verified', async () => {
    await renderPage();
    document.querySelector<HTMLInputElement>('input[value="hardware"]')?.click();
    await settle();
    expect(document.querySelector('[data-problem]')?.getAttribute('data-problem')).toBe('board:none');
    const seen = new Set<string>();
    const observer = new MutationObserver(() => {
      document.querySelectorAll('[data-problem]').forEach((banner) => seen.add(banner.getAttribute('data-problem') ?? ''));
    });
    observer.observe(document.body, { subtree: true, childList: true, attributes: true });
    withBoard(false, 40);
    buttonNamed('Refresh Board')?.click();
    await settle(60);
    observer.disconnect();
    expect([...seen].filter((id) => id.startsWith('board:unchecked'))).toEqual([]);
    // Once checked, the banner and the Board card agree.
    expect(document.querySelector('[data-problem]')?.textContent).toContain('No WVU firmware detected on COM5');
    expect(document.querySelector('[data-testid="board-state"]')?.textContent).toContain('No WVU firmware');
    expect(document.querySelector('#message-action-button')?.textContent?.trim()).toBe('Restore WVU Firmware');
    expect(document.querySelector('#message-secondary-button')?.textContent?.trim()).toBe('Verify again');
  });

  it('names a board that never answered Verify the same way in the banner and the Board card', async () => {
    // The startup Verify leaves the session Faulted (verification, port_open_no_bytes), as the backend does.
    withBoard(false);
    await renderPage();
    expect(session).toMatchObject({ state: 'Faulted', fault_origin: 'verification', connection_diagnostics: { failure_category: 'port_open_no_bytes' } });
    // Simulator practice: the board problem stays in the Board card.
    expect(document.querySelector('[data-problem]')).toBeNull();
    expect(document.querySelector('[data-testid="recording-state"]')?.textContent).toBe('Ready');
    expect(document.querySelector('[data-testid="board-state"]')?.textContent).toContain('No WVU firmware');
    document.querySelector<HTMLInputElement>('input[value="hardware"]')?.click();
    await settle();
    const banner = document.querySelector('[data-problem]');
    expect(banner?.textContent).toContain('No WVU firmware detected on COM5');
    expect(banner?.textContent).not.toMatch(/retry/i);
    expect(document.querySelector('#message-action-button')?.textContent?.trim()).toBe('Restore WVU Firmware');
    // The readiness item names the same button it moves focus to.
    const sourceItem = [...document.querySelectorAll('.readiness li')].find((item) => item.textContent?.includes('Source'));
    expect(sourceItem?.textContent).toContain('select Restore WVU Firmware first');
    sourceItem?.querySelector('button')?.click();
    await settle();
    expect(document.activeElement?.id).toBe('message-action-button');
  });

  it('names a port that Verify found busy the same way in the banner and the Board card', async () => {
    withBoard(false, 25, 'port_busy');
    await renderPage();
    expect(session).toMatchObject({ state: 'Faulted', fault_origin: 'verification', connection_diagnostics: { failure_category: 'port_busy' } });
    const boardState = () => document.querySelector('[data-testid="board-state"]')?.textContent;
    // Simulator practice: the chip stays Ready and only the Board card explains.
    expect(document.querySelector('[data-problem]')).toBeNull();
    expect(document.querySelector('[data-testid="recording-state"]')?.textContent).toBe('Ready');
    expect(boardState()).toContain('Port busy');
    document.querySelector<HTMLInputElement>('input[value="hardware"]')?.click();
    await settle();
    const banner = document.querySelector('[data-problem]');
    expect(banner?.textContent).toContain('Another program is using the board');
    expect(banner?.textContent).toContain('then verify the firmware again');
    expect(banner?.textContent).not.toMatch(/retry/i);
    expect(document.querySelector('#message-action-button')?.textContent?.trim()).toBe('Verify Firmware');
    expect(boardState()).toContain('Port busy');
  });

  it('reports a failed Verify after a Simulator recording as a board problem, not as that recording', async () => {
    // The last recording ran on the simulator; its frames are still shown.
    session = idleSession({ simulator: true, board: 'Simulator', port: 'SIM' });
    withBoard(false);
    await renderPage();
    expect(session).toMatchObject({ state: 'Faulted', simulator: true, port: 'COM5', fault_origin: 'verification' });
    // Simulator practice: the board problem stays in the Board card.
    expect(document.querySelector('[data-problem]')).toBeNull();
    expect(document.querySelector('[data-testid="recording-state"]')?.textContent).toBe('Ready');
    expect(document.body.textContent).not.toMatch(/simulator recording (stopped|could not start)/i);
    expect(document.querySelector('[data-testid="board-state"]')?.textContent).toContain('No WVU firmware');
    document.querySelector<HTMLInputElement>('input[value="hardware"]')?.click();
    await settle();
    const banner = document.querySelector('[data-problem]');
    expect(banner?.textContent).toContain('No WVU firmware detected on COM5');
    expect(banner?.textContent).not.toMatch(/simulator/i);
    expect(document.querySelector('#message-action-button')?.textContent?.trim()).toBe('Restore WVU Firmware');
  });

  it('says a recording the drive refused before START could not start, not that it was not saved', async () => {
    withBoard(true);
    await renderPage();
    expect(sourceChecked()).toBe('hardware');
    // The worker refused before START: no recording ran, so no summary.
    session = idleSession({
      state: 'Faulted', fault_origin: 'storage', port: 'COM5', last_summary: null,
      last_error: 'storage: only 200 MiB free in C:\\BMEG 420L; recording requires at least 250 MiB',
      connection_diagnostics: { failure_category: 'recording_storage_error', recommended_action: 'Backend text.' }
    });
    await settle(60);
    const banner = document.querySelector('[data-problem]');
    expect(banner?.textContent).toContain('The recording could not start');
    expect(banner?.textContent).toContain('Only 200 MiB free in C:\\BMEG 420L; recording requires at least 250 MiB.');
    expect(document.body.textContent).not.toMatch(/not saved completely|Files already saved/);
    expect(document.querySelector('#message-action-button')?.textContent?.trim()).toBe('Clear connection');
  });

  it('shows the D4 output a stored legacy lab still drives while recording', async () => {
    // A 1.0.2 revision that unchecked D4 kept `led_outputs` and has no `digital_outputs`.
    const legacy = structuredClone(bp);
    delete legacy.acquisition.digital_outputs;
    backend.handlers.list_acquisition_profiles = () => [general, legacy];
    session = idleSession({
      state: 'Acquiring', recording_active: true, simulator: true, duration: { mode: 'until_stopped' },
      digital_output_mask: 1, display_origin_timestamp_us: 1_000_000, profile: { bench_notice_acknowledged: false, profile: legacy }
    });
    await renderPage();
    const outputs = [...document.querySelectorAll('.metric-grid span')].find((span) => span.textContent?.startsWith('Lab outputs'));
    expect(outputs?.textContent).toContain('D4 Green LED — HIGH');
  });

  it('keeps a failed restore on the Board card instead of verifying the board that appeared', async () => {
    const job = { id: 4, kind: 'restore_reference', stage: 'uploading', active: true, message: 'Writing the WVU firmware to the board.', original_port: 'COM34' };
    firmwareStatus = { compatibility: 'upload_in_progress', job, verified_port: null };
    await renderPage();
    // The upload failed; the board is now listed on another port, in upload mode.
    const failure = {
      category: 'upload_failed', stage: 'uploading', title: 'Upload failed', explanation: 'No device found on COM9.',
      recommended_action: 'Press RESET twice.', technical_details: 'bossac: No device found on COM9'
    };
    const finished = { ...job, stage: 'failed', active: false, failure, log_path: 'C:\\logs\\firmware_job_4.json', completed_utc: '2026-11-18T10:00:00Z' };
    backend.handlers.list_boards = () => [{ ...board, port: 'COM9', usb_pid: 0x1002 }];
    backend.handlers.verify_wvu_reference_firmware = () => { throw new Error('the board must not be verified now'); };
    firmwareStatus = { compatibility: 'verification_failed', job: null, last_restore: finished, last_failure: failure, verified_port: null };
    for (let round = 0; round < 40 && !backend.calls.some((call) => call.command === 'list_boards' && round > 0); round += 1) await settle(50);
    await settle(50);
    expect(backend.calls.filter((call) => call.command === 'list_boards').length).toBeGreaterThan(1);
    expect(backend.calls.some((call) => call.command === 'verify_wvu_reference_firmware')).toBe(false);
    const card = document.querySelector('[aria-labelledby="last-firmware-operation-title"]');
    expect(card?.textContent).toContain('Restore failed');
    expect(card?.textContent).toContain('C:\\logs\\firmware_job_4.json');
    // Recovery stays available.
    expect(buttonNamed('Restore WVU Firmware')?.disabled).toBe(false);
    expect(buttonNamed('Verify Firmware')?.disabled).toBe(false);
  });

  it('keeps Restore available when a failed Repair kept the Arduino tools already in use', async () => {
    backend.handlers.list_boards = () => [board];
    // The last restore could not compile; Verify is not part of this test.
    backend.handlers.verify_wvu_reference_firmware = () => { throw new Error('not checked'); };
    firmwareStatus = {
      compatibility: 'verification_failed', job: null, verified_port: null,
      last_failure: { category: 'compile_failed', stage: 'compiling', title: 'Compile failed', explanation: 'The sketch did not compile.', recommended_action: 'Repair.', technical_details: 'gcc' }
    };
    backend.handlers.repair_arduino_runtime = () => {
      throw {
        title: 'Arduino tools could not be prepared', explanation: 'Windows could not write the Arduino tools to this computer.',
        recommended_action: 'Close any other copy, then select Repair Arduino tools.', technical_details: 'Access is denied. (os error 5)',
        repairable: true, previous_runtime_kept: true
      };
    };
    await renderPage();
    await settle(60);
    expect(buttonNamed('Restore WVU Firmware')?.disabled).toBe(false);
    const repair = buttonNamed('Repair Arduino tools');
    expect(repair?.disabled).toBe(false);
    repair?.click();
    await settle();
    expect(backend.calls.some((call) => call.command === 'repair_arduino_runtime')).toBe(true);
    expect(infoLine()).toContain('the tools already in place were kept');
    expect(document.querySelector('.tools-warning')).toBeNull();
    expect(buttonNamed('Restore WVU Firmware')?.disabled).toBe(false);
  });

  it('shows an Arduino-tools problem once: in the Board card for Simulator practice, in the banner for Hardware', async () => {
    const problem = 'The Arduino tools included with the application were not found.';
    backend.handlers.prepare_arduino_runtime = () => { throw { title: 'Arduino tools missing', explanation: problem, recommended_action: 'Reinstall.', repairable: false }; };
    await renderPage();
    // Outside Advanced details (raw diagnostics), the text appears once.
    const occurrences = () => {
      const copy = document.body.cloneNode(true) as HTMLElement;
      copy.querySelectorAll('details.advanced-details').forEach((details) => details.remove());
      return (copy.textContent?.split('were not found').length ?? 1) - 1;
    };
    expect(document.querySelector('[data-problem]')).toBeNull();
    expect(document.querySelectorAll('.tools-warning')).toHaveLength(1);
    expect(infoLine()).not.toContain('were not found');
    expect(occurrences()).toBe(1);
    document.querySelector<HTMLInputElement>('input[value="hardware"]')?.click();
    await settle();
    expect(document.querySelector('[data-problem]')?.getAttribute('data-problem')).toBe('tools:failed');
    expect(document.querySelectorAll('.tools-warning')).toHaveLength(0);
    expect(occurrences()).toBe(1);
  });

  it('clears a Start refusal once the setup it described is fixed', async () => {
    backend.handlers.get_project_folder = () => ({ project_folder: '', available: true, notice: null });
    backend.handlers.set_project_folder = ({ projectFolder }) => ({ project_folder: projectFolder });
    await renderPage();
    buttonNamed('Start recording')?.click();
    await settle();
    expect(backend.calls.some((call) => call.command.startsWith('start_profile'))).toBe(false);
    expect(infoLine()).toBe('Choose a writable Project folder before recording.');
    backend.openResult = 'C:\\BMEG 420L';
    document.getElementById('project-folder-browse')?.click();
    await settle();
    expect(document.querySelector('[data-testid="recording-state"]')?.textContent).toBe('Ready');
    expect(document.querySelector('.recording-bar')?.textContent).not.toContain('Choose a writable Project folder');
    expect(document.querySelector('.recording-bar .caption')).toBeNull();
  });

  it('drops a Start failure about free space once the Project folder has room again', async () => {
    let free = 100 * 1024 ** 2;
    backend.handlers.estimate_recording_storage = () => ({
      records_per_second: 1000, bmeg_bytes_per_second: 20_000, csv_bytes_per_second: 30_000,
      projected_bytes: 5_000_000, required_free_bytes: 300_000_000, critical_free_bytes: 262_144_000,
      available_free_bytes: free
    });
    backend.handlers.start_profile_simulator_recording = () => {
      throw { stage: 'INITIALIZE_SESSION', code: 'session_start', userMessage: 'There is not enough free space in the Project folder for this recording.', technicalDetail: 'storage guard' };
    };
    backend.handlers.set_project_folder = ({ projectFolder }) => ({ project_folder: projectFolder });
    await renderPage();
    await settle(60);
    expect(document.querySelector('[data-problem]')?.getAttribute('data-problem')).toBe('storage:shortfall');
    buttonNamed('Start recording')?.click();
    await settle();
    expect(document.querySelector('.recording-bar .caption')?.textContent).toContain('not enough free space');
    // The student frees space, then chooses the same Project folder from the banner.
    free = 8 * 1024 ** 3;
    backend.openResult = 'C:\\BMEG 420L';
    document.querySelector<HTMLButtonElement>('#message-action-button')?.click();
    await settle(60);
    expect(backend.calls.filter((call) => call.command === 'estimate_recording_storage').length).toBeGreaterThan(1);
    expect(document.querySelector('[data-problem]')).toBeNull();
    expect(document.querySelector('.recording-bar .caption')).toBeNull();
    expect(document.querySelector('[data-testid="recording-state"]')?.textContent).toBe('Ready');
  });

  it('keeps the page still when a recording ends, and moves focus to Start only from a focused Stop', async () => {
    let run = 0;
    const recording = (overrides: Record<string, unknown> = {}) => idleSession({
      state: 'Acquiring', recording_active: true, simulator: true, duration: { mode: 'timed', seconds: 60 },
      samples: 100, measured_rate_hz: 100, display_origin_timestamp_us: 1_000_000,
      profile: { bench_notice_acknowledged: false, profile: general }, ...overrides
    });
    const finished = () => idleSession({
      last_summary: {
        state: 'Disconnected', samples: 6000, packets: 600, measured_rate_hz: 100, board_elapsed_seconds: 60, host_elapsed_seconds: 60.2,
        bmeg_path: `C:\\BMEG 420L\\run${run}.bmeg`, csv_path: '', metadata_path: `C:\\BMEG 420L\\run${run}.metadata.json`,
        recording_status: 'complete', duration: { mode: 'timed', seconds: 60 }, stop_reason: 'timed_complete', completion_status: 'complete',
        integrity: idleSession().integrity
      }
    });
    backend.handlers.start_profile_simulator_recording = () => { run += 1; return (session = recording()); };
    backend.handlers.stop_recording = () => (session = recording({ state: 'Stopping' }));
    await renderPage();
    const start = () => document.getElementById('start-recording-button') as HTMLButtonElement;
    start().focus();
    start().click();
    await settle();
    expect(document.activeElement?.textContent?.trim()).toBe('Stop recording');

    // Stop with keyboard focus: focus follows it to Start, and nothing scrolls the page.
    const scrollIntoView = vi.fn();
    const originalScrollIntoView = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = scrollIntoView;
    const scrollTo = vi.spyOn(window, 'scrollTo').mockImplementation(() => undefined);
    const focus = vi.spyOn(HTMLElement.prototype, 'focus');
    try {
      buttonNamed('Stop recording')?.click();
      await settle();
      session = finished();
      await settle();
      expect(document.activeElement?.id).toBe('start-recording-button');
      expect(focus).toHaveBeenCalledWith({ preventScroll: true });
      expect(focus.mock.calls.every(([options]) => (options as FocusOptions | undefined)?.preventScroll === true)).toBe(true);

      // A second, timed recording ends by itself after the student clicked the
      // plots (which take no focus): focus is not pulled back to the bar.
      start().click();
      await settle();
      expect(document.activeElement?.textContent?.trim()).toBe('Stop recording');
      document.getElementById('live-panel')?.dispatchEvent(new Event('pointerdown', { bubbles: true }));
      (document.activeElement as HTMLElement).blur();
      await settle();
      session = finished();
      await settle();
      expect(document.activeElement).toBe(document.body);
      expect(scrollIntoView).not.toHaveBeenCalled();
      expect(scrollTo).not.toHaveBeenCalled();
    } finally {
      Element.prototype.scrollIntoView = originalScrollIntoView;
      scrollTo.mockRestore();
      focus.mockRestore();
    }
  });

  it('keeps the after-recording layout after a reload, without announcing that recording again', async () => {
    session = idleSession({
      last_summary: {
        state: 'Disconnected', samples: 6000, packets: 600, measured_rate_hz: 100, board_elapsed_seconds: 60, host_elapsed_seconds: 60.2,
        bmeg_path: 'C:\\BMEG 420L\\run.bmeg', csv_path: 'C:\\BMEG 420L\\run.csv', metadata_path: 'C:\\BMEG 420L\\run.metadata.json',
        recording_status: 'complete', duration: { mode: 'timed', seconds: 60 }, stop_reason: 'timed_complete', completion_status: 'complete',
        integrity: idleSession().integrity
      }
    });
    await renderPage();
    const setupTitle = document.getElementById('setup-title')!;
    const card = document.getElementById('result-card')!;
    expect(document.getElementById('live-panel')!.compareDocumentPosition(card)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(card.compareDocumentPosition(setupTitle)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(document.querySelector('#live-panel .teaching-line')?.textContent).toContain('Teaching use only — not a medical device.');
    expect(infoLine()).not.toContain('Recording saved');
    expect(document.querySelector('#message-info-button')).toBeNull();
  });

  it('leads with the live plots once a recording starts, then shows the result right below them', async () => {
    const recording = (overrides: Record<string, unknown> = {}) => idleSession({
      state: 'Connecting', recording_active: true, simulator: true, duration: { mode: 'timed', seconds: 60 },
      display_origin_timestamp_us: 1_000_000, profile: { bench_notice_acknowledged: false, profile: general }, ...overrides
    });
    backend.handlers.start_profile_simulator_recording = () => (session = recording());
    backend.handlers.stop_recording = () => (session = recording({ state: 'Stopping' }));
    await renderPage();
    // Before any recording, Setup leads and the teaching notice is at the top.
    const setupTitle = document.getElementById('setup-title')!;
    expect(setupTitle.compareDocumentPosition(document.getElementById('live-panel')!)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    const start = document.getElementById('start-recording-button') as HTMLButtonElement;
    start.focus();
    start.click();
    await settle();
    expect(infoLine()).toBe('Starting recording…');
    session = recording({ state: 'Acquiring', samples: 100, measured_rate_hz: 100 });
    await settle();
    // The "Starting…" text is replaced once samples flow.
    expect(infoLine()).toBe('Recording started.');
    // Live signals now come first, with the teaching notice beside the plots.
    expect(document.getElementById('live-panel')!.compareDocumentPosition(setupTitle)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(document.querySelector('#live-panel .teaching-line')?.textContent).toContain('Teaching use only — not a medical device.');
    expect(document.querySelector('fieldset.source')).toBeNull();
    // Keyboard focus followed Start to Stop.
    expect(document.activeElement?.textContent?.trim()).toBe('Stop recording');

    buttonNamed('Stop recording')?.click();
    await settle();
    session = idleSession({
      last_summary: {
        state: 'Disconnected', samples: 6000, packets: 600, measured_rate_hz: 100, board_elapsed_seconds: 60, host_elapsed_seconds: 60.2,
        bmeg_path: 'C:\\BMEG 420L\\run.bmeg', csv_path: 'C:\\BMEG 420L\\run.csv', metadata_path: 'C:\\BMEG 420L\\run.metadata.json',
        recording_status: 'stopped_by_user', duration: { mode: 'timed', seconds: 60 }, stop_reason: 'user', completion_status: 'complete',
        integrity: idleSession().integrity
      }
    });
    await settle();
    // Focus followed Stop back to Start.
    expect(document.activeElement?.id).toBe('start-recording-button');
    const card = document.getElementById('result-card')!;
    expect(document.getElementById('live-panel')!.compareDocumentPosition(card)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(card.compareDocumentPosition(setupTitle)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(card.textContent).toContain('How it ended: Stopped by you.');
    expect(infoLine()).toContain('Recording saved');
    const show = document.querySelector<HTMLButtonElement>('#message-info-button');
    expect(show?.textContent).toBe('Show Last recording');
    show?.click();
    await settle();
    expect(document.activeElement?.id).toBe('result-title');
  });
});

describe('Acquisition page: Install lab file', () => {
  const source = 'C:\\Users\\Student\\Downloads\\general_week3.lab.json';
  const week3 = { ...general, display_name: 'General Analog — Week 3', profile_version: '2.0.3', source: 'imported' } as LabProfile;
  const summary = {
    profile_id: general.profile_id, profile_version: '2.0.3', display_name: week3.display_name, acquisition_mode: 'simultaneous',
    channels: [{ pin: 'A0', label: 'Vin' }, { pin: 'A1', label: 'Vout' }], sample_rate_hz: 1000, adc_resolution_bits: 14,
    already_installed: false,
    replaces: { profile_id: general.profile_id, profile_version: general.profile_version, display_name: general.display_name, course_default: true, has_course_default: true },
    refusal: null as string | null
  };
  const installButton = () => buttonNamed('Install lab file…');
  const installResult = () => document.querySelector('#lab-card .install-result')?.textContent ?? '';
  let installed = false;

  beforeEach(() => {
    installed = false;
    backend.openResult = source;
    backend.handlers.inspect_lab_file = () => summary;
    backend.handlers.install_lab_file = () => { installed = true; return week3; };
    backend.handlers.list_acquisition_profiles = () => installed ? [week3, bp] : [general, bp];
  });

  it('lets a student install a lab file that replaces a course default after a warning, then selects it', async () => {
    await renderPage();
    chooseLab(bp);
    await settle();
    expect(document.querySelector('#lab-card .mode-badge')?.textContent).toBe('Student mode');
    expect(installButton()?.disabled).toBe(false);

    installButton()!.click();
    await settle();
    expect(backend.openCalls).toBe(1);
    expect(backend.calls.find((call) => call.command === 'inspect_lab_file')?.args).toEqual({ source });
    expect(backend.confirmMessages).toHaveLength(1);
    expect(backend.confirmMessages[0]).toContain(`This replaces ${general.display_name} ${general.profile_version} on this computer. Your instructor can undo this with Restore course default.`);
    expect(backend.confirmOptions[0]).toMatchObject({ title: 'Install lab file', kind: 'warning' });
    expect(backend.calls.find((call) => call.command === 'install_lab_file')?.args).toEqual({ source });
    // The list is reloaded and the installed lab is selected.
    expect(labSelect().value).toBe(general.profile_id);
    expect([...labSelect().options].map((option) => option.textContent)).toContain('General Analog — Week 3 — 2.0.3');
    expect(installResult()).toBe(`Installed General Analog — Week 3 2.0.3 in place of ${general.display_name} ${general.profile_version}. It is now the selected lab.`);
  });

  it('installs nothing when the student cancels the question', async () => {
    backend.confirmAnswer = false;
    await renderPage();
    installButton()!.click();
    await settle();
    expect(backend.confirmMessages).toHaveLength(1);
    expect(backend.calls.map((call) => call.command)).not.toContain('install_lab_file');
    expect(installResult()).toBe('');
  });

  it('shows a refusal without asking, and a conflict that Rust reports', async () => {
    summary.refusal = 'A different lab with the same lab ID and version as General Analog — Week 3 2.0.3 is already installed on this computer, so this file was not installed. Ask your instructor for a lab file with a new version.';
    try {
      await renderPage();
      installButton()!.click();
      await settle();
      expect(backend.confirmMessages).toHaveLength(0);
      expect(backend.calls.map((call) => call.command)).not.toContain('install_lab_file');
      expect(installResult()).toBe(summary.refusal);
    } finally {
      summary.refusal = null;
    }

    backend.handlers.install_lab_file = () => { throw 'a recording is running; install the lab file after it has finished'; };
    installButton()!.click();
    await settle();
    expect(installResult()).toBe('The lab file was not installed: a recording is running; install the lab file after it has finished');
    expect(labSelect().value).toBe(general.profile_id);
    expect([...labSelect().options].map((option) => option.textContent)).not.toContain('General Analog — Week 3 — 2.0.3');
  });

  it('is offered in Instructor mode too and is disabled while recording', async () => {
    backend.handlers.get_profile_mode = () => 'instructor_authoring';
    await renderPage();
    expect(installButton()?.disabled).toBe(false);

    session = idleSession({
      state: 'Acquiring', simulator: true, samples: 500, measured_rate_hz: 99.96, recording_active: true,
      duration: { mode: 'timed', seconds: 60 }, elapsed_seconds: 5, remaining_seconds: 55,
      display_origin_timestamp_us: 1_000_000, profile: { bench_notice_acknowledged: false, profile: general }
    });
    await settle(80);
    expect(installButton()?.disabled).toBe(true);
    expect(document.querySelector('#lab-card .install-row')?.textContent).toContain('when no recording is running');
    installButton()!.click();
    await settle();
    expect(backend.openCalls).toBe(0);
  });
});
