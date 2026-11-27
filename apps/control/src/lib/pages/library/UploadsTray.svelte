<script lang="ts">
  // Uploads in progress, at the foot of the Library's middle column: one row per file with its
  // progress, state and Cancel (Retry when it failed); "Clear" takes finished ones away.
  // Uploads go on while the page is closed; a reconnect wakes the waiting ones.
  import X from "lucide-svelte/icons/x";
  import RotateCcw from "lucide-svelte/icons/rotate-ccw";
  import Upload from "lucide-svelte/icons/upload";
  import { libraryPage } from "../../state/library.svelte";
  import { overallProgress, uploadDoneText, type UploadView } from "./upload";
  import { formatBytes } from "./logic";

  let list = $derived(libraryPage.uploads);
  let going = $derived(list.filter((u) => u.phase !== "done" && u.phase !== "cancelled" && u.phase !== "failed").length);
  let finished = $derived(list.length - going);
  let total = $derived(overallProgress(list));
  let open = $state(true);

  function stateText(u: UploadView): string {
    switch (u.phase) {
      case "queued":
        return "Waiting to start";
      case "sending":
        return `${formatBytes(u.sent)} of ${formatBytes(u.size)}`;
      case "waiting":
        return `${u.message ?? "Waiting"} · ${formatBytes(u.sent)} sent`;
      case "finishing":
        return u.hashed < u.size ? `Checking ${Math.floor((u.hashed / Math.max(1, u.size)) * 100)} %` : "Adding to the library";
      case "done":
        return uploadDoneText(u.result);
      case "failed":
        return u.message ?? "Failed";
      case "cancelled":
        return "Cancelled";
    }
  }
  const pct = (u: UploadView) => (u.phase === "done" ? 100 : Math.round((u.sent / Math.max(1, u.size)) * 100));
</script>

{#if list.length}
  <section class="tray" aria-label="Uploads">
    <div class="head">
      <button type="button" class="title" aria-expanded={open} onclick={() => (open = !open)}>
        <Upload size={14} strokeWidth={1.8} aria-hidden="true" />
        {going ? `Uploading ${going} ${going === 1 ? "file" : "files"} · ${Math.round(total * 100)} %` : `${finished} ${finished === 1 ? "upload" : "uploads"} finished`}
      </button>
      {#if finished}<button type="button" class="link" onclick={() => libraryPage.clearUploads()}>Clear</button>{/if}
    </div>
    {#if going}<div class="bar all" aria-hidden="true"><span style:width={`${total * 100}%`}></span></div>{/if}
    {#if open}
      <ul>
        {#each list as u (u.key)}
          <li class:failed={u.phase === "failed"} class:waiting={u.phase === "waiting"} class:done={u.phase === "done"}>
            <div class="row">
              <span class="name" title={u.name}>{u.replaces ? "↻ " : ""}{u.name}</span>
              {#if u.phase === "failed"}
                <button type="button" class="icon" aria-label={`Retry ${u.name}`} title="Try again" onclick={() => libraryPage.retryUpload(u.key)}><RotateCcw size={13} strokeWidth={2} /></button>
              {/if}
              {#if u.phase !== "done" && u.phase !== "cancelled"}
                <button type="button" class="icon" aria-label={`Cancel ${u.name}`} title="Cancel" onclick={() => libraryPage.cancelUpload(u.key)}><X size={13} strokeWidth={2} /></button>
              {/if}
            </div>
            <div class="bar" role="progressbar" aria-label={u.name} aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct(u)}><span style:width={`${pct(u)}%`}></span></div>
            <span class="state">{stateText(u)}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<style>
  .tray {
    flex: none;
    max-height: 40%;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--line);
    background: var(--surface-panel);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px 6px;
  }
  .title {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    font-size: 12.5px;
    font-weight: 600;
    text-align: left;
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-link);
    font-size: var(--fs-sm);
  }
  ul {
    margin: 0;
    padding: 0 12px 10px;
    list-style: none;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .name {
    flex: 1;
    min-width: 0;
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .icon {
    display: inline-flex;
    padding: 3px;
    border: 0;
    border-radius: var(--r-sm);
    background: none;
    color: var(--text-label);
  }
  .icon:hover {
    background: var(--surface-overlay);
    color: var(--text);
  }
  .bar {
    height: 4px;
    margin-top: 4px;
    border-radius: 2px;
    background: var(--surface-well);
    overflow: hidden;
  }
  .bar.all {
    margin: 0 12px 8px;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.2s;
  }
  .done .bar span {
    background: var(--ok);
  }
  .waiting .bar span {
    background: var(--live);
  }
  .failed .bar span {
    background: var(--danger);
  }
  .state {
    display: block;
    margin-top: 3px;
    font-size: var(--fs-xs);
    color: var(--text-meta);
  }
  .waiting .state {
    color: var(--live);
  }
  .failed .state {
    color: var(--danger-text);
  }
</style>
