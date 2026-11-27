<script lang="ts">
  // A machine-translation preview's banner (edit/translatePreview.svelte.ts), shared by Reflow and
  // the slide editor: progress (model download, slides translated) with Cancel; once done, the
  // notice with Discard / Keep N slides. `compact` stacks it for the inspector's narrow column.
  import Sparkles from "lucide-svelte/icons/sparkles";
  import { MACHINE_NOTICE, translateJobText } from "../edit/language";
  import type { TranslatePreview } from "../edit/translatePreview.svelte";

  let {
    preview,
    locked = false,
    compact = false,
    where = "shown in the second column",
    onkeep,
  }: {
    preview: TranslatePreview;
    /** Someone else is editing: Keep is off. */
    locked?: boolean;
    compact?: boolean;
    /** Where the proposals are shown, for the notice. */
    where?: string;
    onkeep: () => void;
  } = $props();

  let job = $derived(preview.job);
</script>

{#if job}
  <div class="mtbar" class:compact class:ready={preview.proposing} class:bad={job.status === "failed"} role="status" data-testid="translate-banner">
    <Sparkles size={15} strokeWidth={1.8} aria-hidden="true" />
    {#if preview.proposing}
      <span class="note"><strong>{job.notice || MACHINE_NOTICE}.</strong> {translateJobText(job)} ({job.model}); {where}, not saved yet.</span>
      <span class="btns">
        <button type="button" class="btn" onclick={() => preview.discard()}>Discard</button>
        <button type="button" class="btn primary" onclick={onkeep} disabled={locked || preview.keeping}>Keep {preview.count} {preview.count === 1 ? "slide" : "slides"}</button>
      </span>
    {:else}
      <span class="note">{translateJobText(job)}</span>
      {#if job.status === "downloading" && job.downloadSize}<progress max={job.downloadSize} value={job.downloaded ?? 0}></progress>{/if}
      {#if job.status === "translating" && job.total}<progress max={job.total} value={job.done}></progress>{/if}
      <span class="btns">
        <button type="button" class="btn" onclick={() => preview.cancel()}>{preview.running ? "Cancel" : "Close"}</button>
      </span>
    {/if}
  </div>
{/if}

<style>
  .mtbar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--line);
    background: var(--surface-panel);
    color: var(--text);
    font-size: var(--fs-sm);
  }
  .mtbar.compact {
    flex-wrap: wrap;
    gap: 6px 8px;
    padding: 8px 10px;
    border: 1px solid var(--line);
    border-radius: var(--r);
  }
  .mtbar.compact .note {
    flex: 1 1 calc(100% - 30px);
  }
  .mtbar.compact .btns {
    margin-left: auto;
  }
  .mtbar.ready {
    background: #1d2c45;
    color: #dbe6f7;
  }
  .mtbar.bad {
    color: var(--danger, #f2777a);
  }
  .note {
    flex: 1;
    min-width: 0;
  }
  .btns {
    display: inline-flex;
    gap: 8px;
  }
  progress {
    width: 160px;
    max-width: 100%;
  }
  .btn {
    flex: none;
    white-space: nowrap;
    padding: 5px 12px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text);
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  .btn.primary {
    background: var(--accent, #8b7cf6);
    border-color: var(--accent, #8b7cf6);
    color: #fff;
  }
  .btn:disabled {
    opacity: 0.5;
  }
</style>
