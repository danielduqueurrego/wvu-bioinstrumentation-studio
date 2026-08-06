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

<fieldset disabled={disabled} aria-describedby="operating-mode-help">
  <legend>Operating mode</legend>
  <p id="operating-mode-help" class="help">Student is the default. Confirm the acknowledgement, then select Instructor authoring to enable its local workflow tools.</p>
  <div class="choice-row">
    <label class="choice"><input type="radio" name="profile-operating-mode" value="student" bind:group={operatingMode} onchange={handleModeChange} /> Student</label>
    <label class="choice"><input type="radio" name="profile-operating-mode" value="instructor_authoring" bind:group={operatingMode} onchange={handleModeChange} aria-describedby="instructor-mode-explanation" /> Instructor authoring</label>
    <label class="choice instructor-confirm"><input type="checkbox" bind:checked={instructorAcknowledgement} onchange={handleAcknowledgementChange} aria-describedby="instructor-mode-explanation" /> I understand instructor mode can change acquisition settings.</label>
  </div>
  <p id="instructor-mode-explanation" class="help">Instructor mode is a local workflow guard, not strong authentication. Without acknowledgement, Student remains selected and instructor tools remain unavailable.</p>
</fieldset>

<style>
  fieldset { min-width: 0; border: 0; padding: 0; margin: 0; }
  legend { font-weight: 650; margin-bottom: .3rem; }
  .help { margin: .7rem 0 0; color: #4b5965; }
  .choice-row { display: flex; min-width: 0; flex-wrap: wrap; gap: .7rem; align-items: end; }
  .choice { min-width: 0; display: flex; align-items: center; gap: .4rem; font-weight: 600; }
  .choice input { width: auto; min-height: auto; }
  .instructor-confirm { flex-basis: min(100%, 30rem); }
  input:focus-visible { outline: 3px solid #EEAA00; outline-offset: 2px; }
</style>
