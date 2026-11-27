<script lang="ts">
  import Button from "../ui/Button.svelte";
  import { onMount } from "svelte";
  import Server from "lucide-svelte/icons/server";
  import RefreshCw from "lucide-svelte/icons/refresh-cw";
  import KeyRound from "lucide-svelte/icons/key-round";
  import Loader from "lucide-svelte/icons/loader-circle";
  import Logo from "./Logo.svelte";
  import { APP_FULL_NAME, DEFAULT_PORT, SERVER_NAME } from "../config";
  import { REVOKED_TEXT, session } from "../state/session.svelte";
  import { discoverServers, isTauri, type DiscoveredServer } from "../tauri";
  import { COMPUTER_NAME_MAX } from "../computerName";

  let { booted }: { booted: boolean } = $props();

  let address = $state(session.defaultAddress());
  let code = $state("");
  let computer = $state(session.computerName());
  let found = $state<DiscoveredServer[]>([]);
  let scanning = $state(false);
  const canDiscover = isTauri();

  async function scan() {
    if (!canDiscover) return;
    scanning = true;
    try {
      found = await discoverServers();
    } catch {
      found = [];
    } finally {
      scanning = false;
    }
  }

  onMount(() => void scan());

  function submitAddress(e: SubmitEvent) {
    e.preventDefault();
    void session.connect(address);
  }

  function submitCode(e: SubmitEvent) {
    e.preventDefault();
    if (code.replace(/\D/g, "").length === 6) void session.pair(code.replace(/\D/g, ""));
  }

  function onCodeInput(e: Event) {
    const v = (e.currentTarget as HTMLInputElement).value.replace(/\D/g, "").slice(0, 6);
    code = v;
    if (v.length === 6) void session.pair(v);
  }
</script>

<main class="connect">
  <div class="card" aria-busy={session.connecting || !booted}>
    <header>
      <Logo size={40} />
      <div>
        <h1>{APP_FULL_NAME}</h1>
        <p class="sub">Connect to a {SERVER_NAME} server</p>
      </div>
    </header>

    {#if !booted}
      <p class="status"><Loader size={16} class="spin" /> Looking for the server…</p>
    {:else if session.pairing?.waiting}
      <div class="waiting" role="status">
        <h2><KeyRound size={18} /> Pair with “{session.pairing.server.name}”</h2>
        {#if session.pairing.codeWrong}
          <p class="error" role="alert">
            That code is not right, or it has expired. Check the code shown on the server and enter it again.
          </p>
        {/if}
        <p class="status"><Loader size={16} class="spin" /> Waiting for approval…</p>
        <p class="hint">
          A site administrator approves this computer on the Site page of {APP_FULL_NAME}. This page goes on by
          itself once they do.
        </p>
        <div class="row">
          <Button type="button" onclick={() => session.cancelPairing()}>Back</Button>
          {#if session.pairing.codeWrong}
            <Button type="button" variant="primary" onclick={() => session.retryCode()}>Enter the code again</Button>
          {/if}
        </div>
      </div>
    {:else if session.pairing}
      <form onsubmit={submitCode}>
        <h2><KeyRound size={18} /> Pair with “{session.pairing.server.name}”</h2>
        {#if session.revoked}
          <p class="removed" role="alert">{REVOKED_TEXT}</p>
          <p class="hint">
            An administrator removed this computer's key. Enter the 6-digit code shown on the server (in its log, or in
            the {APP_FULL_NAME} app on the server machine), or ask for approval below.
          </p>
        {:else}
          <p class="hint">
            This computer isn't paired yet. Enter the 6-digit code shown on the server (in its log, or in the
            {APP_FULL_NAME} app on the server machine).
          </p>
        {/if}
        {#if session.pairing.returnTo}
          <p class="hint" data-testid="pair-return">
            This gives this computer the site's key for uploads, collections, backups and exports. {APP_FULL_NAME} then
            goes back to {session.pairing.returnTo.replace(/^https?:\/\//, "")}.
          </p>
        {/if}
        <label class="name-field">
          <span>This computer's name</span>
          <input
            type="text"
            value={computer}
            maxlength={COMPUTER_NAME_MAX}
            spellcheck="false"
            title="The administrator sees this name when approving, and in the site's list of keys"
            oninput={(e) => (computer = e.currentTarget.value)}
            onchange={() => {
              session.setComputerName(computer);
              computer = session.computerName();
            }}
            disabled={session.connecting}
          />
        </label>
        <label class="sr-only" for="pair-code">Pairing code</label>
        <input
          id="pair-code"
          class="code"
          inputmode="numeric"
          autocomplete="one-time-code"
          placeholder="000000"
          maxlength="6"
          value={code}
          oninput={onCodeInput}
          disabled={session.connecting}
        />
        {#if session.connectError}<p class="error" role="alert">{session.connectError}</p>{/if}
        <button type="button" class="link" onclick={() => void session.pair(null)} disabled={session.connecting}>
          No code? Ask a site administrator to approve this computer
        </button>
        <div class="row">
          <Button type="button" onclick={() => session.cancelPairing()}>Back</Button>
          <Button type="submit" variant="primary" disabled={code.length !== 6 || session.connecting}>
            {#if session.connecting}<Loader size={14} class="spin" />{/if} Pair
          </Button>
        </div>
      </form>
    {:else}
      {#if canDiscover}
        <section aria-label="Servers on this network">
          <div class="list-head">
            <span class="section-title">On this network</span>
            <button class="icon" onclick={scan} aria-label="Search again" disabled={scanning}>
              <RefreshCw size={14} class={scanning ? "spin" : ""} />
            </button>
          </div>
          {#if found.length}
            <ul class="servers">
              {#each found as s (s.id)}
                <li>
                  <button
                    class="server"
                    disabled={!s.baseUrl || session.connecting}
                    onclick={() => s.baseUrl && session.connect(s.baseUrl)}
                  >
                    <Server size={18} />
                    <span class="name">{s.name}</span>
                    <span class="meta">{s.baseUrl ?? "no address"} · v{s.version}</span>
                  </button>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="hint">{scanning ? "Searching…" : "No servers found. Type an address below."}</p>
          {/if}
        </section>
      {/if}

      <form onsubmit={submitAddress}>
        <label for="addr" class="section-title">Server address</label>
        <div class="row">
          <input id="addr" type="text" bind:value={address} placeholder={`booth.local:${DEFAULT_PORT}`} spellcheck="false" />
          <Button type="submit" variant="primary" disabled={session.connecting}>
            {#if session.connecting}<Loader size={14} class="spin" />{/if} Connect
          </Button>
        </div>
        {#if session.connectError}
          <p class="error" role="alert">{session.connectError}</p>
        {/if}
        <p class="hint">A host name or IP address. The port defaults to {DEFAULT_PORT}.</p>
      </form>
    {/if}
  </div>
</main>

<style>
  .removed {
    margin: 0 0 8px;
    padding: 8px 10px;
    border: 1px solid var(--danger);
    border-radius: var(--r);
    background: var(--danger-soft);
    color: var(--text);
    font-weight: 600;
  }
  .connect {
    height: 100%;
    display: grid;
    place-items: center;
    background: radial-gradient(1200px 600px at 50% -10%, var(--surface-panel) 0%, var(--bg-0) 60%);
  }
  .card {
    width: min(480px, calc(100vw - 32px));
    background: var(--bg-1);
    border: 1px solid var(--line);
    border-radius: 12px;
    padding: 28px;
    box-shadow: var(--shadow-dialog);
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  header {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  h1 {
    margin: 0;
    font-size: 22px;
  }
  h2 {
    margin: 0 0 6px;
    font-size: 15px;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .sub {
    margin: 2px 0 0;
    color: var(--text-dim);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .row {
    display: flex;
    gap: 8px;
  }
  .row input {
    flex: 1;
  }
  input {
    min-height: 36px !important;
    font-size: 14px;
  }
  .name-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .code {
    font-family: var(--mono);
    font-size: 28px !important;
    letter-spacing: 0.4em;
    text-align: center;
    padding: 10px !important;
  }
  form .row:last-child {
    justify-content: flex-end;
  }
  .hint {
    margin: 0;
    color: var(--text-dim);
    font-size: 12px;
  }
  .link {
    align-self: flex-start;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-ring);
    font-size: 12px;
    text-decoration: underline;
    cursor: pointer;
  }
  .link:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .waiting {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .waiting .row {
    justify-content: flex-end;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-dim);
  }
  .error {
    margin: 0;
    color: var(--danger-text);
    background: rgba(229, 72, 77, 0.12);
    border: 1px solid rgba(229, 72, 77, 0.4);
    padding: 8px 10px;
    border-radius: var(--radius);
  }
  .list-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 6px;
  }
  .icon {
    background: none;
    border: 0;
    color: var(--text-dim);
    padding: 4px;
    border-radius: 4px;
  }
  .servers {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .server {
    width: 100%;
    display: grid;
    grid-template-columns: auto 1fr;
    grid-template-rows: auto auto;
    column-gap: 10px;
    text-align: left;
    align-items: center;
    padding: 10px 12px;
    border-radius: var(--radius);
    border: 1px solid var(--line);
    background: var(--bg-2);
  }
  .server:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .server :global(svg) {
    grid-row: span 2;
  }
  .name {
    font-weight: 600;
  }
  .meta {
    color: var(--text-dim);
    font-size: 12px;
  }
  :global(.spin) {
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
