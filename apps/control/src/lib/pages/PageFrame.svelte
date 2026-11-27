<script lang="ts">
  // A full-screen page: its toolbar row (name, controls, Done or Close, back to where the
  // operator came from) over the page's content. Used by every page but Show.
  import type { Snippet } from "svelte";
  import PageToolbar from "../ui/PageToolbar.svelte";
  import { shell } from "../state/shell.svelte";
  import { pageDef, type PageId } from "../shell/pages";

  let { page, subtitle, controls, children }: { page: PageId; subtitle?: string; controls?: Snippet; children: Snippet } = $props();

  let def = $derived(pageDef(page));
</script>

<section class="page" aria-label={def.title}>
  <PageToolbar
    title={def.title}
    {subtitle}
    doneLabel={def.closeLabel}
    doneVariant={def.closeLabel === "Done" ? "primary" : "outline"}
    ondone={() => shell.close()}
    children={controls}
  />
  <div class="content">{@render children()}</div>
</section>

<style>
  .page {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--surface-app);
  }
  .content {
    flex: 1;
    min-height: 0;
    display: flex;
    overflow: auto;
  }
</style>
