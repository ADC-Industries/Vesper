<script lang="ts">
  // The Library's media tools (plan 4.2, round 4), under the media column's search: the Import
  // panel (files or a whole folder, into a chosen manual bin; dropping files on the column works
  // too), the All | Missing filter, and for missing files "Find missing files" and "Match a
  // folder on this computer". Uploads go to the Codex box (plan 2.5).
  import Upload from "lucide-svelte/icons/upload";
  import FolderOpen from "lucide-svelte/icons/folder-open";
  import SearchCheck from "lucide-svelte/icons/search-check";
  import FolderSearch from "lucide-svelte/icons/folder-search";
  import { Button, Segmented, Select } from "../../ui";
  import { libraryPage } from "../../state/library.svelte";
  import { flattenBins, mediaState } from "../../state/media.svelte";
  import { mediaFiles } from "./upload";

  let { importOpen = $bindable(false), readOnly, missing }: { importOpen?: boolean; readOnly: boolean; missing: number } = $props();

  let bins = $derived(flattenBins(mediaState.mediaBins).filter((b) => b.kind === "manual" && b.browser === "media" && b.id));
  let binOptions = $derived([{ value: "", label: "No bin (library only)" }, ...bins.map((b) => ({ value: b.id!, label: [...b.path, b.name].join(" › ") }))]);
  let bin = $state("");
  $effect(() => {
    libraryPage.importBin = bin || null;
  });

  let filesEl = $state<HTMLInputElement | null>(null);
  let folderEl = $state<HTMLInputElement | null>(null);
  let matchEl = $state<HTMLInputElement | null>(null);

  function picked(e: Event & { currentTarget: HTMLInputElement }, match = false) {
    const list = mediaFiles(e.currentTarget.files ?? []);
    e.currentTarget.value = "";
    if (match) void libraryPage.matchFolder(list);
    else {
      libraryPage.importFiles(list);
      importOpen = false;
    }
  }

  const filters = $derived([
    { value: "all" as const, label: "All" },
    { value: "missing" as const, label: missing ? `Missing (${missing})` : "Missing" },
  ]);
</script>

<input bind:this={filesEl} type="file" multiple accept="image/*,video/*,audio/*" hidden onchange={(e) => picked(e)} />
<input bind:this={folderEl} type="file" webkitdirectory multiple hidden onchange={(e) => picked(e)} />
<input bind:this={matchEl} type="file" webkitdirectory multiple hidden onchange={(e) => picked(e, true)} />

{#if importOpen}
  <div class="import" role="group" aria-label="Import media">
    <div class="line">
      <span class="lbl">Into bin</span>
      <Select label="Bin the new files go into" options={binOptions} bind:value={bin} style="flex: 1; min-width: 0" />
    </div>
    <div class="line">
      <Button size="sm" variant="primary" icon={Upload} disabled={readOnly} onclick={() => filesEl?.click()}>Choose files…</Button>
      <Button size="sm" variant="secondary" icon={FolderOpen} disabled={readOnly} onclick={() => folderEl?.click()}>Choose a folder…</Button>
    </div>
    <p class="hint">Or drop files on this column. Import copies them into the Codex, so every Vesper of the site has them. Big files carry on after a dropped connection.</p>
  </div>
{/if}

<div class="show">
  <Segmented size="sm" label="Show" options={filters} bind:value={libraryPage.mediaFilter} />
</div>
{#if libraryPage.mediaFilter === "missing"}
  <div class="actions">
    <Button size="sm" variant="secondary" icon={SearchCheck} disabled={readOnly || !missing || libraryPage.busy} onclick={() => libraryPage.findMissing()}>Find missing files</Button>
    <Button size="sm" variant="secondary" icon={FolderSearch} disabled={readOnly || !missing || libraryPage.busy} onclick={() => matchEl?.click()}>Match a folder on this computer…</Button>
  </div>
{/if}

<style>
  .import {
    margin: 0 12px 8px;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-radius: var(--r-card);
    border: 1px solid var(--accent);
    background: var(--surface-raised);
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .lbl {
    font-size: var(--fs-sm);
    color: var(--text-label);
    white-space: nowrap;
  }
  .hint {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-meta);
    line-height: 1.4;
  }
  .show {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px 8px;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 0 12px 8px;
  }
</style>
