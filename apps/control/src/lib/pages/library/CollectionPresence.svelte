<script lang="ts">
  // "2 others have this collection open" (the Library board's connection card, G8): who else has
  // the Library page open on this collection, and what each is doing ("editing Morning Light",
  // "browsing"); the list opens on a click. Nothing when nobody else has it open.
  import Users from "lucide-svelte/icons/users";
  import type { DocumentKind } from "$protocol";
  import { presence } from "../../state/presence.svelte";
  import { collectionSummary } from "../../state/presenceLogic";
  import { libraryPage } from "../../state/library.svelte";
  import { timersState } from "../../state/timers.svelte";
  import { Popover, StatusDot } from "../../ui";

  let { collection }: { collection: string | null } = $props();

  function nameOf(kind: DocumentKind, id: string): string | null {
    const named = (xs: { id: string; name?: string | null }[]) => xs.find((x) => x.id === id)?.name || null;
    switch (kind) {
      case "presentation":
        return named(libraryPage.songs);
      case "theme":
        return named(libraryPage.themes);
      case "prop":
        return named(libraryPage.props);
      case "timer":
        return named(timersState.timerDocs);
      default:
        return null;
    }
  }

  let others = $derived(presence.collectionOthers(collection, nameOf));
  let summary = $derived(collectionSummary(others));
  let open = $state(false);
</script>

{#if others.length}
  <span class="anchor">
    <button type="button" class="who-btn" aria-expanded={open} onclick={() => (open = !open)} data-collection-presence>
      <Users size={13} strokeWidth={2} aria-hidden="true" />
      <span>{summary}</span>
    </button>
    {#if open}
      <Popover label={summary} width={232} align="left" side="above" onclose={() => (open = false)}>
        <div class="list">
          <p class="cap">{summary}</p>
          {#each others as o (o.key)}
            <div class="row">
              <StatusDot status={o.editing ? "info" : "ok"} />
              <span class="name">{o.label}</span>
              <span class="doing">{o.doing}</span>
            </div>
          {/each}
        </div>
      </Popover>
    {/if}
  </span>
{/if}

<style>
  .anchor {
    position: relative;
    display: flex;
  }
  .who-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 2px 0;
    border: 0;
    background: none;
    color: var(--text-2);
    font: inherit;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-align: left;
    cursor: pointer;
  }
  .who-btn:hover {
    color: var(--text);
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
  }
  .cap {
    margin: 0 0 2px;
    color: var(--text-label);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .row {
    display: grid;
    grid-template-columns: auto 1fr;
    column-gap: 8px;
    align-items: center;
  }
  .name {
    font-size: var(--fs-sm);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .doing {
    grid-column: 2;
    color: var(--text-meta);
    font-size: var(--fs-xs);
  }
</style>
