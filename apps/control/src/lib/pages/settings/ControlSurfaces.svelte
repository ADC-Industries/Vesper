<script lang="ts">
  // Settings ▸ Network & IDN ▸ Control surfaces: Remote API compatibility (the endpoints older
  // Stream Deck / Companion button profiles call, on a port of their own; capability
  // `remoteCompat`) and a pointer to the Companion module.
  import type { RemoteCompatStatus, UpdateRemoteCompat } from "$protocol";
  import { Button, StatusDot, Switch } from "../../ui";
  import { session } from "../../state/session.svelte";
  import { parsePort, remoteCompatLine } from "./controlSurfaces";

  let status = $state.raw<RemoteCompatStatus | null>(null);
  let portText = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);

  let line = $derived(remoteCompatLine(status));
  let port = $derived(parsePort(portText));
  let portDirty = $derived(!!status && port !== null && port !== status.port);

  async function load() {
    const conn = session.conn;
    if (!conn) return;
    try {
      status = await conn.rest.remoteCompat();
      portText = String(status.port);
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }
  void load();

  async function save(change: UpdateRemoteCompat) {
    const conn = session.conn;
    if (!conn) return;
    busy = true;
    error = null;
    try {
      status = await conn.rest.saveRemoteCompat(change);
      portText = String(status.port);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="cs">
  <div class="row">
    <span>Remote API compatibility</span>
    <Switch label="Remote API compatibility" checked={status?.enabled ?? false} disabled={!status || busy} onchange={(on) => save({ enabled: on })} data-testid="remote-compat-switch" />
    <StatusDot status={line.status} label={line.text} />
  </div>
  <div class="row">
    <label for="remote-compat-port">Port</label>
    <input
      id="remote-compat-port"
      class="port num"
      inputmode="numeric"
      bind:value={portText}
      disabled={!status || busy}
      aria-invalid={portText !== "" && port === null}
      onkeydown={(e) => {
        if (e.key === "Enter" && portDirty && port !== null) void save({ port });
      }}
    />
    <Button size="sm" disabled={!portDirty || busy} onclick={() => port !== null && save({ port })}>Use this port</Button>
    {#if portText !== "" && port === null}<span class="err">1 to 65535</span>{/if}
  </div>
  {#if error}<p class="err" role="alert">{error}</p>{/if}
  <p class="meta">
    Answers the common endpoints older Stream Deck and Companion button profiles call (next and previous slide, slides by number, clear, timers,
    messages, looks, props, macros, stage message), so they keep working with Vesper: point them at this server's address and the port above.
  </p>
  <p class="meta">
    Requests from this server's own machine are trusted. Profiles on other computers need this site's key, sent as <code>Authorization: Bearer</code>
    or <code>?token=</code>: pair them like a tablet. For new setups, Vesper's Companion module adds feedbacks and variables.
  </p>
</div>

<style>
  .cs {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 12px;
  }
  .row > :first-child {
    min-width: 164px;
    color: var(--text-label);
  }
  .port {
    width: 80px;
    min-height: 28px;
  }
  .num {
    font-variant-numeric: tabular-nums;
  }
  .meta {
    margin: 0;
    color: var(--text-meta);
    line-height: 1.45;
  }
  .err {
    margin: 0;
    color: var(--danger-text);
  }
  code {
    font-size: var(--fs-sm);
  }
</style>
