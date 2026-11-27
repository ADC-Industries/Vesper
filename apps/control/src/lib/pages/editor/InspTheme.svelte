<script lang="ts">
  // Inspector: how the slide (or the selected object) follows its theme (linked themes,
  // docs/THEMES-PLAN.md). The properties it sets itself are listed with an override dot, and
  // "Reset to theme" clears them (the server keeps the overrides; edits record them).
  import RotateCcw from "lucide-svelte/icons/rotate-ccw";
  import type { Element, Slide } from "$protocol";
  import Button from "../../ui/Button.svelte";
  import type { EditState } from "../../edit/editState.svelte";
  import { overrideLabel, slideOverrides } from "../library/logic";

  let { st, slide, el = null }: { st: EditState; slide: Slide; el?: Element | null } = $props();

  let linked = $derived(!!slide.themeLink && !!slide.templateOrigin);
  let follows = $derived(el ? !!el.fromTheme : linked);
  let themeName = $derived(slide.templateOrigin?.theme.name ?? "its theme");
  let overrides = $derived(linked ? (el ? slideOverrides(slide, el.id) : slideOverrides(slide)) : []);

  function reset() {
    void st.run("Reset to Theme", [{ type: "resetToTheme", slide: slide.id, ...(el ? { element: el.id } : {}) }]);
  }
</script>

{#if linked}
  <section class="sec theme" data-testid="theme-link">
    <div class="line between">
      <h4>{el ? (follows ? `From the theme “${themeName}”` : "This slide's own object") : `Follows the theme “${themeName}”`}</h4>
      {#if follows && overrides.length}
        <Button size="sm" variant="outline" icon={RotateCcw} onclick={reset} title={el ? "Use the theme's settings for this object again" : "Use the theme's settings for the whole slide again"}>Reset to theme</Button>
      {/if}
    </div>
    {#if follows}
      {#if overrides.length}
        <ul class="over" aria-label="Changed on this {el ? 'object' : 'slide'}">
          {#each overrides as o (o)}<li><span class="dot" aria-hidden="true"></span>{overrideLabel(o)}</li>{/each}
        </ul>
      {:else}
        <p class="note">Everything comes from the theme: changing the theme changes this {el ? "object" : "slide"}.</p>
      {/if}
    {:else}
      <p class="note">The theme doesn't touch it.</p>
    {/if}
  </section>
{/if}

<style>
  .over {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .over li {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border-radius: 10px;
    background: var(--surface-sunk);
    font-size: 12px;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--live);
  }
  .note {
    margin: 0;
    font-size: 12px;
    color: var(--text-meta);
  }
</style>
