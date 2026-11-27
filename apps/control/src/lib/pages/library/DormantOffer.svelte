<script lang="ts">
  // The Library's offer to upload this show box's own, dormant collection to the site's Codex
  // (G15), its progress, and "Show it here" once it is in.
  import Upload from "lucide-svelte/icons/upload";
  import { StatusDot } from "../../ui";
  import { dormantState } from "../../state/dormant.svelte";
  import { siteState } from "../../state/site.svelte";
  import { dormantCard, progressText } from "./dormant";

  let { readOnly = false }: { readOnly?: boolean } = $props();

  let dormantId = $derived(siteState.codex?.role === "client" ? (siteState.codex.dormantCollection ?? null) : null);
  // Read again when the box's Codex status changes (start and end of the upload, a reconnect).
  $effect(() => {
    void siteState.codex;
    if (dormantId) void dormantState.load();
    else dormantState.reset();
  });

  let u = $derived(dormantState.upload);
  let card = $derived(dormantCard(u));
  let shownHere = $derived(!!u?.collection && siteState.codex?.collection === u.collection);
</script>

{#if u && card}
  <section class="card dormant" aria-label="This box's own collection" data-testid="dormant-offer">
    <div class="row">
      <StatusDot status={card.tone === "error" ? "error" : card.tone === "ok" ? "ok" : card.tone === "busy" ? "warn" : "off"} />
      <span class="ct">{card.title}</span>
    </div>
    <p class="detail">{card.detail}</p>
    {#if u.state === "copying"}
      <div class="bar" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(progressText(u).fraction * 100)}>
        <span style:width={`${Math.round(progressText(u).fraction * 100)}%`}></span>
      </div>
      <button type="button" class="link" onclick={() => dormantState.cancel()}>Cancel</button>
    {:else if u.state === "done"}
      {#if shownHere}
        <p class="detail">This box shows it now.</p>
      {:else}
        <button type="button" class="act" disabled={readOnly} onclick={() => dormantState.showHere()}>Show it here</button>
      {/if}
    {:else}
      <button type="button" class="act" disabled={readOnly || dormantState.busy} onclick={() => dormantState.start()}>
        <Upload size={13} strokeWidth={2} aria-hidden="true" />{u.state === "idle" ? "Upload it" : "Upload again"}
      </button>
      {#if u.state === "idle"}<p class="hint">Admins only. It becomes a new collection; this box's copy stays as it is.</p>{/if}
    {/if}
  </section>
{/if}

<style>
  .dormant {
    margin-top: 8px;
    display: flex;
    flex-direction: column;
    padding: 8px;
    border-radius: var(--r-card);
    background: var(--surface-raised);
    border: 1px solid var(--line-strong);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ct {
    font-weight: 600;
    font-size: 13px;
  }
  .detail,
  .hint {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.4;
  }
  .hint {
    font-size: 11px;
  }
  .bar {
    margin-top: 8px;
    height: 6px;
    border-radius: 3px;
    background: var(--line);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.3s;
  }
  .act {
    align-self: flex-start;
    margin-top: 8px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 5px 10px;
    border-radius: 6px;
    border: 1px solid var(--line-strong);
    background: var(--surface-2);
    color: var(--text);
    font-size: 12px;
    cursor: pointer;
  }
  .act:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .link {
    align-self: flex-start;
    margin-top: 6px;
    background: none;
    border: 0;
    padding: 0;
    color: var(--accent);
    font-size: 12px;
    cursor: pointer;
  }
</style>
