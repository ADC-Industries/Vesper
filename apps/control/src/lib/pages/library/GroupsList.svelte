<script lang="ts">
  // The Library's Design ▸ Groups (G17): the collection's global groups (Verse, Chorus, …: the
  // names and colours the song editor and Reflow offer for sections) in their order, with
  // colour, rename, move up / down and delete. Songs keep their sections as they are when a
  // group changes.
  import Pencil from "lucide-svelte/icons/pencil";
  import ArrowUp from "lucide-svelte/icons/arrow-up";
  import ArrowDown from "lucide-svelte/icons/arrow-down";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import { Button, ColorWell } from "../../ui";
  import { binsPage } from "../../state/bins.svelte";
  import { session } from "../../state/session.svelte";
  import { colorFromHex, groupMove, hexOf } from "./bins";

  let { readOnly }: { readOnly: boolean } = $props();
  let locked = $derived(readOnly || binsPage.busy);
  let groups = $derived(session.globalGroups);
</script>

<p class="note">The section names and colours songs pick from. Changing a group doesn't change songs that already use it.</p>
<div class="list" role="list" aria-label="Groups">
  {#each groups as g (g.id)}
    <div class="row" role="listitem">
      <ColorWell label={`Colour of ${g.name}`} value={hexOf(g.color)} disabled={locked} onchange={(v) => binsPage.recolorGroup(g, colorFromHex(v))} />
      <span class="name">{g.name || "Untitled"}</span>
      {#if g.hotkey?.code}<span class="key">{g.hotkey.code.replace(/^Key|^Digit/, "")}</span>{/if}
      <Button size="sm" variant="ghost" icon={Pencil} iconOnly aria-label={`Rename ${g.name}`} title="Rename" disabled={locked} onclick={() => binsPage.renameGroup(g)} />
      <Button size="sm" variant="ghost" icon={ArrowUp} iconOnly aria-label={`Move ${g.name} up`} title="Move up" disabled={locked || !groupMove(groups, g.id, -1)} onclick={() => binsPage.moveGroup(g, -1)} />
      <Button size="sm" variant="ghost" icon={ArrowDown} iconOnly aria-label={`Move ${g.name} down`} title="Move down" disabled={locked || !groupMove(groups, g.id, 1)} onclick={() => binsPage.moveGroup(g, 1)} />
      <Button size="sm" variant="ghost" icon={Trash2} iconOnly aria-label={`Delete ${g.name}`} title="Delete" disabled={locked} onclick={() => binsPage.deleteGroup(g)} />
    </div>
  {:else}
    <p class="empty">No groups yet. “+ New group” adds one.</p>
  {/each}
</div>

<style>
  .note {
    margin: 0;
    padding: 4px 16px 8px;
    font-size: var(--fs-xs);
    color: var(--text-meta);
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    border-top: 1px solid var(--surface-raised);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    border-bottom: 1px solid var(--surface-raised);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }
  .key {
    padding: 0 6px;
    border: 1px solid var(--line-strong);
    border-radius: 4px;
    font-size: var(--fs-xs);
    color: var(--text-meta);
  }
  .empty {
    margin: 0;
    padding: 16px;
    color: var(--text-label);
  }
</style>
