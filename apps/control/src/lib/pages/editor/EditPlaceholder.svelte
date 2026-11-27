<script lang="ts">
  import Pencil from "lucide-svelte/icons/pencil";
  import PageToolbar from "../../ui/PageToolbar.svelte";
  import { session } from "../../state/session.svelte";
  import { shell } from "../../state/shell.svelte";
</script>

<section class="edit" aria-label="Edit view">
  <PageToolbar title="Edit" ondone={() => shell.close()} />
  <div class="msg">
    <Pencil size={40} strokeWidth={1.3} />
    {#if session.openLoading}
      <p>Opening…</p>
    {:else}
      <p><strong>Select a presentation to edit.</strong></p>
      <p>Pick one in the library or a playlist; its slides open here.</p>
    {/if}
  </div>
</section>

<style>
  .edit {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--work);
  }
  .msg {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    color: var(--text-dim);
  }
  p {
    margin: 2px 0;
  }
</style>
