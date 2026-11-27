<script lang="ts">
  // "2 others have this open" (G8): a pill next to a document's title, naming who else has it
  // open and where; the list opens on a click. Nothing when nobody else has it open.
  import Users from "lucide-svelte/icons/users";
  import type { DocumentKind } from "$protocol";
  import { presence } from "../state/presence.svelte";
  import { othersSummary } from "../state/presenceLogic";
  import { Popover, StatusDot } from "../ui";

  let { kind, id }: { kind: DocumentKind; id: string | null | undefined } = $props();

  let others = $derived(presence.others(kind, id));
  let open = $state(false);
  let summary = $derived(othersSummary(others));
  let short = $derived(others.length === 1 ? others[0]!.label : `${others.length} others`);
</script>

{#if others.length}
  <span class="anchor">
    <button
      type="button"
      class="pill"
      class:editing={others.some((o) => o.editing)}
      title={others.map((o) => `${o.label}: ${o.editing ? "editing" : "viewing"} ${o.where}`).join("\n")}
      aria-label={summary}
      aria-expanded={open}
      onclick={() => (open = !open)}
      data-presence-badge
    >
      <Users size={13} strokeWidth={2} aria-hidden="true" />
      {short}
    </button>
    {#if open}
      <Popover label={summary} width={260} align="right" onclose={() => (open = false)}>
        <div class="who">
          <p class="cap">{summary}</p>
          {#each others as o (o.key)}
            <div class="row">
              <StatusDot status={o.editing ? "info" : "ok"} />
              <span class="name">{o.label}</span>
              <span class="where">{o.editing ? "editing" : "viewing"} {o.where}</span>
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
    display: inline-flex;
    flex: none;
  }
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 9px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r-pill);
    background: var(--surface-sunk);
    color: var(--text-2);
    font: inherit;
    font-size: var(--fs-xs);
    font-weight: 600;
    white-space: nowrap;
    cursor: pointer;
  }
  .pill:hover {
    border-color: var(--line-hover);
  }
  .pill.editing {
    color: var(--info-text);
    border-color: color-mix(in srgb, var(--info) 45%, transparent);
  }
  .who {
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
  .where {
    grid-column: 2;
    color: var(--text-meta);
    font-size: var(--fs-xs);
  }
</style>
