<script lang="ts">
  // The main window once connected: the header, the connection banners, the current page
  // (shell.page / shell.mode, mirrored in the URL hash), dialogs and toasts. Every keydown goes
  // through the shortcut table (keyboard.ts).
  import { onMount, setContext } from "svelte";
  import type { Connection } from "../server";
  import { session } from "../state/session.svelte";
  import { shell } from "../state/shell.svelte";
  import { createLive, LIVE_KEY } from "../live";
  import { isTauri } from "../tauri";
  import { keyAction } from "./keyboard";
  import Header from "./Header.svelte";
  import Banners from "./Banners.svelte";
  import Toasts from "./Toasts.svelte";
  import ShortcutHelp from "./ShortcutHelp.svelte";
  import LivePanel from "./LivePanel.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import DiagnosticsPanel from "../components/DiagnosticsPanel.svelte";
  import AboutDialog from "../components/AboutDialog.svelte";
  import SlidesImportDialog from "../components/SlidesImportDialog.svelte";
  import UnlockDialog from "../pages/site/UnlockDialog.svelte";
  import { sitePage } from "../state/site.svelte";
  import ContextMenu from "../components/ContextMenu.svelte";
  import ChunkError from "../components/ChunkError.svelte";
  import PromptDialog from "../components/PromptDialog.svelte";
  import MessageDialog from "../pages/show/MessageDialog.svelte";
  import ChordProDialog from "../components/ChordProDialog.svelte";
  import SongSelectDialog from "../components/SongSelectDialog.svelte";
  import ShowPage from "../pages/show/ShowPage.svelte";
  // The slide editor (canvas, inspector panels, transitions) loads on first use too; the
  // Library's song editing shares its chunk.
  const editPage = () => import("../pages/editor/EditPage.svelte");
  import ReflowPage from "../pages/reflow/ReflowPage.svelte";
  // The Bible page loads on first use (its own chunk).
  const biblePage = () => import("../pages/bible/BiblePage.svelte");
  import LibraryPage from "../pages/library/LibraryPage.svelte";
  import StagePage from "../pages/stage/StagePage.svelte";
  import TimersPage from "../pages/timers/TimersPage.svelte";
  import ScreensPage from "../pages/screens/ScreensPage.svelte";
  // The Site page (administration) loads on first use, like the Bible page.
  const sitePageModule = () => import("../pages/site/SitePage.svelte");
  import SettingsPage from "../pages/settings/SettingsPage.svelte";

  let { conn }: { conn: Connection } = $props();

  // svelte-ignore state_referenced_locally
  const live = createLive(conn);
  setContext(LIVE_KEY, live);
  // svelte-ignore state_referenced_locally
  const status = conn.status;
  const desktop = isTauri();

  onMount(() => shell.initRouting());

  function onKeydown(e: KeyboardEvent) {
    if (shell.menuOpen) return;
    const action = keyAction(e, {
      dialogOpen: shell.dialogOpen,
      editing: shell.route === "edit" || shell.pageEditing,
      noShowKeys: shell.page === "screens",
      desktop,
    });
    if (!action) return;
    e.preventDefault();
    switch (action.kind) {
      case "closeDialog":
        shell.closeDialogs();
        break;
      case "help":
        shell.showHelp = !shell.showHelp;
        break;
      case "app":
        shell.appAction(action.id);
        break;
      case "command":
        void session.run(action.command);
        break;
    }
  }

  // After a mouse click, buttons and sliders give focus back to the page. Otherwise the
  // next Space would press the focused button again (re-triggering a slide or a clear)
  // instead of advancing, and arrows would move a slider. Keyboard users keep focus.
  function releaseFocus(e: Event) {
    const el = (e.target as HTMLElement | null)?.closest?.("button, input[type=range]");
    if (!el || !(el instanceof HTMLElement)) return;
    if (e instanceof MouseEvent && e.detail === 0) return;
    queueMicrotask(() => el.blur());
  }
</script>

<svelte:window onkeydown={onKeydown} onclickcapture={releaseFocus} />

<div class="shell" class:offline={$status.phase !== "online"}>
  <Header {conn} />
  <Banners {conn} />
  <main class="workspace">
    {#if shell.page === "show"}
      {#if shell.mode === "edit"}
        {#await editPage() then m}<m.default {conn} />{:catch e}<ChunkError error={e} what="The editor" />{/await}
      {:else if shell.mode === "reflow"}
        <ReflowPage {conn} />
      {:else}
        <ShowPage {conn} />
      {/if}
    {:else if shell.page === "bible"}
      {#await biblePage() then m}<m.default />{:catch e}<ChunkError error={e} what="The Bible page" />{/await}
    {:else if shell.page === "library"}
      <LibraryPage {conn} />
    {:else if shell.page === "stage"}
      <StagePage {conn} />
    {:else if shell.page === "timers"}
      <TimersPage {conn} />
    {:else if shell.page === "screens"}
      <ScreensPage />
    {:else if shell.page === "site"}
      {#await sitePageModule() then m}<m.default />{:catch e}<ChunkError error={e} what="The Site page" />{/await}
    {:else if shell.page === "settings"}
      <SettingsPage />
    {/if}
  </main>
  <Toasts {conn} />
  {#if shell.showHelp}<ShortcutHelp />{/if}
  {#if shell.showDiagnostics}<DiagnosticsPanel {conn} />{/if}
  {#if shell.showLive}
    <Dialog title="Live output" onclose={() => (shell.showLive = false)} width={500}><LivePanel {conn} /></Dialog>
  {/if}
  {#if shell.showAbout}<AboutDialog />{/if}
  {#if shell.showSlidesImport}<SlidesImportDialog />{/if}
  {#if sitePage.ask}<UnlockDialog ask={sitePage.ask} />{/if}
  {#if shell.messageEditor}{#key shell.messageEditor}<MessageDialog />{/key}{/if}
  {#if shell.chordPro}{#key shell.chordPro}<ChordProDialog />{/key}{/if}
  {#if shell.songSelect}{#key shell.songSelect}<SongSelectDialog />{/key}{/if}
  {#if shell.prompt}{#key shell.prompt}<PromptDialog />{/key}{/if}
  {#if shell.contextMenu}{#key shell.contextMenu}<ContextMenu x={shell.contextMenu.x} y={shell.contextMenu.y} items={shell.contextMenu.items} />{/key}{/if}
</div>

<style>
  .shell {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-width: 1180px;
    min-height: 600px;
    background: var(--surface-app);
  }
  /* While offline the live controls can't do anything: dim them so nobody trusts them. */
  .shell.offline :global(.strip),
  .shell.offline :global(.clear-all),
  .shell.offline :global(.slides .scroll),
  .shell.offline :global(.bin .scroll) {
    opacity: 0.45;
    pointer-events: none;
    filter: grayscale(0.6);
  }
  .workspace {
    flex: 1;
    min-height: 0;
    position: relative;
  }
</style>
