<script lang="ts">
  // Align buttons: the selection to each other (with distribute), or to the slide.
  import AlignStartVertical from "lucide-svelte/icons/align-start-vertical";
  import AlignCenterVertical from "lucide-svelte/icons/align-center-vertical";
  import AlignEndVertical from "lucide-svelte/icons/align-end-vertical";
  import AlignStartHorizontal from "lucide-svelte/icons/align-start-horizontal";
  import AlignCenterHorizontal from "lucide-svelte/icons/align-center-horizontal";
  import AlignEndHorizontal from "lucide-svelte/icons/align-end-horizontal";
  import AlignHorizontalSpaceAround from "lucide-svelte/icons/align-horizontal-space-around";
  import AlignVerticalSpaceAround from "lucide-svelte/icons/align-vertical-space-around";
  import type { EditState } from "../../edit/editState.svelte";
  import type { AlignMode, DistributeMode } from "../../edit/geometry";
  import { alignOps, distributeOps } from "./arrange";

  let { st, toSlide = false }: { st: EditState; toSlide?: boolean } = $props();

  let sel = $derived(st.selected);
  let free = $derived(sel.filter((e) => !e.locked));
  let canAlign = $derived(toSlide ? free.length > 0 : sel.length > 1 && free.length > 0);
  let canSpread = $derived(free.length >= 3);
  const where = $derived(toSlide ? " on the slide" : "");

  const H: { mode: AlignMode; icon: typeof AlignStartVertical; label: string }[] = [
    { mode: "left", icon: AlignStartVertical, label: "Align left" },
    { mode: "centre", icon: AlignCenterVertical, label: "Align centre" },
    { mode: "right", icon: AlignEndVertical, label: "Align right" },
  ];
  const V: { mode: AlignMode; icon: typeof AlignStartVertical; label: string }[] = [
    { mode: "top", icon: AlignStartHorizontal, label: "Align top" },
    { mode: "middle", icon: AlignCenterHorizontal, label: "Align middle" },
    { mode: "bottom", icon: AlignEndHorizontal, label: "Align bottom" },
  ];

  function doAlign(mode: AlignMode) {
    const slide = st.slideId;
    if (slide) void st.run("Align", alignOps(slide, sel, mode, toSlide ? st.canvasSize() : undefined));
  }
  function doSpread(mode: DistributeMode) {
    const slide = st.slideId;
    if (slide) void st.run("Distribute", distributeOps(slide, sel, mode));
  }
</script>

<div class="alignbar" role="toolbar" aria-label={toSlide ? "Align on the slide" : "Align to each other"} data-align-bar={toSlide ? "slide" : "each"}>
  {#each [H, V] as group, gi (gi)}
    <div class="seg" role="group" aria-label={gi ? `Align vertically${where}` : `Align horizontally${where}`}>
      {#each group as b (b.mode)}
        {@const Icon = b.icon}
        <button onclick={() => doAlign(b.mode)} disabled={!canAlign} title={`${b.label}${where}`} aria-label={`${b.label}${where}`}><Icon size={16} strokeWidth={1.8} /></button>
      {/each}
    </div>
  {/each}
  {#if !toSlide}
    <div class="seg" role="group" aria-label="Distribute">
      <button onclick={() => doSpread("horizontal")} disabled={!canSpread} title={canSpread ? "Distribute horizontally" : "Select three or more to distribute"} aria-label="Distribute horizontally"><AlignHorizontalSpaceAround size={16} strokeWidth={1.8} /></button>
      <button onclick={() => doSpread("vertical")} disabled={!canSpread} title={canSpread ? "Distribute vertically" : "Select three or more to distribute"} aria-label="Distribute vertically"><AlignVerticalSpaceAround size={16} strokeWidth={1.8} /></button>
    </div>
  {/if}
</div>

<style>
  .alignbar {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .alignbar :global(.seg button:disabled) {
    opacity: 0.45;
  }
  .alignbar :global(.seg button:hover:not(:disabled)) {
    background: var(--surface-overlay);
    color: var(--text);
  }
</style>
