<script lang="ts">
  // A draggable divider (WAI-ARIA window splitter): 1 px line, 6 px hit area that glows violet.
  // `dir="col"` sits between columns (drag left/right), `dir="row"` between stacked panels.
  // Reports the new size through `onchange`: the start value plus the pointer movement (times
  // `scale`, negative when the panel after the divider grows). Arrow keys move it 10 px (Shift:
  // 50), Home / End go to the bounds, double-click resets it to `defaultValue`.
  import { clamp, splitterKey } from "./logic";

  let {
    dir,
    value,
    onchange,
    scale = 1,
    label,
    min,
    max,
    defaultValue,
  }: {
    dir: "col" | "row";
    value: number;
    onchange: (v: number) => void;
    scale?: number;
    label: string;
    min?: number;
    max?: number;
    defaultValue?: number;
  } = $props();

  let dragging = $state(false);

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    dragging = true;
    const start = dir === "col" ? e.clientX : e.clientY;
    const v0 = value;
    const move = (ev: PointerEvent) => {
      const d = (dir === "col" ? ev.clientX : ev.clientY) - start;
      onchange(clamp(v0 + d * scale, min, max));
    };
    const up = () => {
      dragging = false;
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
    e.preventDefault();
  }

  function key(e: KeyboardEvent) {
    const v = splitterKey(e.key, e.shiftKey, value, { dir, scale, min, max });
    if (v === null) return;
    e.preventDefault();
    e.stopPropagation();
    onchange(v);
  }
</script>

<!-- A focusable separator is a widget (WAI-ARIA "window splitter"). -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="ui-splitter {dir}"
  class:dragging
  role="separator"
  aria-orientation={dir === "col" ? "vertical" : "horizontal"}
  aria-label={label}
  aria-valuenow={Math.round(value)}
  aria-valuemin={min}
  aria-valuemax={max}
  tabindex="0"
  onpointerdown={down}
  onkeydown={key}
  ondblclick={() => defaultValue !== undefined && onchange(defaultValue)}
></div>

<style>
  .ui-splitter {
    position: relative;
    flex: none;
    background: var(--line);
    z-index: 5;
    touch-action: none;
    outline: none;
  }
  .ui-splitter::after {
    content: "";
    position: absolute;
    border-radius: 3px;
    transition: background-color 0.12s;
  }
  .col {
    width: 1px;
    cursor: col-resize;
  }
  .col::after {
    inset: 0 -3px 0 -2px;
  }
  .row {
    height: 1px;
    cursor: row-resize;
  }
  .row::after {
    inset: -3px 0 -2px 0;
  }
  .ui-splitter:hover::after,
  .ui-splitter.dragging::after,
  .ui-splitter:focus-visible::after {
    background: var(--accent);
  }
</style>
