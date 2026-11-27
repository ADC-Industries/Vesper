<script lang="ts">
  // The Oriel menu: the logo button and the canvas's two-pane menu (categories left, items
  // right). Content from orielMenu.ts, keys from menu.ts (paneMenuKey). F10 opens it. Lit while
  // the Settings page is open (the header's highlight for it).
  import Check from "lucide-svelte/icons/check";
  import ChevronDown from "lucide-svelte/icons/chevron-down";
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import Logo from "../components/Logo.svelte";
  import { APP_NAME } from "../config";
  import { useLive } from "../live";
  import { isTauri } from "../tauri";
  import { session } from "../state/session.svelte";
  import { shell } from "../state/shell.svelte";
  import { showView } from "../state/show.svelte";
  import { slidesImport } from "../state/slidesImport.svelte";
  import { CLOSED, clickItem, hoverItem, inCategories, paneMenuKey, pickCategory, type MenuItem, type MenuNav } from "./menu";
  import { orielMenu } from "./orielMenu";

  const live = useLive();
  const screensEnabled = live.screensEnabled;
  const options = live.options;

  let nav = $state<MenuNav>(CLOSED);
  let root = $state<HTMLElement | null>(null);
  let logo = $state<HTMLButtonElement | null>(null);
  const desktop = isTauri();

  $effect(() => {
    shell.menuOpen = nav.open !== null;
  });

  let menus = $derived(
    orielMenu({
      appName: APP_NAME,
      desktop,
      route: shell.route,
      available: (r) => shell.available(r),
      undoLabel: session.undoState.undoLabel,
      redoLabel: session.undoState.redoLabel,
      undoBusy: session.undoState.busy,
      audienceOn: $screensEnabled.audience,
      stageOn: $screensEnabled.stage,
      continuous: $options.continuousPlaylist,
      slideView: showView.slideView,
      mediaVisible: showView.mediaVisible,
      act: {
        app: (id) => shell.appAction(id),
        newPlaylistFolder: () => void session.newPlaylist(true),
        newLibrary: () => void session.newLibrary(),
        setSlideView: (v) => showView.setSlideView(v),
        toggleContinuous: () => void session.run({ type: "setOption", continuousPlaylist: !$options.continuousPlaylist }),
        liveOutput: () => shell.closeDialogs() && (shell.showLive = true),
        fullScreen: toggleFullscreen,
        resetLayout: () => showView.resetLayout(),
        settings: (s) => shell.openSettings(s),
        shortcuts: () => shell.closeDialogs() && (shell.showHelp = true),
        diagnostics: () => shell.closeDialogs() && (shell.showDiagnostics = true),
        about: () => shell.closeDialogs() && (shell.showAbout = true),
        disconnect: () => session.disconnect(),
        importSlides: () => slidesImport.open(),
      },
    }),
  );

  let items = $derived(nav.open === null ? [] : (menus[nav.open]?.items ?? []));

  function toggleFullscreen() {
    if (typeof document === "undefined") return;
    if (document.fullscreenElement) void document.exitFullscreen();
    else void document.documentElement.requestFullscreen?.();
  }

  function open(keyboard: boolean) {
    shell.popover = null;
    nav = pickCategory(menus, 0);
    if (keyboard) queueMicrotask(() => focusCurrent());
  }

  function close(refocus = false) {
    nav = CLOSED;
    if (refocus) logo?.focus();
  }

  function run(fn: (() => void) | undefined) {
    if (fn) queueMicrotask(fn);
  }

  /** Keeps the DOM focus on the highlighted category or item (screen readers follow it). */
  function focusCurrent() {
    if (!root || nav.open === null) return;
    const sel = inCategories(nav) ? `[data-cat="${nav.open}"]` : `[data-item="${nav.path[0]}"]`;
    root.querySelector<HTMLElement>(sel)?.focus();
  }

  function onKey(e: KeyboardEvent) {
    if (nav.open === null) {
      if (e.key === "F10" && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey) {
        e.preventDefault();
        open(true);
      }
      return;
    }
    const r = paneMenuKey(menus, nav, e.key);
    const closing = r.nav.open === null;
    nav = r.nav;
    if (r.handled) {
      e.preventDefault();
      e.stopImmediatePropagation();
    }
    if (closing) close(!r.run && e.key === "Escape");
    else queueMicrotask(focusCurrent);
    run(r.run);
  }

  function onPointerDown(e: PointerEvent) {
    if (nav.open !== null && root && !root.contains(e.target as Node)) close();
  }

  function onItemClick(i: number) {
    const r = clickItem(menus, nav, 0, i);
    nav = r.nav;
    run(r.run);
  }

  function noteOf(it: MenuItem): string {
    return it.type === "item" ? (it.note ?? it.shortcut ?? "") : "";
  }
</script>

<svelte:window onkeydowncapture={onKey} onpointerdowncapture={onPointerDown} onblur={() => close()} />

<div class="oriel" bind:this={root}>
  <button
    bind:this={logo}
    class="logo"
    class:open={nav.open !== null}
    class:here={shell.page === "settings"}
    aria-current={shell.page === "settings" ? "page" : undefined}
    aria-haspopup="menu"
    aria-expanded={nav.open !== null}
    title={`${APP_NAME} menu (F10)`}
    onclick={(e) => (nav.open === null ? open(e.detail === 0) : close())}
  >
    <Logo size={22} />
    <span class="name">{APP_NAME}</span>
    <ChevronDown size={14} strokeWidth={1.8} />
  </button>
  {#if nav.open !== null}
    <div class="menu" role="menu" aria-label={`${APP_NAME} menu`}>
      <div class="cats">
        {#each menus as m, i (m.id)}
          <button
            role="menuitem"
            class="cat"
            class:on={nav.open === i}
            aria-haspopup="menu"
            aria-expanded={nav.open === i}
            tabindex="-1"
            data-cat={i}
            onclick={() => (nav = pickCategory(menus, i))}
            onpointerenter={() => (nav = pickCategory(menus, i))}
          >
            {m.label}<ChevronRight size={14} strokeWidth={1.8} />
          </button>
        {/each}
      </div>
      <div class="items" role="group" aria-label={menus[nav.open]?.label}>
        {#each items as it, i (i)}
          {#if it.type === "separator"}
            <div class="sep" role="separator"></div>
          {:else if it.type === "header"}
            <div class="hdr" role="presentation">{it.label}</div>
          {:else}
            <button
              role={it.checked === undefined ? "menuitem" : "menuitemcheckbox"}
              class="it"
              class:hot={!inCategories(nav) && nav.path[0] === i}
              class:dis={it.disabled}
              aria-disabled={it.disabled ? "true" : undefined}
              aria-checked={it.checked === undefined ? undefined : it.checked}
              title={it.title}
              tabindex="-1"
              data-item={i}
              onpointerenter={() => (nav = hoverItem(menus, nav, 0, i))}
              onclick={() => onItemClick(i)}
            >
              <span class="ck">{#if it.checked}<Check size={14} strokeWidth={2.4} />{/if}</span>
              <span class="lbl">{it.label}</span>
              <span class="sc" class:later={!!it.note}>{noteOf(it)}</span>
            </button>
          {/if}
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .oriel {
    position: relative;
    padding-right: 8px;
  }
  .logo {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    margin: -4px -8px;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--text);
  }
  .logo:hover,
  .logo.open {
    background: var(--surface-raised);
  }
  /* On the Settings page (it lives in this menu), lit like the header's page buttons. */
  .logo.here:not(.open) {
    background: var(--accent-sel);
  }
  .logo :global(svg:last-child) {
    color: var(--text-label);
  }
  .name {
    font-weight: 700;
    font-size: var(--fs-title);
    letter-spacing: 0.02em;
  }
  .menu {
    position: absolute;
    left: -4px;
    top: calc(100% + 8px);
    z-index: 60;
    display: flex;
    background: var(--surface-raised);
    border: 1px solid var(--line-strong);
    border-radius: var(--r-card);
    box-shadow: var(--shadow-menu);
    overflow: hidden;
  }
  .cats {
    width: 150px;
    padding: 4px;
    border-right: 1px solid var(--line-strong);
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .cat {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 7px 10px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text);
    text-align: left;
    font-size: var(--fs);
  }
  .cat :global(svg) {
    color: var(--text-label);
  }
  .cat.on {
    background: var(--accent-sel);
  }
  .items {
    width: 300px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    max-height: min(520px, calc(100vh - 80px));
    overflow-y: auto;
  }
  .it {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text);
    text-align: left;
    font-size: var(--fs);
    flex: none;
  }
  .it.hot:not(.dis),
  .it:focus-visible {
    background: var(--accent-sel);
    outline: none;
  }
  .it.dis {
    color: var(--text-disabled);
    cursor: default;
  }
  .ck {
    width: 14px;
    flex: none;
    display: grid;
    place-items: center;
    color: var(--accent-ring);
  }
  .lbl {
    flex: 1;
    white-space: nowrap;
  }
  .sc {
    color: var(--text-meta);
    font-size: 11.5px;
    white-space: nowrap;
  }
  .sc.later {
    font-style: italic;
    color: var(--text-disabled);
  }
  .sep {
    height: 1px;
    margin: 4px 6px;
    background: var(--line-strong);
    flex: none;
  }
  .hdr {
    padding: 8px 10px 3px 32px;
    font-size: var(--fs-xs);
    font-weight: 700;
    letter-spacing: var(--label-tracking);
    text-transform: uppercase;
    color: var(--text-label);
    flex: none;
  }
</style>
