<script lang="ts">
  // Inspector, Arrange tab (several objects selected): duplicate and delete (at the top, so the
  // docked object list never covers them on a short window), align to each other and on the
  // slide, distribute, match size and order. Their fill, stroke and so on follow (InspShape,
  // applied to every selected object).
  import BringToFront from "lucide-svelte/icons/bring-to-front";
  import SendToBack from "lucide-svelte/icons/send-to-back";
  import ArrowUpToLine from "lucide-svelte/icons/arrow-up-to-line";
  import ArrowDownToLine from "lucide-svelte/icons/arrow-down-to-line";
  import type { EditState } from "../../edit/editState.svelte";
  import Button from "../../ui/Button.svelte";
  import AlignBar from "./AlignBar.svelte";
  import { matchSizeOps, orderOps, type OrderMove } from "./arrange";
  import { deleteSelection, duplicateSelection } from "./keys";

  let { st }: { st: EditState } = $props();

  let sel = $derived(st.selected);
  let locked = $derived(sel.filter((e) => e.locked).length);

  function match(dim: "width" | "height" | "both") {
    const slide = st.slideId;
    if (slide) void st.run("Match Size", matchSizeOps(slide, sel, dim));
  }
  function order(move: OrderMove, label: string) {
    const slide = st.slide;
    if (slide) void st.run(label, orderOps(slide, st.selection, move));
  }
  const ORDER: { move: OrderMove; label: string; icon: typeof BringToFront }[] = [
    { move: "front", label: "Bring to Front", icon: BringToFront },
    { move: "forward", label: "Bring Forward", icon: ArrowUpToLine },
    { move: "backward", label: "Send Backward", icon: ArrowDownToLine },
    { move: "back", label: "Send to Back", icon: SendToBack },
  ];
</script>

<section class="sec">
  <h4 class="ename">{sel.length} objects selected</h4>
  <p class="note">Shift- or Ctrl-click objects to add or remove them, or drag a box around them on the slide. Arrow keys move them together.{#if locked}&nbsp;Locked objects stay where they are.{/if}</p>
  <div class="btn2">
    <Button size="sm" onclick={() => void duplicateSelection(st)}>Duplicate all</Button>
    <Button size="sm" variant="danger" onclick={() => void deleteSelection(st)}>Delete all</Button>
  </div>
</section>

<section class="sec" data-arrange>
  <span class="label">Align to each other</span>
  <AlignBar {st} />
  <span class="label">Align on the slide</span>
  <AlignBar {st} toSlide />
  <span class="label">Match size</span>
  <div class="three">
    <Button size="sm" onclick={() => match("width")} title="Same width as the first selected object">Same width</Button>
    <Button size="sm" onclick={() => match("height")} title="Same height as the first selected object">Same height</Button>
    <Button size="sm" onclick={() => match("both")} title="Same size as the first selected object">Same size</Button>
  </div>
  <span class="label">Order</span>
  <div class="seg" role="group" aria-label="Order">
    {#each ORDER as o (o.move)}
      {@const Icon = o.icon}
      <button onclick={() => order(o.move, o.label)} title={o.label} aria-label={o.label}><Icon size={16} strokeWidth={1.8} /></button>
    {/each}
  </div>
</section>

<style>
  .three {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 6px;
  }
  .seg {
    align-self: flex-start;
  }
</style>
