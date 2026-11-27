<script lang="ts">
  import Button from "../ui/Button.svelte";
  import Eraser from "lucide-svelte/icons/eraser";
  import type { ClearGroup } from "$protocol";
  import type { Connection } from "../server";
  import { session } from "../state/session.svelte";
  import { shell } from "../state/shell.svelte";
  import { useLive } from "../live";
  import { cssColor } from "../model";
  import ScreenStats from "../pages/show/ScreenStats.svelte";

  /** `connection: false` leaves out the connection line (the connection pill shows its own). */
  let { conn, connection = true }: { conn: Connection; connection?: boolean } = $props();
  // svelte-ignore state_referenced_locally
  const status = conn.status;
  const live = useLive();
  const diagnostics = live.diagnostics;
  const activity = live.activity;

  let online = $derived($status.phase === "online");
  let host = $derived(session.baseUrl.replace(/^https?:\/\//, ""));

  function groupActive(g: ClearGroup): boolean {
    const a = $activity;
    return (g.targets ?? []).some((t) => {
      switch (t.layer) {
        case "all":
          return Object.values(a).some(Boolean);
        case "slide":
          return t.destination === "announcements" ? a.announcements : a.slide;
        case "media":
        case "videoInput":
        case "props":
        case "messages":
        case "audio":
        case "announcements":
          return a[t.layer];
        default:
          return false;
      }
    });
  }
</script>

<div class="status">
  {#if connection}
  <section>
    <div class="row">
      <span class="dot" class:bad={!online}></span>
      <strong>{online ? "Connected" : $status.phase === "incompatible" ? "Incompatible server" : "Offline"}</strong>
      <span class="dim">{session.server?.name ?? "Server"} · {host}</span>
    </div>
    {#if session.server}
      <div class="dim small">Server {session.server.version} · protocol {session.server.protocolVersion} · {session.server.auth === "local" ? "this machine" : "paired"}</div>
    {/if}
    {#if !online && $status.lastError}<div class="err small">{$status.lastError}</div>{/if}
  </section>
  {/if}

  <section>
    <div class="head"><span class="section-title">Screens</span></div>
    <ScreenStats />
  </section>

  {#if session.clearGroups.length}
    <section>
      <div class="head"><span class="section-title">Clear groups</span></div>
      <div class="groups">
        {#each session.clearGroups as g (g.id)}
          <button
            class="group"
            class:lit={groupActive(g)}
            style:--tint={cssColor(g.tint) ?? "#6a6a6a"}
            onclick={() => session.run({ type: "clearGroup", id: g.id })}
            title={`Run clear group ${g.name ?? ""}`}
          >
            <Eraser size={13} />{g.name ?? "Clear group"}
          </button>
        {/each}
      </div>
    </section>
  {/if}

  <section class="foot">
    <Button
     
      onclick={() => {
        shell.popover = null;
        shell.showDiagnostics = true;
      }}
    >
      Diagnostics{#if $diagnostics && $diagnostics.missingMedia + $diagnostics.fontMisses + $diagnostics.loadIssues > 0}
        <span class="warn">· {$diagnostics.missingMedia} missing media, {$diagnostics.fontMisses} fonts</span>
      {/if}
    </Button>
    <Button onclick={() => session.disconnect()}>Switch server</Button>
  </section>
</div>

<style>
  .status {
    padding: 4px 0;
  }
  section {
    padding: 8px 12px;
    border-bottom: 1px solid var(--line);
  }
  section:last-child {
    border-bottom: 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--ok);
  }
  .dot.bad {
    background: var(--danger);
  }
  .dim {
    color: var(--text-label);
  }
  .small {
    font-size: 12px;
    margin-top: 3px;
  }
  .err {
    color: var(--danger-text);
  }
  .head {
    margin-bottom: 6px;
  }
  .groups {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 4px 9px;
    border-radius: 3px;
    border: 1px solid var(--tint);
    background: var(--surface-panel);
    color: var(--text-2);
    font-size: 12px;
  }
  .group.lit {
    background: var(--onair-dim);
    color: var(--text-strong);
  }
  .foot {
    display: flex;
    gap: 8px;
  }
  .warn {
    color: var(--warn);
  }
</style>
