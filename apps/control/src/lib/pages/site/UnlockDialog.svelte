<script lang="ts">
  // The site's unlock (access `pin`: the admin PIN) or sign-in (access `users`), asked when an
  // admin action was refused with 403; the action runs again once it worked.
  import { Button, Dialog } from "../../ui";
  import { sitePage, type SiteAsk } from "../../state/site.svelte";

  let { ask }: { ask: SiteAsk } = $props();

  let pin = $state("");
  let user = $state("");
  let password = $state("");

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (ask.kind === "unlock") sitePage.reply({ pin });
    else sitePage.reply({ user, password });
    pin = password = "";
  }
  function focus(node: HTMLInputElement) {
    queueMicrotask(() => node.focus());
  }
</script>

<Dialog title={ask.kind === "unlock" ? "Unlock the site" : "Sign in to the site"} onclose={() => sitePage.reply(null)} width={380} escape>
  <form id="site-unlock" class="form" onsubmit={submit}>
    {#if ask.kind === "unlock"}
      <p>This is an admin action. Type the site's admin PIN; Oriel stays unlocked for 30 minutes after the last admin action.</p>
      <label>PIN<input use:focus type="password" inputmode="numeric" autocomplete="off" maxlength="8" bind:value={pin} /></label>
    {:else}
      <p>This is an admin action. Sign in as a site user with the admin role.</p>
      <label>User<input use:focus type="text" autocomplete="username" bind:value={user} /></label>
      <label>Password<input type="password" autocomplete="current-password" bind:value={password} /></label>
    {/if}
    {#if ask.error}<span class="err" role="alert">{ask.error}</span>{/if}
  </form>
  {#snippet footer()}
    <Button onclick={() => sitePage.reply(null)}>Cancel</Button>
    <Button variant="primary" type="submit" form="site-unlock" disabled={ask.kind === "unlock" ? pin.length < 4 : !user || !password}>{ask.kind === "unlock" ? "Unlock" : "Sign in"}</Button>
  {/snippet}
</Dialog>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  p {
    margin: 0;
    color: var(--text-2);
    line-height: 1.5;
  }
  label {
    display: grid;
    grid-template-columns: 80px 1fr;
    align-items: center;
    gap: 8px;
    color: var(--text-label);
  }
  .err {
    color: var(--danger-text);
  }
</style>
