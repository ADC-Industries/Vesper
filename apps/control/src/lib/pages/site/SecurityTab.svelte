<script lang="ts">
  // Site ▸ Security (not on the canvas, plan question 11): who may do the site's admin actions
  // (open / PIN / users) with unlock and lock, the site's users, the paired computers (the
  // site's API keys) with Revoke, this server's membership of another site (join, leave) and
  // the audit list.
  import type { SiteAccess, SiteApiKey, SiteRole } from "$protocol";
  import { Button } from "../../ui";
  import { sitePage } from "../../state/site.svelte";
  import { shell } from "../../state/shell.svelte";
  import { ACCESS, ago, formatWhen, inOtherSite, membershipText, spacedCode } from "./logic";

  let you = $derived(sitePage.status?.you);
  let pinFor = $state<SiteAccess | null>(null);
  let pin = $state("");
  let newUser = $state({ name: "", role: "operator" as SiteRole, password: "" });
  let joinAddr = $state("");
  let joinCode = $state("");
  let m = $derived(sitePage.membership);

  const refresh = () => Promise.all([sitePage.loadStatus(), sitePage.loadSecurity()]);

  async function setAccess(a: SiteAccess) {
    if (!sitePage.status || a === you?.access) return;
    if (a === "pin" && !pinFor) {
      pinFor = "pin";
      pin = "";
      return;
    }
    const rev = sitePage.status.revision;
    const body = a === "pin" ? { access: a, pin } : { access: a };
    pinFor = null;
    pin = "";
    await sitePage.act("Changing who can do admin actions", (s) => s.saveSettings(body, rev), refresh);
  }
  // After damaged site settings the access reads "users" (the radio can't be chosen again):
  // keeping it sets it again, which ends the lockdown.
  async function keepAccess() {
    const st = sitePage.status;
    if (!st?.settingsDamaged) return;
    await sitePage.act("Setting the access again", (s) => s.saveSettings({ access: st.you.access }, st.revision), refresh);
  }
  async function revoke(k: SiteApiKey) {
    if (!(await shell.confirm("Revoke this computer?", `“${k.name}” stops working on every server of the site at once. It can pair again later.`, "Revoke"))) return;
    await sitePage.act(`Revoking ${k.name}`, (s) => s.revokeKey(k.id), sitePage.loadSecurity);
  }
  async function addUser() {
    const u = { ...newUser };
    if (!u.name.trim() || u.password.length < 8) return;
    if (await sitePage.act(`Adding ${u.name}`, (s) => s.putUser(u.name.trim(), { role: u.role, password: u.password }), sitePage.loadSecurity))
      newUser = { name: "", role: "operator", password: "" };
  }
  async function removeUser(name: string) {
    if (!(await shell.confirm("Remove this user?", `${name} can't sign in to the site any more.`, "Remove"))) return;
    await sitePage.act(`Removing ${name}`, (s) => s.removeUser(name), sitePage.loadSecurity);
  }
  async function join() {
    const server = joinAddr.trim();
    const code = joinCode.replace(/\s+/g, "");
    if (!server && !code) return;
    await sitePage.act("Joining the site", (s) => s.join({ ...(server ? { server } : {}), ...(code ? { code } : {}) }), sitePage.loadStatus);
    joinCode = "";
  }
  async function leave() {
    if (!(await shell.confirm("Leave that site?", "This server runs its own site again; computers paired through that site pair again here.", "Leave"))) return;
    await sitePage.act("Leaving the site", (s) => s.leave(), sitePage.loadStatus);
  }
  let keys = $derived([...sitePage.apiKeys].sort((a, b) => Number(a.revoked) - Number(b.revoked)));
</script>

<div class="st-head"><h1>Security</h1></div>

<section class="st-card" aria-labelledby="access-h">
  <h2 id="access-h">Who can do admin actions</h2>
  <span class="st-meta">Approving and removing devices, site codes, restores, exports and these settings. Running the show never needs a PIN or a sign-in.</span>
  <div role="radiogroup" aria-labelledby="access-h" class="modes">
    {#each Object.entries(ACCESS) as [value, a] (value)}
      <label class="mode">
        <input type="radio" name="site-access" checked={(pinFor ?? you?.access) === value} onchange={() => setAccess(value as SiteAccess)} disabled={!you} />
        <span class="st-two"><span class="st-strong">{a.name}</span><span class="st-meta">{a.desc}</span></span>
      </label>
    {/each}
  </div>
  {#if pinFor === "pin"}
    <div class="st-row">
      <label class="st-row">New PIN<input type="password" inputmode="numeric" maxlength="8" bind:value={pin} placeholder="4–8 digits" /></label>
      <Button size="sm" onclick={() => (pinFor = null)}>Cancel</Button>
      <Button size="sm" variant="primary" disabled={!/^\d{4,8}$/.test(pin)} onclick={() => setAccess("pin")}>Use this PIN</Button>
    </div>
  {/if}
  {#if sitePage.status?.settingsDamaged && you}
    <div class="st-row" data-testid="site-keep-access">
      <span class="st-meta">The site settings were damaged, so the access was set to {ACCESS[you.access].name} and paired computers can only view. Choose an access, or keep this one.</span>
      <Button size="sm" variant={you.role === "admin" ? "primary" : undefined} onclick={keepAccess}>Keep {ACCESS[you.access].name}</Button>
    </div>
  {/if}
  {#if you && you.access !== "open"}
    <div class="st-row you">
      {#if you.unlocked || you.signedIn}
        <span class="ok">This computer acts as {you.user ?? "admin"}{you.unlockSecondsLeft ? ` · locks in ${Math.ceil(you.unlockSecondsLeft / 60)} min` : ""}</span>
        <Button size="sm" onclick={() => sitePage.lock()}>{you.access === "users" ? "Sign out" : "Lock"}</Button>
      {:else}
        <span class="st-meta">This computer is locked: it can read the site but not change it.</span>
        <Button size="sm" variant="primary" onclick={() => sitePage.unlockFor()}>{you.access === "users" ? "Sign in…" : "Unlock…"}</Button>
      {/if}
    </div>
  {/if}
</section>

<h2 class="st-section">Paired computers</h2>
{#if sitePage.problems.apiKeys}<p class="st-problem">{sitePage.problems.apiKeys}</p>{/if}
<div class="st-table-wrap">
  <table class="st" data-testid="api-keys">
    <thead><tr><th>Computer</th><th>Paired</th><th>Last used</th><th>Expires</th><th aria-label="Actions"></th></tr></thead>
    <tbody>
      {#each keys as k (k.id)}
        <tr class:revoked={k.revoked}>
          <td><span class="st-two"><span class="st-strong">{k.name}</span><span class="st-meta">by {k.by}</span></span></td>
          <td>{formatWhen(k.createdAt)}</td>
          <td>{k.lastUsed ? `${ago(k.lastUsed)}${k.lastMachine ? ` on ${k.lastMachine}` : ""}` : "Never"}</td>
          <td>{k.revoked ? `Revoked ${formatWhen(k.revokedAt)}` : formatWhen(k.expiresAt)}</td>
          <td class="actions">{#if !k.revoked}<Button size="sm" onclick={() => revoke(k)}>Revoke</Button>{/if}</td>
        </tr>
      {:else}
        <tr><td colspan="5" class="st-empty">No paired computers (this server isn't the site's server, or none paired yet).</td></tr>
      {/each}
    </tbody>
  </table>
</div>

<h2 class="st-section">Users</h2>
<div class="st-card">
  {#each sitePage.users as u (u.name)}
    <div class="st-row">
      <span class="st-strong user">{u.name}</span>
      <span class="st-meta role">{u.role}</span>
      <span class="st-grow"></span>
      <Button size="sm" onclick={() => removeUser(u.name)}>Remove</Button>
    </div>
  {:else}
    <span class="st-empty">No users yet. They are needed for the “Users” level.</span>
  {/each}
  <div class="st-row add">
    <input type="text" placeholder="User name" aria-label="New user's name" bind:value={newUser.name} />
    <select aria-label="Role" bind:value={newUser.role}>
      <option value="admin">Admin</option><option value="operator">Operator</option><option value="viewer">Viewer</option>
    </select>
    <input type="password" placeholder="Password (8+ characters)" aria-label="Password" autocomplete="new-password" bind:value={newUser.password} />
    <Button size="sm" onclick={addUser} disabled={!newUser.name.trim() || newUser.password.length < 8}>Add user</Button>
  </div>
</div>

<h2 class="st-section">This server</h2>
<div class="st-card" data-testid="membership">
  {#if m}
    <span>{membershipText(m)}{#if m.state === "waitingForApproval" && m.code}{" "}<b class="code">{spacedCode(m.code)}</b>{/if}</span>
    {#if inOtherSite(m)}
      <div class="st-row">
        {#if m.state === "codeNeeded" || m.state === "wrongCode" || m.state === "codeExpired"}
          <input type="text" inputmode="numeric" placeholder="Join code" aria-label="Join code" bind:value={joinCode} />
          <Button size="sm" variant="primary" onclick={join}>Send code</Button>
        {/if}
        <Button size="sm" onclick={leave}>{m.state === "enrolled" ? "Leave that site" : "Stop joining"}</Button>
      </div>
    {:else}
      <span class="st-meta">To make this server part of another Vesper server's site, give that server's address and, if it asks for one, its site code. Its other machines must leave or be removed first.</span>
      <div class="st-row">
        <input type="text" placeholder="Site server address" aria-label="Site server address" bind:value={joinAddr} />
        <input type="text" inputmode="numeric" placeholder="Site code (optional)" aria-label="Site code" bind:value={joinCode} />
        <Button size="sm" onclick={join} disabled={!joinAddr.trim()}>Join that site</Button>
      </div>
    {/if}
  {:else}
    <span class="st-empty">Reading…</span>
  {/if}
</div>

<h2 class="st-section">Audit</h2>
{#if sitePage.problems.audit}<p class="st-problem">{sitePage.problems.audit}</p>{/if}
<div class="st-table-wrap">
  <table class="st" data-testid="audit">
    <thead><tr><th>When</th><th>Who</th><th>From</th><th>Action</th><th>Result</th></tr></thead>
    <tbody>
      {#each sitePage.audit.slice(0, 50) as a, i (i)}
        <tr>
          <td>{formatWhen(a.at)}</td>
          <td><span class="st-two"><span>{a.user}</span><span class="st-meta">{a.client}</span></span></td>
          <td class="st-meta">{a.peer}</td>
          <td class="st-mono">{a.action}</td>
          <td class:fail={!a.ok}>{a.ok ? "Done" : `Refused${a.error ? `: ${a.error}` : ""}`}</td>
        </tr>
      {:else}
        <tr><td colspan="5" class="st-empty">Nothing yet.</td></tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .modes {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .mode {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    cursor: pointer;
  }
  .mode input {
    margin-top: 3px;
    accent-color: var(--accent);
  }
  .ok {
    color: var(--ok-text);
  }
  tr.revoked td {
    color: var(--text-meta);
  }
  .user {
    min-width: 160px;
  }
  .role {
    text-transform: capitalize;
  }
  .add {
    padding-top: 6px;
    border-top: 1px solid var(--line);
  }
  .fail {
    color: var(--danger-text);
  }
  .code {
    letter-spacing: 0.1em;
  }
</style>
