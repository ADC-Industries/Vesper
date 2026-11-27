<script lang="ts">
  // Inspector, theme mode: the role (slot) the selected object fills in its template slide:
  // lyrics, title, scripture, reference, or none (decoration that every song following the
  // template shows as it is). Songs pour their text into the slots (docs/THEMES-PLAN.md).
  import type { Element, Slide } from "$protocol";
  import type { EditState } from "../../edit/editState.svelte";
  import { SLOT_ROLES, placeholderFor } from "../../edit/themeTemplates";

  let { st, slide, el }: { st: EditState; slide: Slide; el: Element } = $props();

  let cur = $derived(el.placeholder ? (el.placeholder.slot ?? "") : "");
  let isText = $derived(el.kind?.type === "text");
  let value = $derived(el.placeholder ? cur || (isText ? "" : "picture") : "");
  let roles = $derived(
    isText
      ? [...SLOT_ROLES, ...(cur && !SLOT_ROLES.some((r) => r.slot === cur) ? [{ slot: cur, label: cur }] : [])]
      : [{ slot: "", label: "None (decoration)" }, { slot: cur || "picture", label: "Picture" }],
  );

  function set(slot: string) {
    if (slot === value) return;
    void st.run("Slot Role", [{ type: "setElement", slide: slide.id, element: el.id, patch: { placeholder: placeholderFor(el, slot) } }]);
  }
</script>

<section class="sec" data-testid="slot-role">
  <h4>Role in the template</h4>
  <select class="wide" {value} onchange={(e) => set(e.currentTarget.value)} aria-label="Role in the template">
    {#each roles as r (r.slot)}<option value={r.slot}>{r.label}</option>{/each}
  </select>
  <p class="note">
    {#if el.placeholder}Songs that follow this template slide show their {isText ? "text" : "picture"} here.{:else}Shown as it is on every song that follows this template slide.{/if}
  </p>
</section>

<style>
  .note {
    margin: 6px 0 0;
    font-size: 12px;
    color: var(--text-meta);
  }
</style>
