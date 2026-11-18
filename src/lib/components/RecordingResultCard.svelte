<script lang="ts">
  import CopyButton from './CopyButton.svelte';
  import { stopReasonText, type RecordingResultView } from '$lib/recording-result';

  /**
   * The finished recording: each file with ✓ Saved or ✗ Not saved, how the
   * recording ended, and the counts to check before leaving the bench. Copy
   * path uses the clipboard (or selects the text); Show in folder asks the
   * page to open File Explorer on that file.
   */
  type Props = {
    view: RecordingResultView;
    /** The backend stop reason; shown in plain words (the raw code is in Advanced details). */
    stopReason: string;
    labReference?: string;
    onReveal: (path: string) => void;
    onFeedback: (message: string) => void;
  };

  let { view, stopReason, labReference, onReveal, onFeedback }: Props = $props();
</script>

<section class="result-card" id="result-card" class:incomplete={!view.complete} aria-labelledby="result-title">
  <!-- Focusable so the information line can take the student here after a recording. -->
  <h2 id="result-title" tabindex="-1">Last recording</h2>
  <p class="result-headline">
    <span class="headline-symbol" aria-hidden="true">{view.complete ? '✓' : '!'}</span>
    <strong>{view.headline}.</strong>
    <span>{view.detail}</span>
  </p>

  <ul class="result-files" aria-label="Recording files">
    {#each view.files as file (file.id)}
      <li class="result-file" class:missing={!file.saved}>
        <span class="file-status">
          <span class="file-symbol" aria-hidden="true">{file.saved ? '✓' : '✗'}</span>
          <strong>{file.label}</strong>
          <span class="file-word">{file.saved ? 'Saved' : 'Not saved'}</span>
        </span>
        {#if file.path}<span class="path" title={file.path}>{file.path}</span>{/if}
        {#if file.note}<span class="note">{file.note}</span>{/if}
        {#if file.path}
          <span class="file-actions">
            <CopyButton text={file.path} label="Copy path" ariaLabel={`Copy path: ${file.label}`} copiedMessage={`${file.label} path copied.`} onCopied={onFeedback} />
            <button type="button" class="reveal" aria-label={`Show in folder: ${file.label}`} onclick={() => onReveal(file.path)}>Show in folder</button>
          </span>
        {/if}
      </li>
    {/each}
  </ul>

  <dl class="result-facts">
    <div><dt>Duration</dt><dd>{view.duration}</dd></div>
    <div><dt>Samples</dt><dd>{view.samples}</dd></div>
    <div><dt>Measured rate</dt><dd>{view.rate}</dd></div>
    <div><dt>Markers</dt><dd>{view.markers}</dd></div>
  </dl>
  <p class="quality" class:attention={!view.qualityOk}>
    <span aria-hidden="true">{view.qualityOk ? '✓' : '!'}</span> {view.quality}
  </p>

  <details class="result-details">
    <summary>Recording details</summary>
    <p>How it ended: {stopReasonText(stopReason)}.</p>
    {#if labReference}<p>Lab reference: {labReference}</p>{/if}
  </details>
</section>

<style>
  .result-card {
    min-width: 0;
    margin: 1rem 0;
    padding: clamp(.8rem, 2vw, 1rem);
    border: 1px solid var(--border, #d7dde2);
    border-left: 6px solid var(--ok, #176c33);
    border-radius: .35rem;
    background: var(--surface, #fff);
  }
  .result-card.incomplete { border-left-color: var(--warn, #9b6700); }
  h2 { margin: 0 0 .4rem; }
  .result-headline { margin: 0 0 .7rem; overflow-wrap: anywhere; }
  .headline-symbol { display: inline-grid; place-items: center; width: 1.4rem; height: 1.4rem; margin-right: .2rem; border-radius: 50%; background: var(--ok, #176c33); color: #fff; font-weight: 900; }
  .incomplete .headline-symbol { background: var(--warn, #9b6700); }
  .result-files { display: grid; gap: .45rem; margin: 0; padding: 0; list-style: none; }
  .result-file {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: .35rem .75rem;
    min-width: 0;
    padding: .5rem .65rem;
    border: 1px solid var(--border, #d7dde2);
    border-radius: .3rem;
    background: #f9fbfc;
  }
  .result-file.missing { border-color: var(--danger, #9d2424); background: var(--danger-bg, #fdf0f0); }
  .file-status { display: inline-flex; align-items: center; gap: .35rem; min-width: 9rem; }
  .file-symbol { font-weight: 900; color: var(--ok, #176c33); }
  .missing .file-symbol { color: var(--danger, #9d2424); }
  .file-word { color: var(--muted, #4b5965); font-size: .85rem; font-weight: 700; }
  .missing .file-word { color: var(--danger, #9d2424); }
  .path { flex: 1 1 18rem; min-width: 0; font-family: ui-monospace, Consolas, monospace; font-size: .88rem; overflow-wrap: anywhere; user-select: text; }
  .note { flex: 1 1 100%; color: var(--ink, #17222e); overflow-wrap: anywhere; }
  .file-actions { display: inline-flex; flex-wrap: wrap; gap: .4rem; }
  .reveal {
    min-height: 2.25rem;
    padding: .35rem .7rem;
    border: 1px solid var(--wvu-navy, #002855);
    border-radius: .3rem;
    background: var(--surface, #fff);
    color: var(--wvu-navy, #002855);
    font: inherit;
    font-weight: 650;
    cursor: pointer;
  }
  .reveal:hover { background: var(--wvu-navy, #002855); color: #fff; }
  .result-facts { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 9rem), 1fr)); gap: .45rem; margin: .75rem 0 0; }
  .result-facts div { min-width: 0; padding: .45rem .65rem; border: 1px solid var(--border, #d7dde2); border-radius: .3rem; }
  .result-facts dt { color: var(--muted, #4b5965); font-size: .78rem; font-weight: 700; text-transform: uppercase; letter-spacing: .03em; }
  .result-facts dd { margin: 0; font-weight: 700; font-variant-numeric: tabular-nums; }
  .quality { margin: .6rem 0 0; font-weight: 600; }
  .quality.attention { color: #6b4700; }
  .result-details { margin-top: .6rem; }
  .result-details summary { cursor: pointer; color: var(--wvu-navy, #002855); font-weight: 700; }
  .result-details p { margin: .35rem 0; overflow-wrap: anywhere; }
</style>
