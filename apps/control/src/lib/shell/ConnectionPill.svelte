<script lang="ts">
  // The header's connection pill: a dot and "Connected" / "Reconnecting…". Its popover shows
  // the server, the Codex role and collection, Reconnect, and the server's status (the old
  // toolbar's Status popover).
  import Popover from "../ui/Popover.svelte";
  import Button from "../ui/Button.svelte";
  import type { Connection } from "../server";
  import { session } from "../state/session.svelte";
  import { shell } from "../state/shell.svelte";
  import { siteState } from "../state/site.svelte";
  import StatusPanel from "./StatusPanel.svelte";

  let { conn }: { conn: Connection } = $props();
  // svelte-ignore state_referenced_locally
  const status = conn.status;

  let online = $derived($status.phase === "online");
  let label = $derived(online ? "Connected" : $status.phase === "incompatible" ? "Incompatible" : "Reconnecting…");
  let open = $derived(shell.popover === "connection");
  let host = $derived(session.baseUrl.replace(/^https?:\/\//, ""));

  const ROLES = { host: "holds the site's Codex", client: "show box (mirrors the site's Codex)", dormant: "joining the site's Codex", unknown: "Codex role unknown" };
  let codexLine = $derived.by(() => {
    const c = siteState.codex;
    if (!c) return siteState.has("codex") ? "Codex: loading…" : "No Codex on this server";
    const sync = c.sync && !c.sync.reachable ? " · the site's server can't be reached" : "";
    return `Collection “${c.collectionName}” · this box ${ROLES[c.role] ?? c.role}${sync}`;
  });

  function reconnect() {
    // Drops the socket; the connection comes straight back (and resyncs everything).
    if (online) conn.socket.kick("Reconnect asked by the operator");
    else conn.socket.retryNow();
  }
</script>

<div class="conn">
  <button
    class="pill"
    class:down={!online}
    aria-expanded={open}
    aria-label={`${session.server?.name ?? "Vesper server"}: ${label}`}
    title="Connection to the Vesper server"
    onclick={() => shell.togglePopover("connection")}
  >
    <span class="dot" aria-hidden="true"></span>{label}
  </button>
  {#if open}
    <Popover align="right" width={440} label="Connection" onclose={() => (shell.popover = null)}>
      <div class="head">
        <strong>{session.server?.name ?? "Vesper server"}</strong>
        <span class="dim">{host}{session.server ? ` · Vesper ${session.server.version} · ${session.server.auth === "local" ? "this machine" : "paired"}` : ""}</span>
        <span class="dim">{online ? "Connected · live state in sync" : `Trying to reconnect (attempt ${Math.max(1, $status.attempt)})`}</span>
        <span class="dim">{codexLine}</span>
        {#if !online && $status.lastError}<span class="err">{$status.lastError}</span>{/if}
        <div class="btns">
          <Button size="sm" variant="secondary" onclick={reconnect}>Reconnect</Button>
        </div>
      </div>
      <StatusPanel {conn} connection={false} />
    </Popover>
  {/if}
</div>

<style>
  .conn {
    position: relative;
  }
  .pill {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 10px;
    border-radius: var(--r-pill);
    border: 1px solid var(--line);
    background: var(--surface-sunk);
    color: var(--text-2);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .pill.down {
    color: var(--danger-text);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--ok);
  }
  .down .dot {
    background: var(--danger);
  }
  .head {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px 12px 10px;
    font-size: 12.5px;
    border-bottom: 1px solid var(--line-strong);
  }
  .head strong {
    font-size: var(--fs);
  }
  .dim {
    color: var(--text-label);
  }
  .err {
    color: var(--danger-text);
  }
  .btns {
    display: flex;
    gap: 6px;
    margin-top: 2px;
  }
</style>
