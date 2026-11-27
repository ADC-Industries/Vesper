<script lang="ts">
  // The header on every page (docs/ORIEL-REDESIGN-PLAN.md 3.1): the Oriel menu, Show / Edit /
  // Reflow, the page buttons (the current one highlighted; on Settings the Oriel menu's button,
  // where Settings lives), the Look picker, "?" and the connection pill.
  import BookOpen from "lucide-svelte/icons/book-open";
  import Library from "lucide-svelte/icons/library";
  import Presentation from "lucide-svelte/icons/presentation";
  import Timer from "lucide-svelte/icons/timer";
  import Monitor from "lucide-svelte/icons/monitor";
  import Network from "lucide-svelte/icons/network";
  import Segmented from "../ui/Segmented.svelte";
  import type { IconComponent } from "../ui/types";
  import type { Connection } from "../server";
  import { isTauri } from "../tauri";
  import { shell } from "../state/shell.svelte";
  import OrielMenu from "./OrielMenu.svelte";
  import LookPicker from "./LookPicker.svelte";
  import ConnectionPill from "./ConnectionPill.svelte";
  import { HEADER_PAGES, type Mode, type PageId } from "./pages";
  import { shortcutLabel } from "./shortcuts";

  let { conn }: { conn: Connection } = $props();

  const ICONS: Record<Exclude<PageId, "settings">, IconComponent> = {
    bible: BookOpen,
    library: Library,
    stage: Presentation,
    timers: Timer,
    screens: Monitor,
    site: Network,
  };
  const desktop = isTauri();
  const keyHint = (id: Parameters<typeof shortcutLabel>[0]) => {
    const k = shortcutLabel(id, desktop);
    return k ? ` (${k})` : "";
  };

  const modes = [
    { value: "show" as Mode, label: "Show", title: `Show${keyHint("viewShow")}` },
    { value: "edit" as Mode, label: "Edit", title: `Edit the selected presentation${keyHint("viewEdit")}` },
    { value: "reflow" as Mode, label: "Reflow", title: `Reflow: the lyrics as text${keyHint("viewReflow")}` },
  ];
  // Off the Show page no mode is lit (the page button is).
  let mode = $derived<Mode | "">(shell.page === "show" ? shell.mode : "");
</script>

<header class="header">
  <OrielMenu />
  <Segmented label="Mode" options={modes} value={mode as Mode} onchange={(m) => shell.go(m)} />
  <nav class="pages" aria-label="Pages">
    {#each HEADER_PAGES as p (p.id)}
      {@const Icon = ICONS[p.id as keyof typeof ICONS]}
      {@const ok = shell.available(p.id)}
      <button
        class="page"
        class:on={shell.page === p.id}
        aria-current={shell.page === p.id ? "page" : undefined}
        disabled={!ok}
        title={ok ? `${p.title}${keyHint(p.action)}` : p.missing}
        onclick={() => shell.appAction(p.action)}
      >
        <Icon size={16} strokeWidth={1.8} aria-hidden="true" />{p.label}
      </button>
    {/each}
  </nav>
  <div class="spacer"></div>
  <LookPicker />
  <button class="help" aria-label="Keyboard shortcuts" title="Keyboard shortcuts (?)" onclick={() => shell.closeDialogs() && (shell.showHelp = !shell.showHelp)}>?</button>
  <ConnectionPill {conn} />
</header>

<style>
  .header {
    position: relative;
    z-index: 50;
    flex: none;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
    background: var(--surface-panel);
    border-bottom: 1px solid var(--line);
  }
  .pages {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  .page {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs);
  }
  .page :global(svg) {
    color: var(--text-label);
  }
  .page:hover:not(:disabled):not(.on) {
    background: var(--surface-raised);
    color: var(--text);
  }
  .page.on {
    background: var(--accent-sel);
    color: var(--text);
  }
  .page.on :global(svg) {
    color: var(--accent-ring);
  }
  .page:disabled {
    opacity: 0.45;
  }
  .spacer {
    flex: 1 1 20px;
  }
  .help {
    width: 30px;
    height: 30px;
    padding: 0;
    border-radius: 50%;
    border: 1px solid var(--line-strong);
    background: var(--surface-raised);
    color: var(--text);
    font-weight: 700;
  }
  .help:hover {
    border-color: var(--line-hover);
  }
</style>
