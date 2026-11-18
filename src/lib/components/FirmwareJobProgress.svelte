<script lang="ts">
  import { formatClock } from '$lib/recording-status';
  import {
    doubleResetSecondsLeft,
    FIRMWARE_WRITE_NOTICE,
    firmwareJobView,
    firmwareStepper
  } from '$lib/firmware-job';
  import type { FirmwareJob } from '$lib/session-types';

  /**
   * The restore as six steps: Prepare → Compile → Switch board to upload
   * mode → Upload → Reconnect → Verify, driven by the backend's job stage.
   * While a native-USB board waits for the double press of RESET a card
   * says so, with the time left. From the upload onward there is no Cancel
   * (the backend refuses it): the board is being written.
   *
   * Step changes are announced through the page's single live region (the
   * page sets its information line); nothing here is a live region.
   */
  type Props = {
    job: FirmwareJob;
    closeBlockedNotice?: string;
    onCancel: () => void;
  };

  let { job, closeBlockedNotice = '', onCancel }: Props = $props();

  const view = $derived(firmwareJobView(job));
  const steps = $derived(firmwareStepper(job));
  let now = $state(Date.now());
  const secondsLeft = $derived(doubleResetSecondsLeft(job, now));
  const waitingForReset = $derived(Boolean(job.active && job.stage === 'waiting_for_bootloader' && job.manual_reset_deadline_utc));

  // A one-second clock only while the double-reset card is shown.
  $effect(() => {
    if (!waitingForReset) return;
    now = Date.now();
    const timer = window.setInterval(() => { now = Date.now(); }, 1000);
    return () => window.clearInterval(timer);
  });

  const STATUS_WORD = { done: 'done', current: 'in progress', pending: 'not started', failed: 'failed' } as const;
</script>

<div class="firmware-progress">
  <h2 id="operation-title">{view.title}</h2>
  <ol class="stepper" aria-label="Restore steps">
    {#each steps as step, index (step.id)}
      <li class="step {step.status}" aria-current={step.status === 'current' ? 'step' : undefined}>
        <span class="step-marker" aria-hidden="true">{step.status === 'done' ? '✓' : step.status === 'failed' ? '✗' : index + 1}</span>
        <span class="step-label">{step.label}</span>
        <span class="step-status">{step.status === 'current' || step.status === 'failed' ? STATUS_WORD[step.status] : ''}<span class="visually-hidden">{step.status === 'done' || step.status === 'pending' ? STATUS_WORD[step.status] : ''}</span></span>
      </li>
    {/each}
  </ol>
  <p id="operation-stage" class="stage-detail">{view.detail}</p>

  {#if secondsLeft !== undefined}
    <section class="reset-card" aria-labelledby="double-reset-title">
      <h3 id="double-reset-title">Press the RESET button twice quickly</h3>
      <p>Find the small RESET button on the UNO R4 WiFi and press it two times in quick succession, like a double-click. The board then switches to upload mode and the restore continues on its own.</p>
      <p class="countdown">Time left: <strong>{formatClock(secondsLeft)}</strong></p>
      <p class="help">If the time runs out, nothing is written; select Restore WVU Firmware again.</p>
    </section>
  {/if}

  {#if job.active}
    {#if view.cancelable}
      <button type="button" class="cancel" onclick={onCancel}>{view.cancelLabel}</button>
    {:else}
      <p class="write-notice"><span aria-hidden="true">⚠</span> {FIRMWARE_WRITE_NOTICE}</p>
    {/if}
  {/if}
  {#if closeBlockedNotice}<p class="help">{closeBlockedNotice}</p>{/if}
</div>

<style>
  .firmware-progress { min-width: 0; }
  h2 { margin: 0 0 .75rem; }
  .stepper { display: grid; gap: .3rem; margin: 0; padding: 0; list-style: none; }
  .step { display: flex; flex-wrap: wrap; align-items: center; gap: .55rem; min-width: 0; padding: .3rem .4rem; border-radius: .3rem; }
  .step-marker {
    flex: 0 0 auto;
    display: grid;
    place-items: center;
    width: 1.7rem;
    height: 1.7rem;
    border: 2px solid var(--border-strong, #8493a0);
    border-radius: 50%;
    color: var(--muted, #4b5965);
    font-weight: 800;
    font-size: .9rem;
  }
  .step-label { font-weight: 650; }
  .step-status { color: var(--muted, #4b5965); font-size: .85rem; font-weight: 700; }
  .step.pending .step-label { color: var(--muted, #4b5965); }
  .step.done .step-marker { border-color: var(--ok, #176c33); background: var(--ok, #176c33); color: #fff; }
  .step.current { background: var(--surface-muted, #f2f5f8); }
  .step.current .step-marker { border-color: var(--wvu-navy, #002855); background: var(--wvu-navy, #002855); color: #fff; }
  .step.current .step-label { font-weight: 800; }
  .step.failed .step-marker { border-color: var(--danger, #9d2424); background: var(--danger, #9d2424); color: #fff; }
  .step.failed .step-status { color: var(--danger, #9d2424); }
  .stage-detail { margin: .75rem 0 0; overflow-wrap: anywhere; }
  .reset-card {
    margin-top: .8rem;
    padding: .75rem .9rem;
    border: 2px solid var(--wvu-gold, #EEAA00);
    border-left-width: 8px;
    border-radius: .35rem;
    background: #fff8e1;
  }
  .reset-card h3 { margin: 0 0 .4rem; font-size: 1.15rem; color: var(--ink, #17222e); }
  .reset-card p { margin: .35rem 0; }
  .countdown { font-size: 1.1rem; font-variant-numeric: tabular-nums; }
  .help { color: var(--muted, #4b5965); }
  .cancel {
    margin-top: .9rem;
    min-height: 2.5rem;
    padding: .5rem .9rem;
    border: 1px solid #761a1a;
    border-radius: .3rem;
    background: var(--danger, #9d2424);
    color: #fff;
    font: inherit;
    font-weight: 700;
    cursor: pointer;
  }
  .write-notice {
    margin: .9rem 0 0;
    padding: .6rem .75rem;
    border-left: 5px solid var(--danger, #9d2424);
    background: var(--danger-bg, #fdf0f0);
    font-weight: 800;
  }
  .visually-hidden { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
</style>
