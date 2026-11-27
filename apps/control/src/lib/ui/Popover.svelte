<script lang="ts">
  // A popover anchored to its parent (which needs `position: relative`). Closes on Esc and on a
  // pointer press outside it and outside its anchor (so the anchor's button can toggle it).
  import type { Snippet } from "svelte";

  let {
    onclose,
    align = "left",
    side = "below",
    width = 300,
    label,
    role = "dialog",
    children,
  }: {
    onclose: () => void;
    align?: "left" | "center" | "right";
    /** Opens under its anchor (default) or over it (an anchor at the bottom of a column). */
    side?: "below" | "above";
    width?: number;
    label?: string;
    role?: "dialog" | "menu" | "listbox" | "none";
    children: Snippet;
  } = $props();

  function attach(node: HTMLElement) {
    const anchor = node.parentElement;
    const down = (e: PointerEvent) => {
      if (anchor && !anchor.contains(e.target as Node)) onclose();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onclose();
      }
    };
    window.addEventListener("pointerdown", down, true);
    node.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("pointerdown", down, true);
      node.removeEventListener("keydown", key);
    };
  }
</script>

<div
  class="ui-popover {align}"
  class:above={side === "above"}
  role={role === "none" ? undefined : role}
  aria-label={label}
  style:width={`min(${width}px, calc(100vw - 24px))`}
  {@attach attach}
>
  {@render children()}
</div>

<style>
  .ui-popover {
    position: absolute;
    top: calc(100% + 6px);
    z-index: 60;
    max-height: calc(100vh - 120px);
    overflow: auto;
    background: var(--surface-raised);
    border: 1px solid var(--line-strong);
    border-radius: var(--r-popover);
    box-shadow: var(--shadow-menu);
    color: var(--text);
  }
  .above {
    top: auto;
    bottom: calc(100% + 6px);
  }
  .left {
    left: 0;
  }
  .right {
    right: 0;
  }
  .center {
    left: 50%;
    transform: translateX(-50%);
  }
</style>
