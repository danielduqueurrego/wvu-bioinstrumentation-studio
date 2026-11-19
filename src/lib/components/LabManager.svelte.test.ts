// @vitest-environment happy-dom
/**
 * The Lab Manager's destructive actions must wait for the instructor's answer.
 * In the real app plugin-dialog replaces window.confirm with an async function
 * (a Promise is always truthy), so these use the plugin's own confirm, which
 * is mocked here.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import type { LabListEntry, LabProfile } from '$lib/labs';
import LabManager from './LabManager.svelte';

const backend = vi.hoisted(() => ({
  handlers: {} as Record<string, (args: Record<string, unknown>) => unknown>,
  calls: [] as Array<{ command: string; args: Record<string, unknown> }>,
  confirmAnswer: false as boolean | Error,
  confirms: [] as Array<{ message: string; options: unknown }>,
  saveResult: null as string | null,
  saves: [] as unknown[]
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (command: string, args: Record<string, unknown> = {}) => {
    backend.calls.push({ command, args });
    const handler = backend.handlers[command];
    if (!handler) throw new Error(`unexpected command ${command}`);
    return handler(args);
  }
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({
  confirm: async (message: string, options?: unknown) => {
    backend.confirms.push({ message, options });
    if (backend.confirmAnswer instanceof Error) throw backend.confirmAnswer;
    return backend.confirmAnswer;
  },
  open: async () => null,
  save: async (options?: unknown) => {
    backend.saves.push(options);
    return backend.saveResult;
  }
}));

const factory = JSON.parse(
  readFileSync(resolve(process.cwd(), 'profiles', 'general_analog_development.profile.json'), 'utf8')
) as LabProfile;
const local: LabProfile = { ...factory, profile_version: '2.0.1', source: 'instructor' };
const entries: LabListEntry[] = [
  { profile: factory, active: false, retired: false },
  { profile: local, active: true, retired: false }
];

let catalogChanges = 0;
let statuses: string[] = [];

function installBackend() {
  backend.calls = [];
  backend.confirms = [];
  backend.confirmAnswer = false;
  backend.saveResult = null;
  backend.saves = [];
  backend.handlers = {
    list_instructor_labs: () => entries,
    get_catalog_status: () => ({ writable: true, skipped_labs: [] }),
    reset_local_lab_customizations: () => ({ backup_path: null, removed_local_revisions: 1 }),
    retire_profile: () => null,
    begin_lab_edit: () => structuredClone(local)
  };
}

function render() {
  catalogChanges = 0;
  statuses = [];
  const component = mount(LabManager, {
    target: document.body,
    props: {
      selectedProfile: local,
      onSaved: () => {},
      onCatalogChanged: () => { catalogChanges += 1; },
      onStatus: (message: string) => { statuses.push(message); },
      disabled: false
    }
  });
  flushSync();
  return component;
}

async function settle() {
  for (let round = 0; round < 4; round += 1) {
    await new Promise((done) => setTimeout(done, 0));
    flushSync();
  }
}

function button(text: string): HTMLButtonElement {
  const found = [...document.querySelectorAll<HTMLButtonElement>('button')].find((candidate) => candidate.textContent?.trim() === text);
  if (!found) throw new Error(`button "${text}" not found`);
  return found;
}

async function click(text: string) {
  button(text).click();
  await settle();
}

const commands = () => backend.calls.map((call) => call.command);
const dialogOpen = () => document.querySelector('[role="dialog"]') !== null;

beforeEach(installBackend);

afterEach(() => {
  document.body.replaceChildren();
});

describe('LabManager confirmations', () => {
  it('resets local labs only after the instructor confirms', async () => {
    const component = render();
    await click('Manage Labs');

    await click('Reset local customizations');
    expect(backend.confirms).toHaveLength(1);
    expect(backend.confirms[0].message).toContain('Reset local lab customizations?');
    expect(backend.confirms[0].options).toMatchObject({ kind: 'warning' });
    expect(commands()).not.toContain('reset_local_lab_customizations');
    expect(catalogChanges).toBe(0);

    backend.confirmAnswer = true;
    await click('Reset local customizations');
    expect(commands()).toContain('reset_local_lab_customizations');
    expect(catalogChanges).toBe(1);
    expect(statuses.at(-1)).toContain('Reset local lab customizations.');
    unmount(component);
  });

  it('changes nothing when the confirmation cannot be shown', async () => {
    const component = render();
    await click('Manage Labs');
    backend.confirmAnswer = new Error('dialog unavailable');
    await click('Reset local customizations');
    expect(commands()).not.toContain('reset_local_lab_customizations');
    expect(document.querySelector('#lab-manager-error')?.textContent).toContain('nothing was changed');
    unmount(component);
  });

  it('retires the active revision only after the instructor confirms', async () => {
    const component = render();
    await click('Manage Labs');

    await click('Retire');
    expect(backend.confirms[0].message).toContain(`Retire ${local.display_name} 2.0.1, the active revision?`);
    expect(commands()).not.toContain('retire_profile');

    backend.confirmAnswer = true;
    await click('Retire');
    const retire = backend.calls.find((call) => call.command === 'retire_profile');
    expect(retire?.args).toEqual({ profileId: local.profile_id, profileVersion: '2.0.1' });
    expect(catalogChanges).toBe(1);
    unmount(component);
  });

  it('keeps an unsaved draft open unless the instructor discards it', async () => {
    const component = render();
    await click('Manage Labs');
    await click('Edit selected lab');
    expect(button('Save changes as new version')).toBeTruthy();

    await click('Close');
    expect(backend.confirms[0].message).toContain('Discard unsaved lab changes?');
    expect(dialogOpen()).toBe(true);
    expect(button('Save changes as new version')).toBeTruthy();

    backend.confirmAnswer = true;
    await click('Close');
    expect(dialogOpen()).toBe(false);
    unmount(component);
  });

  it('closes without asking when there is no draft', async () => {
    const component = render();
    await click('Manage Labs');
    await click('Close');
    expect(backend.confirms).toHaveLength(0);
    expect(dialogOpen()).toBe(false);
    unmount(component);
  });
});

describe('LabManager Save as course default file', () => {
  const destination = 'C:\\Users\\me\\Desktop\\general_analog_development.profile.json';
  const outcome = {
    path: destination, file_name: 'general_analog_development.profile.json', profile_id: local.profile_id,
    display_name: local.display_name, profile_version: '2.1.0', canonical_hash: 'b'.repeat(64), replaces_version: '2.0.0',
    factory_file_name: 'general_analog_development.profile.json', source_line: 'factory_lab_file!("general_analog_development.profile.json"),'
  };

  beforeEach(() => {
    backend.handlers.list_instructor_labs = () => entries.map((entry) => ({ ...entry, course_default_file_name: 'general_analog_development.profile.json' }));
    backend.handlers.export_course_default_file = () => outcome;
  });

  const courseDefaultButtons = () => [...document.querySelectorAll<HTMLButtonElement>('button')].filter((candidate) => candidate.textContent?.trim() === 'Save as course default file…');

  it('offers it for local revisions only and explains the next steps after saving', async () => {
    const component = render();
    await click('Manage Labs');
    // The factory row has nothing new to save; only the local revision offers it.
    expect(courseDefaultButtons()).toHaveLength(1);

    backend.saveResult = destination;
    await click('Save as course default file…');
    expect(backend.saves[0]).toMatchObject({ defaultPath: 'general_analog_development.profile.json', filters: [{ extensions: ['profile.json'] }] });
    const exported = backend.calls.find((call) => call.command === 'export_course_default_file');
    // Tauri maps these camelCase names to the command's snake_case arguments.
    expect(exported?.args).toEqual({ profileId: local.profile_id, profileVersion: '2.0.1', destination });
    const notice = document.querySelector('.course-default-notice')?.textContent ?? '';
    expect(notice).toContain(`Saved ${local.display_name} 2.1.0 as a course default file: ${destination}.`);
    expect(notice).toContain('scripts\\update_factory_labs.ps1 check');
    expect(notice).toContain('Rebuild the application.');
    expect(statuses.at(-1)).toBe(notice);
    // Saving a file never reloads or changes the catalog.
    expect(catalogChanges).toBe(0);
    unmount(component);
  });

  it('writes nothing when the Save dialog is cancelled and shows a refusal from Rust', async () => {
    const component = render();
    await click('Manage Labs');
    await click('Save as course default file…');
    expect(backend.saves).toHaveLength(1);
    expect(commands()).not.toContain('export_course_default_file');

    backend.saveResult = destination;
    backend.handlers.export_course_default_file = () => { throw 'student mode may not perform this profile authoring action'; };
    await click('Save as course default file…');
    expect(document.querySelector('#lab-manager-error')?.textContent).toContain('student mode may not perform');
    expect(document.querySelector('.course-default-notice')).toBeNull();
    unmount(component);
  });
});

describe('LabManager acquisition modes', () => {
  // The built-in pulse-ox lab 1.0.0 (legacy 4-state), kept in profiles/legacy/.
  const pulseLegacy = JSON.parse(
    readFileSync(resolve(process.cwd(), 'profiles', 'legacy', 'pulseox_tx_rx_course_capture_1.0.0.profile.json'), 'utf8')
  ) as LabProfile;
  // The built-in pulse-ox lab 1.1.0 (LED-sequenced).
  const pulseSequenced = JSON.parse(
    readFileSync(resolve(process.cwd(), 'profiles', 'pulseox_tx_rx_course_capture.profile.json'), 'utf8')
  ) as LabProfile;

  function modeSelect(): HTMLSelectElement {
    const select = [...document.querySelectorAll('label')]
      .find((label) => label.textContent?.startsWith('Acquisition mode'))
      ?.querySelector('select');
    if (!select) throw new Error('mode select not found');
    return select;
  }

  const options = () => [...modeSelect().options].map((option) => [option.value, option.textContent?.trim()]);

  async function chooseMode(value: string) {
    const select = modeSelect();
    select.value = value;
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
  }

  const labelled = (label: string) => document.querySelector<HTMLInputElement | HTMLSelectElement>(`[aria-label="${label}"]`);

  async function setDwell(value: string) {
    const dwell = [...document.querySelectorAll('label')].find((label) => label.textContent?.startsWith('State dwell'))!.querySelector('input')!;
    dwell.value = value;
    dwell.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();
  }

  /** Saves the draft and returns what `save_lab_draft` received. */
  async function saveAndCapture(): Promise<LabProfile | undefined> {
    let saved: LabProfile | undefined;
    backend.handlers.save_lab_draft = (args) => {
      saved = args.draft as LabProfile;
      return saved;
    };
    await click('Save changes as new version');
    expect(document.querySelector('#lab-manager-error')).toBeNull();
    return saved;
  }

  it('offers simultaneous and LED-sequenced pulse-ox, never the legacy mode, for a new or simultaneous lab', async () => {
    const component = render();
    await click('Manage Labs');
    await click('Edit selected lab');
    expect(options()).toEqual([
      ['simultaneous', 'Simultaneous Analog'],
      ['pulseox_sequenced', 'Pulse Oximetry — LED-sequenced']
    ]);
    // The simultaneous grid has no LED-state column.
    expect(labelled('Channel 1 shown by LED state')).toBeNull();

    await chooseMode('pulseox_sequenced');
    expect(document.body.textContent).toContain('Pulse oximetry — LED-sequenced');
    expect((labelled('Channel 1 ID') as HTMLInputElement).value).toBe('tx');
    expect((labelled('Channel 2 ID') as HTMLInputElement).value).toBe('rx');
    expect((labelled('Channel 1 shown by LED state') as HTMLInputElement).checked).toBe(true);
    expect(document.querySelector('.estimate')?.textContent).toContain('1000 records/s');
    expect(document.body.textContent).toContain('1000 rows/s for each channel (every row holds all 2 channels); 250 rows/s per LED state');
    // Still no legacy 4-state mode.
    expect(options().map(([value]) => value)).toEqual(['simultaneous', 'pulseox_sequenced']);

    // The dwell sets the record rate; a third channel can be added.
    const dwell = [...document.querySelectorAll('label')].find((label) => label.textContent?.startsWith('State dwell'))!.querySelector('input')!;
    dwell.value = '2500';
    dwell.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();
    expect(document.body.textContent).toContain('400 rows/s for each channel');
    await click('Add channel');
    expect(labelled('Channel 3 pin')?.value).toBe('A2');
    expect((labelled('Channel 3 shown by LED state') as HTMLInputElement).checked).toBe(false);

    let saved: LabProfile | undefined;
    backend.handlers.save_lab_draft = (args) => {
      saved = args.draft as LabProfile;
      return saved;
    };
    await click('Save changes as new version');
    expect(document.querySelector('#lab-manager-error')).toBeNull();
    expect(saved?.acquisition).toMatchObject({
      acquisition_mode: 'pulseox_sequenced', state_dwell_us: 2500, sample_rate_hz: 400, adc_resolution_bits: 14,
      digital_outputs: [
        { pin: 'D5', label: 'Red LED', behavior: 'acquisition_sequenced' },
        { pin: 'D6', label: 'IR LED', behavior: 'acquisition_sequenced' }
      ]
    });
    expect(saved?.acquisition.analog_inputs).toBeUndefined();
    expect(saved?.acquisition.channels?.map((channel) => [channel.pin, channel.id, channel.split_by_led_state === true]))
      .toEqual([['A0', 'tx', true], ['A1', 'rx', true], ['A2', 'channel_1', false]]);
    expect(saved?.required_firmware.build).toBe('0x00010007');
    expect(saved?.plot_defaults?.groups).toEqual([{ channel_ids: ['tx'] }, { channel_ids: ['rx'] }, { channel_ids: ['channel_1'] }]);
    unmount(component);
  });

  it('keeps the legacy 4-state mode available only while editing a lab that uses it', async () => {
    backend.handlers.begin_lab_edit = () => structuredClone(pulseLegacy);
    const component = render();
    await click('Manage Labs');
    await click('Edit selected lab');
    expect(options()).toEqual([
      ['simultaneous', 'Simultaneous Analog'],
      ['pulseox_sequenced', 'Pulse Oximetry — LED-sequenced'],
      ['pulseox_4state', 'Pulse Oximetry — legacy 4-state']
    ]);
    expect(modeSelect().value).toBe('pulseox_4state');
    expect(document.body.textContent).toContain('Pulse oximetry — legacy 4-state');

    // Trying another mode keeps the way back to the lab's own mode.
    await chooseMode('pulseox_sequenced');
    expect(options().map(([value]) => value)).toContain('pulseox_4state');
    await chooseMode('simultaneous');
    expect(labelled('Channel 1 shown by LED state')).toBeNull();
    expect(options().map(([value]) => value)).toContain('pulseox_4state');
    unmount(component);
  });

  it('edits the built-in pulse-ox lab 1.1.0 in the LED-sequenced mode, with no legacy option', async () => {
    backend.handlers.begin_lab_edit = () => structuredClone(pulseSequenced);
    const component = render();
    await click('Manage Labs');
    await click('Edit selected lab');
    expect(modeSelect().value).toBe('pulseox_sequenced');
    expect(options().map(([value]) => value)).toEqual(['simultaneous', 'pulseox_sequenced']);
    expect((labelled('Channel 1 ID') as HTMLInputElement).value).toBe('tx');
    expect((labelled('Channel 2 ID') as HTMLInputElement).value).toBe('rx');
    expect((labelled('Channel 1 shown by LED state') as HTMLInputElement).checked).toBe(true);
    expect((labelled('Channel 2 shown by LED state') as HTMLInputElement).checked).toBe(true);
    expect(document.body.textContent).toContain('1000 rows/s for each channel (every row holds all 2 channels); 250 rows/s per LED state');
    unmount(component);
  });

  it('gives the rows per LED state from the dwell, not from the rounded record rate', async () => {
    backend.handlers.begin_lab_edit = () => structuredClone(pulseSequenced);
    const component = render();
    await click('Manage Labs');
    await click('Edit selected lab');
    // 1e6 / 1500 µs is stored as 667 records/s; each state gets 1e6 / 6000 ≈ 166.67 rows/s, not 667 / 4.
    await setDwell('1500');
    expect(document.body.textContent).toContain('667 rows/s for each channel (every row holds all 2 channels); about 166.7 rows/s per LED state');
    await setDwell('3000');
    expect(document.body.textContent).toContain('333 rows/s for each channel (every row holds all 2 channels); about 83.3 rows/s per LED state');
    // A fractional dwell is refused by the editor before Rust sees it.
    await setDwell('1000.5');
    backend.handlers.save_lab_draft = () => { throw new Error('must not be called'); };
    await click('Save changes as new version');
    expect(document.querySelector('#lab-manager-error')?.textContent).toContain('dwell must be 1000–5000 µs, in whole microseconds');
    expect(commands()).not.toContain('save_lab_draft');
    unmount(component);
  });

  it('saves an edited legacy 4-state lab in its own mode and record layout', async () => {
    backend.handlers.begin_lab_edit = () => structuredClone(pulseLegacy);
    const component = render();
    await click('Manage Labs');
    await click('Edit selected lab');
    await setDwell('500');
    expect(document.querySelector('.estimate')?.textContent).toContain('500.0 cycles/s nominal');
    const saved = await saveAndCapture();
    expect(saved?.acquisition).toMatchObject({
      acquisition_mode: 'pulseox_4state', channels: [], analog_inputs: { tx: 'A0', rx: 'A1' }, state_dwell_us: 500, sample_rate_hz: 500,
      digital_outputs: [
        { pin: 'D5', label: 'Red LED', behavior: 'acquisition_sequenced' },
        { pin: 'D6', label: 'IR LED', behavior: 'acquisition_sequenced' }
      ]
    });
    expect(saved?.acquisition.led_outputs).toBeUndefined();
    expect(saved?.required_firmware.build).toBe('0x00010006');
    unmount(component);
  });

  it('restores the opened firmware build when a draft leaves the LED-sequenced mode', async () => {
    // A simultaneous lab (0x00010006) tried as LED-sequenced (0x00010007) and switched back.
    let component = render();
    await click('Manage Labs');
    await click('Edit selected lab');
    await chooseMode('pulseox_sequenced');
    await chooseMode('simultaneous');
    expect((await saveAndCapture())?.required_firmware.build).toBe('0x00010006');
    unmount(component);
    document.body.replaceChildren();

    // The same for a legacy 4-state lab.
    backend.handlers.begin_lab_edit = () => structuredClone(pulseLegacy);
    component = render();
    await click('Manage Labs');
    await click('Edit selected lab');
    await chooseMode('pulseox_sequenced');
    await chooseMode('pulseox_4state');
    const legacy = await saveAndCapture();
    expect(legacy?.acquisition.acquisition_mode).toBe('pulseox_4state');
    expect(legacy?.required_firmware.build).toBe('0x00010006');
    unmount(component);
    document.body.replaceChildren();

    // A lab opened in the LED-sequenced mode keeps the build it names.
    backend.handlers.begin_lab_edit = () => structuredClone(pulseSequenced);
    component = render();
    await click('Manage Labs');
    await click('Edit selected lab');
    await chooseMode('simultaneous');
    expect((await saveAndCapture())?.required_firmware.build).toBe('0x00010007');
    unmount(component);
  });

  it('refuses to save a sequenced draft that breaks a Rust rule, before asking Rust', async () => {
    const component = render();
    await click('Manage Labs');
    await click('Edit selected lab');
    await chooseMode('pulseox_sequenced');
    const csv = labelled('Channel 2 CSV field') as HTMLInputElement;
    csv.value = 'led_state';
    csv.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();
    backend.handlers.save_lab_draft = () => { throw new Error('must not be called'); };
    await click('Save changes as new version');
    expect(document.querySelector('#lab-manager-error')?.textContent).toContain('CSV column "led_state"');
    expect(commands()).not.toContain('save_lab_draft');
    unmount(component);
  });
});
