<script lang="ts">
  // The header's Look picker: the box's looks (`GET /looks`) and "Every layer" (`null`),
  // wired to `setLook`.
  import ChevronDown from "lucide-svelte/icons/chevron-down";
  import Menu from "../ui/Menu.svelte";
  import Popover from "../ui/Popover.svelte";
  import type { MenuItem } from "../ui/types";
  import { useLive } from "../live";
  import { session } from "../state/session.svelte";
  import { shell } from "../state/shell.svelte";

  const live = useLive();
  const activeLook = live.activeLook;

  const EVERY = "Every layer";
  let name = $derived($activeLook === null ? EVERY : (session.looks.find((l) => l.id === $activeLook)?.name ?? "Look"));
  let items = $derived<MenuItem[]>([
    ...session.looks.map((l) => ({ id: l.id, label: l.name ?? "Unnamed look", checked: $activeLook === l.id })),
    ...(session.looks.length ? [{ id: "sep", separator: true }] : []),
    { id: "", label: EVERY, checked: $activeLook === null },
  ]);
  let open = $derived(shell.popover === "look");

  function choose(id: string) {
    shell.popover = null;
    void session.run({ type: "setLook", look: id || null });
  }
</script>

<div class="look">
  <span class="lbl" id="look-label">Look</span>
  <button
    class="pick"
    class:open
    aria-haspopup="menu"
    aria-expanded={open}
    aria-labelledby="look-label look-name"
    title="The look on the audience screens"
    onclick={() => shell.togglePopover("look")}
  >
    <span id="look-name">{name}</span><ChevronDown size={14} strokeWidth={1.8} />
  </button>
  {#if open}
    <Popover align="right" width={240} role="none" onclose={() => (shell.popover = null)}>
      <Menu label="Looks" {items} onselect={choose} onclose={() => (shell.popover = null)} />
    </Popover>
  {/if}
</div>

<style>
  .look {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-label);
  }
  .pick {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 120px;
    justify-content: space-between;
    padding: 6px 8px;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text);
    font-size: var(--fs);
  }
  .pick:hover,
  .pick.open {
    border-color: var(--line-strong);
  }
  .pick :global(svg) {
    color: var(--text-label);
  }
</style>
