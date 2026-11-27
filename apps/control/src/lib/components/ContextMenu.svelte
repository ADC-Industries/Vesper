<script lang="ts">
  // Right-click menu: the menu bar's model and keyboard handling (menu.ts) for one menu at the
  // pointer. Closes on a click outside, Esc, blur or after an item runs.
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import { clickItem, hoverItem, menuKey, type MenuDef, type MenuItem, type MenuNav } from "../shell/menu";
  import { shell } from "../state/shell.svelte";

  let { x, y, items }: { x: number; y: number; items: MenuItem[] } = $props();

  let menus = $derived<MenuDef[]>([{ id: "ctx", label: "", items }]);
  let nav = $state<MenuNav>({ open: 0, path: [-1] });
  let root = $state<HTMLElement | null>(null);
  let pos = $state({ left: 0, top: 0 });

  // Keep the menu inside the window.
  $effect(() => {
    if (!root) return;
    const r = root.getBoundingClientRect();
    pos = {
      left: Math.max(4, Math.min(x, window.innerWidth - r.width - 4)),
      top: Math.max(4, Math.min(y, window.innerHeight - r.height - 4)),
    };
  });

  function close() {
    shell.contextMenu = null;
  }

  function run(fn: (() => void) | undefined) {
    close();
    if (fn) queueMicrotask(fn);
  }

  function onKey(e: KeyboardEvent) {
    const r = menuKey(menus, nav, e.key);
    if (r.nav.open === null) {
      e.preventDefault();
      e.stopImmediatePropagation();
      run(r.run);
      return;
    }
    nav = r.nav;
    if (r.handled) {
      e.preventDefault();
      e.stopImmediatePropagation();
    }
  }

  function onPointerDown(e: PointerEvent) {
    if (root && !root.contains(e.target as Node)) close();
  }

  /** Keeps a submenu on screen: shifts it up when it would run off the bottom. */
  function fit(node: HTMLElement) {
    const r = node.getBoundingClientRect();
    const over = r.bottom - (window.innerHeight - 4);
    if (over > 0) node.style.top = `${-4 - over}px`;
  }

  function onClick(level: number, i: number) {
    const r = clickItem(menus, nav, level, i);
    if (r.nav.open === null) run(r.run);
    else nav = r.nav;
  }
</script>

<svelte:window onkeydowncapture={onKey} onpointerdowncapture={onPointerDown} onblur={close} onresize={close} />

{#snippet list(entries: MenuItem[], level: number)}
  <ul class="menu" class:sub={level > 0} role="menu" {@attach (n: HTMLElement) => (level > 0 ? fit(n) : undefined)}>
    {#each entries as it, i (i)}
      {#if it.type === "separator"}
        <li class="sep" role="separator"></li>
      {:else if it.type === "header"}
        <li class="hdr" role="presentation">{it.label}</li>
      {:else}
        {@const hot = nav.path[level] === i}
        {@const subOpen = hot && !!it.submenu && nav.path.length > level + 1}
        <li class="li" role="none">
          <button
            role="menuitem"
            class="mi"
            class:hot
            class:dis={it.disabled}
            class:danger={it.id.startsWith("delete")}
            aria-disabled={it.disabled ? "true" : undefined}
            aria-haspopup={it.submenu ? "menu" : undefined}
            title={it.title}
            tabindex="-1"
            data-menu-item={it.id}
            onpointerenter={() => (nav = hoverItem(menus, nav, level, i))}
            onclick={() => onClick(level, i)}
          >
            <span class="lbl">{it.label}</span>
            <span class="sc">{it.shortcut ?? ""}</span>
            <span class="arrow">{#if it.submenu}<ChevronRight size={13} />{/if}</span>
          </button>
          {#if subOpen && it.submenu}{@render list(it.submenu, level + 1)}{/if}
        </li>
      {/if}
    {/each}
  </ul>
{/snippet}

<div class="ctx" bind:this={root} style:left={`${pos.left || x}px`} style:top={`${pos.top || y}px`} role="presentation" oncontextmenu={(e) => e.preventDefault()}>
  {@render list(items, 0)}
</div>

<style>
  .ctx {
    position: fixed;
    z-index: 90;
  }
  .menu {
    position: relative;
    min-width: 200px;
    margin: 0;
    padding: 3px 0;
    list-style: none;
    background: var(--menu-bg);
    border: 1px solid var(--line);
    box-shadow: 2px 4px 14px rgba(0, 0, 0, 0.45);
    color: var(--text-strong);
  }
  .menu.sub {
    position: absolute;
    top: -4px;
    left: 100%;
    min-width: 180px;
    max-height: 60vh;
    overflow-y: auto;
  }
  .li {
    position: relative;
  }
  .mi {
    width: 100%;
    display: grid;
    grid-template-columns: 1fr auto 18px;
    align-items: center;
    height: 26px;
    padding: 0 4px 0 14px;
    border: 0;
    background: transparent;
    color: inherit;
    text-align: left;
    white-space: nowrap;
    font-size: 13.5px;
  }
  .mi.hot {
    background: var(--menu-hover);
  }
  .mi.dis {
    color: var(--text-label);
    cursor: default;
  }
  .sc {
    padding-left: 24px;
    color: var(--text);
  }
  .arrow {
    display: grid;
    place-items: center;
  }
  .sep {
    height: 1px;
    margin: 3px 0;
    background: var(--menu-sep);
  }
  .hdr {
    height: 22px;
    display: flex;
    align-items: center;
    padding-left: 14px;
    font-style: italic;
    color: var(--text);
  }
</style>
