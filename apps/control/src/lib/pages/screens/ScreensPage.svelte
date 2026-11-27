<script lang="ts">
  // The Screens & Looks page (plan 4.8): Screens | Looks in the toolbar, the live look's name,
  // and for Screens the staged changes' Revert / Apply (ConfigureScreens keeps the staging,
  // validation and conflict logic). The Screens tab stays mounted while Looks shows, so
  // switching tabs never loses unapplied changes.
  import PageFrame from "../PageFrame.svelte";
  import { Button, Segmented } from "../../ui";
  import { session } from "../../state/session.svelte";
  import { screensState } from "../../state/screens.svelte";
  import { useLive } from "../../live";
  import ConfigureScreens from "./ConfigureScreens.svelte";
  import LooksView from "./LooksView.svelte";

  const live = useLive();
  const activeLook = live.activeLook;

  let lookName = $derived($activeLook === null ? "Every layer" : (session.looks.find((l) => l.id === $activeLook)?.name ?? "Look"));
  let ed = $derived(screensState.editor);
  const TABS = [
    { value: "screens" as const, label: "Screens" },
    { value: "looks" as const, label: "Looks" },
  ];
</script>

<PageFrame page="screens">
  {#snippet controls()}
    <Segmented options={TABS} value={screensState.tab} label="Screens or Looks" onchange={(v) => (screensState.tab = v)} />
    <span class="live">Live look: <b>{lookName}</b></span>
    <span class="grow"></span>
    {#if screensState.tab === "screens" && ed}
      {#if ed.dirty}<span class="unsaved" role="status" data-testid="unsaved-indicator"><span class="dot"></span>Changes not applied</span>{/if}
      <Button size="sm" onclick={() => ed.revert()} disabled={!ed.dirty || ed.busy} title={ed.dirty ? "Throw away the changes on this screen" : "No changes to revert"} data-testid="revert-btn">Revert</Button>
      <Button size="sm" variant="primary" onclick={() => void ed.apply()} disabled={!ed.dirty || ed.busy} title={ed.dirty ? "Save and apply live" : "No changes to apply"} data-testid="apply-btn">
        {ed.busy ? "Applying…" : "Apply"}
      </Button>
    {/if}
  {/snippet}
  <div class="tab" class:hidden={screensState.tab !== "screens"}><ConfigureScreens /></div>
  {#if screensState.tab === "looks"}<LooksView />{/if}
</PageFrame>

<style>
  .live {
    color: var(--text-label);
    white-space: nowrap;
  }
  .live b {
    color: var(--text);
    font-weight: 600;
  }
  .grow {
    flex: 1 1 20px;
  }
  .unsaved {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--accent-link);
    font-size: 12.5px;
    white-space: nowrap;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent-ring);
  }
  .tab {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
  }
  .tab.hidden {
    display: none;
  }
</style>
