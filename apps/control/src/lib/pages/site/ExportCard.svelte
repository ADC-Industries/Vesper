<script lang="ts">
  // Protect the Codex ▸ Download an export now: the passphrase twice (8 characters at least),
  // the note that media files aren't in it, "Include the site's identity" (off by default, only
  // on the site's server), then the download.
  import { Button } from "../../ui";
  import { sitePage } from "../../state/site.svelte";
  import { session } from "../../state/session.svelte";
  import { MIN_PASSPHRASE, passphraseProblem } from "./logic";

  let { siteOption }: { siteOption: boolean } = $props();

  let pass = $state("");
  let again = $state("");
  let includeSite = $state(false);
  let busy = $state(false);
  let done = $state<string | null>(null);
  let problem = $derived(pass || again ? passphraseProblem(pass, again) : null);

  async function download() {
    if (passphraseProblem(pass, again)) return;
    busy = true;
    done = null;
    const file = await sitePage.admin("The export", () =>
      sitePage.codexBox((c) => c.exportNow(includeSite && siteOption ? { passphrase: pass, includeSite: true } : { passphrase: pass }, sitePage.auth())),
    );
    busy = false;
    if (!file) return;
    const url = URL.createObjectURL(file.blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = file.fileName;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 10_000);
    pass = again = "";
    done = file.fileName;
    session.conn?.pushNotice("info", `Saved ${file.fileName}. Keep the passphrase somewhere safe: without it the export can't be opened.`, "site:export");
  }
</script>

<section class="st-card" aria-labelledby="export-h" data-testid="export-card">
  <h2 id="export-h">Download an export now</h2>
  <span class="st-meta">Every collection and the settings backups, encrypted with a passphrase you choose.</span>
  <label class="f">Passphrase<input type="password" autocomplete="new-password" bind:value={pass} placeholder={`At least ${MIN_PASSPHRASE} characters`} disabled={busy} /></label>
  <label class="f">Again<input type="password" autocomplete="new-password" bind:value={again} disabled={busy} /></label>
  {#if problem}<span class="err">{problem}</span>{/if}
  <p class="warn">Media files aren't included: back up the media folder separately.</p>
  {#if siteOption}
    <label class="tick"><input type="checkbox" bind:checked={includeSite} disabled={busy} /> Include the site's identity</label>
    <span class="st-meta">Only for moving the whole site to a new server: whoever has the file and its passphrase can stand in for this site.</span>
  {/if}
  <div class="st-row">
    <Button variant="primary" size="sm" onclick={download} disabled={busy || !!passphraseProblem(pass, again)}>{busy ? "Making the export…" : "Download export"}</Button>
    {#if busy}<span class="st-meta" role="status">This can take a minute for a large Codex.</span>{/if}
    {#if done && !busy}<span class="ok" role="status">Saved {done}</span>{/if}
  </div>
</section>

<style>
  .f {
    display: grid;
    grid-template-columns: 96px 1fr;
    align-items: center;
    gap: 8px;
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  .err {
    color: var(--danger-text);
    font-size: var(--fs-sm);
  }
  .warn {
    margin: 0;
    color: var(--live);
    font-size: var(--fs-sm);
  }
  .tick {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .tick input {
    accent-color: var(--accent);
  }
  .ok {
    color: var(--ok-text);
    font-size: var(--fs-sm);
  }
</style>
