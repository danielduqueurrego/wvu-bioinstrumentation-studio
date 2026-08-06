// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import OperatingModeControl from './OperatingModeControl.svelte';
import type { OperatingMode } from '$lib/operating-mode';

type RenderedControl = {
  component: Record<string, never>;
  events: OperatingMode[];
  blocked: () => number;
  student: () => HTMLInputElement;
  instructor: () => HTMLInputElement;
  acknowledgement: () => HTMLInputElement;
};

function renderControl(): RenderedControl {
  const events: OperatingMode[] = [];
  let blocked = 0;
  const component = mount(OperatingModeControl, {
    target: document.body,
    props: {
      operatingMode: 'student',
      instructorAcknowledgement: false,
      onModeConfirmed: (mode: OperatingMode) => events.push(mode),
      onInstructorBlocked: () => { blocked += 1; }
    }
  }) as Record<string, never>;
  flushSync();
  return {
    component,
    events,
    blocked: () => blocked,
    student: () => document.querySelector<HTMLInputElement>('input[value="student"]')!,
    instructor: () => document.querySelector<HTMLInputElement>('input[value="instructor_authoring"]')!,
    acknowledgement: () => document.querySelector<HTMLInputElement>('input[type="checkbox"]')!
  };
}

function click(input: HTMLInputElement) {
  input.click();
  flushSync();
}

function expectSelected(control: RenderedControl, mode: OperatingMode) {
  expect(control.student().checked).toBe(mode === 'student');
  expect(control.instructor().checked).toBe(mode === 'instructor_authoring');
  expect(Number(control.student().checked) + Number(control.instructor().checked)).toBe(1);
}

afterEach(() => {
  document.body.replaceChildren();
});

describe('OperatingModeControl DOM synchronization', () => {
  it('starts with exactly Student selected', () => {
    const control = renderControl();
    expectSelected(control, 'student');
    unmount(control.component);
  });

  it('restores Student when Instructor is selected before acknowledgement', () => {
    const control = renderControl();
    click(control.instructor());
    expectSelected(control, 'student');
    expect(control.events).toEqual([]);
    expect(control.blocked()).toBe(1);
    unmount(control.component);
  });

  it('keeps DOM and mode callbacks synchronized across Instructor, Student, and re-entry', () => {
    const control = renderControl();
    click(control.acknowledgement());
    click(control.instructor());
    expectSelected(control, 'instructor_authoring');
    expect(control.events).toEqual(['instructor_authoring']);

    click(control.student());
    expectSelected(control, 'student');
    expect(control.acknowledgement().checked).toBe(true);
    expect(control.events).toEqual(['instructor_authoring', 'student']);

    click(control.instructor());
    expectSelected(control, 'instructor_authoring');
    expect(control.events).toEqual(['instructor_authoring', 'student', 'instructor_authoring']);
    unmount(control.component);
  });

  it('clearing acknowledgement exits Instructor and preserves radio exclusivity', () => {
    const control = renderControl();
    click(control.acknowledgement());
    click(control.instructor());
    click(control.acknowledgement());
    expect(control.acknowledgement().checked).toBe(false);
    expectSelected(control, 'student');
    expect(control.events).toEqual(['instructor_authoring', 'student']);
    unmount(control.component);
  });

  it('a keyboard-originated native change retains one checked radio and enters Instructor', () => {
    const control = renderControl();
    click(control.acknowledgement());
    control.student().focus();
    // Browsers implement arrow-key radio navigation by changing the focused
    // group's checked value before dispatching change. happy-dom does not
    // implement that default action, so reproduce that standards-defined DOM
    // transition and verify the component's change handler.
    control.instructor().checked = true;
    control.instructor().dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();
    expectSelected(control, 'instructor_authoring');
    expect(control.events).toEqual(['instructor_authoring']);
    unmount(control.component);
  });
});
