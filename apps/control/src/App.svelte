<script lang="ts">
  import { onMount } from "svelte";
  import ConnectScreen from "./lib/components/ConnectScreen.svelte";
  import MainView from "./lib/shell/MainView.svelte";
  import { session } from "./lib/state/session.svelte";

  let booted = $state(false);

  onMount(() => {
    void session.autoConnect().finally(() => (booted = true));
  });
</script>

{#if session.view === "main" && session.conn}
  {#key session.conn}
    <MainView conn={session.conn} />
  {/key}
{:else}
  <ConnectScreen {booted} />
{/if}
