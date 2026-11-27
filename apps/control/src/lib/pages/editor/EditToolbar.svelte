<script lang="ts">
  // The Edit page's toolbar (the canvas's "Editor tools" row): the document's name, Undo / Redo,
  // the inserts (Text, Shape ▾, Media ▾, Live text ▾, Scrolling text, Timer ▾) and Done.
  // Media ▸ Video adds a video element (it plays muted in its box on the screens, plan G16).
  import Undo2 from "lucide-svelte/icons/undo-2";
  import Redo2 from "lucide-svelte/icons/redo-2";
  import Shapes from "lucide-svelte/icons/shapes";
  import ImageIcon from "lucide-svelte/icons/image";
  import LinkIcon from "lucide-svelte/icons/link";
  import ScrollText from "lucide-svelte/icons/scroll-text";
  import Timer from "lucide-svelte/icons/timer";
  import ChevronDown from "lucide-svelte/icons/chevron-down";
  import type { MediaBinItemInfo } from "$protocol";
  import type { Connection } from "../../server";
  import type { EditState } from "../../edit/editState.svelte";
  import { SHAPE_CHOICES, type ShapeChoice } from "../../edit/elements";
  import { session } from "../../state/session.svelte";
  import { shell } from "../../state/shell.svelte";
  import { timersState } from "../../state/timers.svelte";
  import { DATA_LINKS, timerLink } from "../stage/stage";
  import { Button, Menu, PageToolbar, Popover } from "../../ui";
  import MediaPicker from "./MediaPicker.svelte";
  import PresenceBadge from "../../components/PresenceBadge.svelte";
  import type { DocumentKind } from "$protocol";
  import { insertDataLink, insertImage, insertScrollingText, insertShape, insertText } from "./inserts";

  let {
    conn,
    st,
    title,
    kind,
    docKind = "presentation",
    docId = null,
    onedit,
  }: { conn: Connection; st: EditState; title: string; kind: string; docKind?: DocumentKind; docId?: string | null; onedit: (id: string) => void } = $props();


  let open = $state<"shape" | "media" | "link" | "timer" | "image" | "video" | null>(null);
  let undo = $derived(session.undoState);
  let none = $derived(!st.slide);

  function toggle(m: typeof open) {
    open = open === m ? null : m;
  }

  function pickShape(id: string) {
    open = null;
    void insertShape(st, id as ShapeChoice);
  }
  function pickMedia(id: string) {
    open = id === "image" ? "image" : id === "video" ? "video" : null;
  }
  function pickImage(it: MediaBinItemInfo) {
    open = null;
    void insertImage(st, conn, it);
  }
  function pickLink(id: string) {
    open = null;
    const c = DATA_LINKS.find((x) => x.id === id);
    if (c) void insertDataLink(st, c);
  }
  function pickTimer(id: string) {
    open = null;
    const t = timersState.timerDocs.find((x) => x.id === id);
    void insertDataLink(st, timerLink(id === "last" ? null : (t ?? null)));
  }
</script>

<PageToolbar {title} subtitle={kind} ondone={() => shell.close()}>
  <PresenceBadge kind={docKind} id={docId} />
  <span class="grow"></span>
  <div class="hist" role="group" aria-label="History">
    <Button size="sm" iconOnly icon={Undo2} disabled={!undo.undoLabel || undo.busy} title={undo.undoLabel ? `Undo ${undo.undoLabel} (Ctrl+Z)` : "Nothing to undo"} aria-label="Undo" onclick={() => session.undo()} />
    <Button size="sm" iconOnly icon={Redo2} disabled={!undo.redoLabel || undo.busy} title={undo.redoLabel ? `Redo ${undo.redoLabel} (Ctrl+Shift+Z)` : "Nothing to redo"} aria-label="Redo" onclick={() => session.redo()} />
  </div>
  <div class="ins" role="group" aria-label="Insert" data-edit-inserts>
    <Button size="sm" disabled={none} title="Text box" onclick={() => void insertText(st, onedit)}><span class="t" aria-hidden="true">T</span>Text</Button>
    <span class="anchor">
      <Button size="sm" icon={Shapes} disabled={none} pressed={open === "shape"} aria-haspopup="menu" aria-expanded={open === "shape"} onclick={() => toggle("shape")} data-insert="shape">Shape<ChevronDown size={13} /></Button>
      {#if open === "shape"}
        <Popover role="none" width={200} onclose={() => (open = null)}>
          <Menu label="Shapes" items={SHAPE_CHOICES.map((c) => ({ id: c.id, label: c.label }))} onselect={pickShape} onclose={() => (open = null)} />
        </Popover>
      {/if}
    </span>
    <span class="anchor">
      <Button size="sm" icon={ImageIcon} disabled={none} pressed={open === "media" || open === "image"} aria-haspopup="menu" aria-expanded={open === "media"} onclick={() => toggle("media")} data-insert="media">Media<ChevronDown size={13} /></Button>
      {#if open === "media"}
        <Popover role="none" width={240} onclose={() => (open = null)}>
          <Menu
            label="Media"
            items={[
              { id: "image", label: "Image from the media bins…" },
              { id: "video", label: "Video from the media bins…" },
            ]}
            onselect={pickMedia}
            onclose={() => (open = null)}
          />
        </Popover>
      {/if}
      {#if open === "image"}<MediaPicker {conn} onpick={pickImage} onclose={() => (open = null)} />{/if}
      {#if open === "video"}<MediaPicker {conn} kind="video" onpick={pickImage} onclose={() => (open = null)} />{/if}
    </span>
    <span class="anchor">
      <Button size="sm" icon={LinkIcon} disabled={none} pressed={open === "link"} aria-haspopup="menu" aria-expanded={open === "link"} title="Live text: current / next slide, notes, clock, …" onclick={() => toggle("link")} data-insert="link">Live text<ChevronDown size={13} /></Button>
      {#if open === "link"}
        <Popover role="none" width={240} onclose={() => (open = null)}>
          <Menu label="Live text" items={DATA_LINKS.map((c) => ({ id: c.id, label: c.label }))} onselect={pickLink} onclose={() => (open = null)} />
        </Popover>
      {/if}
    </span>
    <Button size="sm" icon={ScrollText} disabled={none} title="Scrolling text: a ticker that moves through its box" onclick={() => void insertScrollingText(st, onedit)} data-insert="scroll">Scrolling text</Button>
    <span class="anchor">
      <Button size="sm" icon={Timer} disabled={none} pressed={open === "timer"} aria-haspopup="menu" aria-expanded={open === "timer"} title="A timer's time" onclick={() => toggle("timer")} data-insert="timer">Timer<ChevronDown size={13} /></Button>
      {#if open === "timer"}
        <Popover role="none" width={240} align="right" onclose={() => (open = null)}>
          <Menu
            label="Timer"
            items={[{ id: "last", label: "Most Recently Started" }, ...timersState.timerDocs.map((t) => ({ id: t.id, label: t.name || "Timer" }))]}
            onselect={pickTimer}
            onclose={() => (open = null)}
          />
        </Popover>
      {/if}
    </span>
  </div>
</PageToolbar>

<style>
  .grow {
    flex: 1 1 20px;
  }
  .hist,
  .ins {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .anchor {
    position: relative;
  }
  .t {
    font-family: Georgia, serif;
    font-weight: 600;
    margin-right: 2px;
  }
</style>
