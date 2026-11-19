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
