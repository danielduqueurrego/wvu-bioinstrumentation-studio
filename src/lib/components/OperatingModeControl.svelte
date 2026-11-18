<script lang="ts">
  import type { OperatingMode } from '$lib/operating-mode';

  export let operatingMode: OperatingMode = 'student';
  export let instructorAcknowledgement = false;
  export let disabled = false;
  export let onModeConfirmed: (mode: OperatingMode) => void = () => {};
  export let onInstructorBlocked: () => void = () => {};

  function handleModeChange() {
    if (operatingMode === 'instructor_authoring' && !instructorAcknowledgement) {
      // bind:group may have set the browser value already. Restore the single
      // authoritative Student value in the same change turn.
      operatingMode = 'student';
      onInstructorBlocked();
      return;
    }
    onModeConfirmed(operatingMode);
  }

  function handleAcknowledgementChange() {
    if (!instructorAcknowledgement && operatingMode === 'instructor_authoring') {
      operatingMode = 'student';
      onModeConfirmed('student');
    }
  }
</script>

<!-- The acknowledgement comes before the Instructor choice, in the order it must be done. -->
<fieldset disabled={disabled} aria-describedby="operating-mode-help">
  <legend>Operating mode</legend>
  <p id="operating-mode-help" class="help">Student is the default. Instructor mode is a safety step against accidental changes, not a password lock: tick the acknowledgement first, then select Instructor mode to edit lab settings.</p>
  <label class="choice instructor-confirm"><input type="checkbox" bind:checked={instructorAcknowledgement} onchange={handleAcknowledgementChange} aria-describedby="instructor-mode-explanation" /> I understand Instructor mode can change acquisition settings.</label>
  <div class="choice-row">
    <label class="choice"><input type="radio" name="profile-operating-mode" value="student" bind:group={operatingMode} onchange={handleModeChange} /> Student</label>
    <label class="choice"><input type="radio" name="profile-operating-mode" value="instructor_authoring" bind:group={operatingMode} onchange={handleModeChange} aria-describedby="instructor-mode-explanation" /> Instructor mode</label>
  </div>
  <p id="instructor-mode-explanation" class="help">Instructor mode changes lab settings on this computer. Without the acknowledgement, Student stays selected and lab-management tools stay unavailable.</p>
</fieldset>

<style>
  fieldset { min-width: 0; border: 0; padding: 0; margin: .6rem 0 0; }
  legend { font-weight: 650; margin-bottom: .3rem; padding: 0; }
  .help { margin: .5rem 0 0; color: var(--muted, #4b5965); }
  .choice-row { display: flex; min-width: 0; flex-wrap: wrap; gap: .7rem; align-items: end; margin-top: .5rem; }
  .choice { min-width: 0; display: flex; align-items: center; gap: .4rem; font-weight: 600; }
  .choice input { width: auto; min-height: auto; }
  .instructor-confirm { margin-top: .5rem; }
  input:focus-visible { outline: 3px solid var(--focus-ring, #002855); outline-offset: 2px; box-shadow: 0 0 0 2px #fff; }
</style>
