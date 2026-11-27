<script lang="ts">
  // The edit lock banner (G8, the Library board's "X has this song open for editing … Take
  // over editing"): someone else is editing the document, so this client looks at it read-only
  // and may take over; or this client lost the lock to someone (it says who), with its typing
  // that wasn't saved yet kept: take editing back to save it, or copy it.
  import Lock from "lucide-svelte/icons/lock";
  import type { DocumentKind } from "$protocol";
  import { presence } from "../state/presence.svelte";
  import { personLabel } from "../state/presenceLogic";
  import { Button } from "../ui";

  let {
    kind,
    id,
    noun,
    kept = 0,
    oncopy,
    ontaken,
    canTake = true,
  }: {
    kind: DocumentKind;
    id: string | null | undefined;
    /** "song", "theme", "stage layout" … */
    noun: string;
    /** Slides with typing kept (not saved) because the lock was lost. */
    kept?: number;
    /** Copies the kept typing (then lets it go). */
    oncopy?: () => void;
    /** This client holds the lock (again). */
    ontaken?: () => void;
    /** This place can edit the document: offers "Take over editing" (D09-6, E09-1: the
     *  Library can't edit themes or props, so taking the lock there would only stop the real
     *  editor). */
    canTake?: boolean;
  } = $props();

  let holder = $derived(presence.lockedBy(kind, id));
  let lost = $derived(presence.lostTo(kind, id));
  let busy = $state(false);

  async function take() {
    if (!id) return;
    busy = true;
    const ok = await presence.takeOver(kind, id);
    busy = false;
    if (ok) ontaken?.();
  }
</script>

{#if holder && id}
  {@const who = personLabel(holder)}
  <div class="lock" role="status" data-lock-banner={lost ? "lost" : "held"}>
    <Lock size={14} strokeWidth={2} aria-hidden="true" />
    <span class="txt">
      {#if lost || kept}
        <b>{who}</b> took over editing this {noun}.
        {#if kept}
          What you typed on {kept === 1 ? "1 slide" : `${kept} slides`} wasn't saved and is kept here: take editing back to save it, or copy it.
        {:else}
          You can look at it, or take editing back.
        {/if}
      {:else if !canTake}
        <b>{who}</b> has this {noun} open for editing. Their changes show here as they save them.
      {:else}
        <b>{who}</b> has this {noun} open for editing. You can look at it, or take over.
      {/if}
    </span>
    {#if canTake || lost || kept}
      <Button size="sm" variant="primary" disabled={busy} onclick={take}>{lost || kept ? "Take back editing" : "Take over editing"}</Button>
    {/if}
    {#if kept && oncopy}
      <Button size="sm" variant="outline" onclick={oncopy}>Copy my typing</Button>
    {:else if lost}
      <Button size="sm" variant="ghost" onclick={() => presence.dismissLost(kind, id)}>OK</Button>
    {/if}
  </div>
{/if}

<style>
  .lock {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--info) 35%, transparent);
    background: color-mix(in srgb, var(--info) 12%, var(--surface-panel));
    color: var(--info-text);
    font-size: var(--fs-sm);
  }
  .lock :global(svg) {
    flex: none;
  }
  .txt {
    flex: 1;
    min-width: 0;
    color: var(--text-2);
  }
  b {
    color: var(--text);
    font-weight: 600;
  }
</style>
