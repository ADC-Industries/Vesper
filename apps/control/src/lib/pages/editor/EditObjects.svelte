<script lang="ts">
  // The slide's objects (docked in the inspector), topmost first, with type icons. Click selects
  // (Shift / Ctrl add), double-click renames, drag reorders (reorderElement), and each row has
  // hide and lock toggles. The header has Bring forward, Send backward, Duplicate and Delete for
  // the selection.
  import Type from "lucide-svelte/icons/type";
  import Square from "lucide-svelte/icons/square";
  import Circle from "lucide-svelte/icons/circle";
  import Triangle from "lucide-svelte/icons/triangle";
  import Minus from "lucide-svelte/icons/minus";
  import ImageIcon from "lucide-svelte/icons/image";
  import Box from "lucide-svelte/icons/box";
  import Eye from "lucide-svelte/icons/eye";
  import EyeOff from "lucide-svelte/icons/eye-off";
  import Lock from "lucide-svelte/icons/lock";
  import LockOpen from "lucide-svelte/icons/lock-open";
  import ArrowUp from "lucide-svelte/icons/arrow-up";
  import ArrowDown from "lucide-svelte/icons/arrow-down";
  import Copy from "lucide-svelte/icons/copy";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import { orderOps } from "./arrange";
  import { deleteSelection, duplicateSelection } from "./keys";
  import type { Element } from "$protocol";
  import type { EditState } from "../../edit/editState.svelte";
  import { elementName, elementType } from "../../edit/elements";
  import { pressRow, moveTarget } from "../../edit/listDrag";
  import { reorderIndex } from "../../edit/ops";

  let { st }: { st: EditState } = $props();

  let els = $derived([...st.elements].reverse());
  let renaming = $state<string | null>(null);
  let listEl = $state<HTMLElement | null>(null);
  let line = $state<{ y: number; left: number; width: number } | null>(null);

  const ICONS = { text: Type, image: ImageIcon, line: Minus, ellipse: Circle, triangle: Triangle, shape: Square, other: Box };

  let none = $derived(!st.selection.length);
  let ids = $derived((st.slide?.elements ?? []).map((e) => e.id));
  let top = $derived(none || st.selection.every((id) => ids.indexOf(id) >= ids.length - st.selection.length));
  let bottom = $derived(none || st.selection.every((id) => ids.indexOf(id) < st.selection.length));
  function order(move: "forward" | "backward") {
    const slide = st.slide;
    if (slide) void st.run(move === "forward" ? "Bring Forward" : "Send Backward", orderOps(slide, st.selection, move));
  }

  function click(e: MouseEvent, el: Element) {
    if (e.shiftKey || e.ctrlKey || e.metaKey) {
      st.select(st.selection.includes(el.id) ? st.selection.filter((x) => x !== el.id) : [...st.selection, el.id]);
    } else st.select([el.id]);
  }

  function set(el: Element, label: string, patch: { hidden?: boolean; locked?: boolean; name?: string }) {
    const slide = st.slideId;
    if (!slide) return;
    void st.run(label, [{ type: "setElement", slide, element: el.id, patch }]);
  }

  function commitName(el: Element, value: string) {
    // Once only: Enter or Escape ends the rename, and the field's blur as it goes away
    // (Chromium / WebView2 send one) must not save it again or save a cancelled name.
    if (renaming !== el.id) return;
    renaming = null;
    const v = value.trim();
    if (v && v !== (el.name ?? "")) set(el, "Rename", { name: v });
  }

  function focusSelect(node: HTMLInputElement) {
    node.focus();
    node.select();
  }

  function press(e: PointerEvent, el: Element) {
    if (!listEl || renaming) return;
    if ((e.target as HTMLElement).closest(".tog")) return;
    pressRow(e, { list: listEl, rows: "[data-object-row]", line: (l) => (line = l), drop: (to) => drop(el, to) });
  }

  function drop(el: Element, to: number) {
    const from = els.findIndex((x) => x.id === el.id);
    const at = moveTarget(from, to);
    const slide = st.slideId;
    if (at === null || !slide) return;
    void st.run("Reorder", [{ type: "reorderElement", slide, element: el.id, index: reorderIndex(els.length, at) }]);
  }
</script>

<div class="objects">
  <header>
    <h2>Objects</h2>
    <button class="hb" onclick={() => order("forward")} disabled={top} title="Bring forward" aria-label="Bring forward"><ArrowUp size={15} /></button>
    <button class="hb" onclick={() => order("backward")} disabled={bottom} title="Send backward" aria-label="Send backward"><ArrowDown size={15} /></button>
    <button class="hb" onclick={() => void duplicateSelection(st)} disabled={none} title="Duplicate (Ctrl+D)" aria-label="Duplicate"><Copy size={15} /></button>
    <button class="hb" onclick={() => void deleteSelection(st)} disabled={none} title="Delete (Del)" aria-label="Delete"><Trash2 size={15} /></button>
  </header>
  <div class="list" bind:this={listEl} role="listbox" aria-label="Objects" aria-multiselectable="true" data-edit-objects>
    {#each els as el (el.id)}
      {@const Icon = ICONS[elementType(el)]}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        class="row"
        class:sel={st.selection.includes(el.id)}
        class:hid={el.hidden}
        role="option"
        tabindex="-1"
        aria-selected={st.selection.includes(el.id)}
        onclick={(e) => click(e, el)}
        ondblclick={() => (renaming = el.id)}
        onpointerdown={(e) => press(e, el)}
        data-object-row={el.id}
      >
        <span class="icon"><Icon size={17} strokeWidth={2} /></span>
        {#if renaming === el.id}
          <input
            class="name-in"
            value={el.name ?? elementName(el)}
            {@attach focusSelect}
            onkeydown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter") commitName(el, e.currentTarget.value);
              if (e.key === "Escape") renaming = null;
            }}
            onblur={(e) => commitName(el, e.currentTarget.value)}
            aria-label="Object name"
          />
        {:else}
          <span class="name">{elementName(el)}</span>
        {/if}
        <button class="tog" class:on={el.hidden} onclick={(e) => (e.stopPropagation(), set(el, el.hidden ? "Show" : "Hide", { hidden: !el.hidden }))} title={el.hidden ? "Hidden: click to show" : "Hide"} aria-label={el.hidden ? "Show object" : "Hide object"} aria-pressed={!!el.hidden}>
          {#if el.hidden}<EyeOff size={14} />{:else}<Eye size={14} />{/if}
        </button>
        <button class="tog" class:on={el.locked} onclick={(e) => (e.stopPropagation(), set(el, el.locked ? "Unlock" : "Lock", { locked: !el.locked }))} title={el.locked ? "Locked: click to unlock" : "Lock"} aria-label={el.locked ? "Unlock object" : "Lock object"} aria-pressed={!!el.locked}>
          {#if el.locked}<Lock size={14} />{:else}<LockOpen size={14} />{/if}
        </button>
      </div>
    {:else}
      <p class="none">No objects on this slide.</p>
    {/each}
  </div>
  {#if line}<div class="line" style:top={`${line.y}px`} style:left={`${line.left}px`} style:width={`${line.width}px`}></div>{/if}
</div>

<style>
  .objects {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--surface-panel);
  }
  header {
    flex: none;
    display: flex;
    align-items: center;
    gap: 2px;
    height: 34px;
    padding: 0 8px 0 12px;
    background: var(--surface-panel);
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
  }
  h2 {
    flex: 1;
    margin: 0;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-label);
  }
  .hb {
    width: 26px;
    height: 24px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-2);
  }
  .hb:hover:not(:disabled) {
    background: var(--surface-overlay);
    color: var(--text);
  }
  .hb:disabled {
    opacity: 0.4;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 2px 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 4px 0 10px;
    cursor: default;
  }
  .row:hover {
    background: var(--surface-raised);
  }
  .row:focus {
    outline: none;
  }
  .row.sel {
    background: var(--accent-sel);
  }
  .row.hid .name,
  .row.hid .icon {
    opacity: 0.45;
  }
  .icon {
    display: inline-flex;
    color: var(--text);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }
  .name-in {
    flex: 1;
    min-width: 0;
    height: 22px;
    padding: 0 4px;
    background: var(--surface-sunk);
    border: 1px solid var(--accent-ring);
    border-radius: var(--r-sm);
  }
  .tog {
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 3px;
    background: transparent;
    color: var(--text-meta);
    opacity: 0;
  }
  .row:hover .tog,
  .tog.on,
  .row.sel .tog {
    opacity: 1;
  }
  .tog.on {
    color: var(--live);
  }
  .tog:hover {
    background: var(--surface-raised);
    color: var(--text-strong);
  }
  .none {
    margin: 8px 10px;
    color: var(--text-faint);
    font-size: 12px;
  }
  .line {
    position: fixed;
    height: 2px;
    background: var(--accent-bright);
    pointer-events: none;
    z-index: 30;
  }
</style>
