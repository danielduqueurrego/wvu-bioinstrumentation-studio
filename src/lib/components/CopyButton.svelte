<script lang="ts">
  import { tick } from 'svelte';
  import { copyText } from '$lib/clipboard';

  /**
   * Copies text with the Clipboard API. If the clipboard is unavailable the
   * text is shown in a read-only field, selected and focused, so Ctrl+C
   * copies it. Feedback goes to the page's one live region (`onCopied`).
   */
  type Props = {
    text: string;
    label: string;
    /** Accessible name when several copy buttons sit side by side. */
    ariaLabel?: string;
    /** Message for the page's information line after a successful copy. */
    copiedMessage?: string;
    multiline?: boolean;
    onCopied?: (message: string) => void;
  };

  let { text, label, ariaLabel, copiedMessage = 'Copied.', multiline = false, onCopied = () => {} }: Props = $props();

  let fallbackText = $state('');
  let fallback = $state<HTMLInputElement | HTMLTextAreaElement | undefined>();
  const uid = $props.id();
  const fieldId = `copy-fallback-${uid}`;

  async function copy() {
    if (await copyText(text)) {
      fallbackText = '';
      onCopied(copiedMessage);
      return;
    }
    fallbackText = text;
    await tick();
    fallback?.focus();
    fallback?.select();
    onCopied('Copying is not available here. The text is selected below; press Ctrl+C to copy it.');
  }
</script>

<span class="copy">
  <button type="button" class="copy-button" aria-label={ariaLabel} onclick={() => void copy()}>{label}</button>
  {#if fallbackText}
    <span class="copy-fallback">
      <label for={fieldId}>Press Ctrl+C to copy</label>
      {#if multiline}
        <textarea id={fieldId} bind:this={fallback} readonly rows="6" value={fallbackText}></textarea>
      {:else}
        <input id={fieldId} bind:this={fallback} readonly value={fallbackText} />
      {/if}
    </span>
  {/if}
</span>

<style>
  .copy { display: contents; }
  .copy-button {
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
  .copy-button:hover { background: var(--wvu-navy, #002855); color: #fff; }
  .copy-fallback { display: grid; flex: 1 1 100%; gap: .25rem; min-width: 0; font-weight: 600; }
  .copy-fallback input, .copy-fallback textarea {
    width: 100%;
    min-width: 0;
    padding: .35rem .5rem;
    border: 1px solid var(--border-strong, #8493a0);
    border-radius: .25rem;
    font: .88rem ui-monospace, Consolas, monospace;
  }
</style>
