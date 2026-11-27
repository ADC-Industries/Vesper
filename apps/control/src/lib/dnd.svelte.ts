// Pointer-based drag and drop for the left column: presentations into playlists (onto a
// playlist row, or at a position in the open playlist's item list with an insertion line) and
// playlist items to reorder them. Pointer events rather than HTML5 drag and drop, so the list
// can switch under the pointer (hovering a playlist row opens it) and touch screens work.

//
// Audio files (`kind: "audio"`, `id` = the media asset) drag from the audio panel onto audio
// playlist rows (`data-drop-audio-playlist`) only.
//
// Playlists and folders (`kind: "node"`) drag within the playlist tree: onto a folder
// (`data-tree-node`, `data-tree-folder`) to move into it, onto a playlist to move before it, or
// onto the "move to the top level" zone (`data-tree-root`). Hovering a closed folder while
// dragging a node or a presentation opens it (spring-loaded). Media bin items (`kind: "media"`,
// `id` = the asset) drop on slides (`data-drop-slide`) as media cues.

export type DragPayload =
  | { kind: "presentation"; id: string; name: string }
  | { kind: "item"; id: string; name: string }
  | { kind: "audio"; id: string; name: string }
  | { kind: "node"; id: string; name: string; folder: boolean }
  | { kind: "media"; id: string; name: string };

/** Where a playlist tree node is dropped. */
export type NodeDrop = { into: string } | { before: string } | { root: true };

/** Hovering a playlist row this long while dragging opens it (drop at a position). */
export const SPRING_MS = 600;
const THRESHOLD = 5;

export interface DropHandlers {
  /** Dropped on a playlist row. */
  onPlaylist(playlistId: string, p: DragPayload): void;
  /** Dropped in the item list before display row `index` (`after` = the row above it). */
  onList(after: string | null, p: DragPayload): void;
  /** Hovered a playlist row long enough. */
  onSpring(playlistId: string): void;
  /** A playlist or folder dropped in the tree. */
  onNode?(drop: NodeDrop, p: DragPayload): void;
  /** Hovered a closed folder long enough while dragging (open it). */
  onFolderSpring?(folderId: string): void;
  /** A media item dropped on a slide (`data-drop-slide`'s value). */
  onSlide?(target: string, p: DragPayload): void;
}

class DragState {
  payload = $state.raw<DragPayload | null>(null);
  x = $state(0);
  y = $state(0);
  overPlaylist = $state<string | null>(null);
  /** The tree drop under the pointer while dragging a node: `into:<id>`, `before:<id>` or `root`. */
  overNode = $state<string | null>(null);
  /** The folder under the pointer that will spring open. */
  overFolder = $state<string | null>(null);
  /** The slide under the pointer while dragging media (`data-drop-slide`). */
  overSlide = $state<string | null>(null);
  /** Insertion line position: the y (px, viewport) and left/width of the list. */
  line = $state.raw<{ y: number; left: number; width: number } | null>(null);
  private after: string | null | undefined = undefined;
  private pending: { x: number; y: number; payload: DragPayload } | null = null;
  private suppress = false;
  private spring: ReturnType<typeof setTimeout> | null = null;
  private folderSpring: ReturnType<typeof setTimeout> | null = null;
  private handlers: DropHandlers | null = null;

  /** Pointer down on a draggable row. */
  press(e: PointerEvent, payload: DragPayload, handlers: DropHandlers) {
    if (e.button !== 0) return;
    // Whatever an earlier press left (it can't have ended without `end`, but be sure).
    this.end();
    this.pending = { x: e.clientX, y: e.clientY, payload };
    this.handlers = handlers;
    window.addEventListener("pointermove", this.move);
    window.addEventListener("pointerup", this.up, { once: true });
    // A touch pan the browser takes over (or the window losing focus) ends the press without
    // a pointerup: forget it, so a later pointerup can't drop it.
    window.addEventListener("pointercancel", this.cancel);
    window.addEventListener("blur", this.cancel);
    window.addEventListener("keydown", this.key, true);
  }

  /** True once after a drop: the click that follows must not open the row. */
  consumeClick(): boolean {
    const s = this.suppress;
    this.suppress = false;
    return s;
  }

  private key = (e: KeyboardEvent) => {
    if (e.key === "Escape" && this.payload) {
      e.preventDefault();
      e.stopImmediatePropagation();
      this.end();
      // The release that follows still clicks the row it ends on: that click isn't a pick.
      window.addEventListener("pointerup", this.swallow, { once: true });
    }
  };

  /** Ends a press or drag without dropping anything. */
  private cancel = () => {
    const dragging = !!this.payload;
    this.end();
    if (dragging) this.swallow();
  };

  /** The next click (right after this) is the end of a drag, not a click. */
  private swallow = () => {
    this.suppress = true;
    setTimeout(() => (this.suppress = false), 0);
  };

  private move = (e: PointerEvent) => {
    if (!this.payload) {
      const p = this.pending;
      if (!p || Math.hypot(e.clientX - p.x, e.clientY - p.y) < THRESHOLD) return;
      this.payload = p.payload;
    }
    this.x = e.clientX;
    this.y = e.clientY;
    this.hit(e.clientX, e.clientY);
  };

  private hit(x: number, y: number) {
    const el = document.elementFromPoint(x, y) as HTMLElement | null;
    const kind = this.payload?.kind;
    if (kind === "media") {
      this.overSlide = el?.closest<HTMLElement>("[data-drop-slide]")?.dataset.dropSlide ?? null;
      return;
    }
    // Spring-loaded folders (dragging a node or a presentation).
    const nodeEl = kind === "node" || kind === "presentation" ? el?.closest<HTMLElement>("[data-tree-node]") : null;
    const folder = nodeEl?.dataset.treeFolder === "closed" ? (nodeEl.dataset.treeNode ?? null) : null;
    if (folder !== this.overFolder) {
      this.overFolder = folder;
      if (this.folderSpring) clearTimeout(this.folderSpring);
      this.folderSpring = folder ? setTimeout(() => folder === this.overFolder && this.handlers?.onFolderSpring?.(folder), SPRING_MS) : null;
    }
    if (kind === "node") {
      const id = nodeEl?.dataset.treeNode;
      const self = this.payload?.id;
      if (el?.closest("[data-tree-root]")) this.overNode = "root";
      else if (id && id !== self) this.overNode = (nodeEl!.dataset.treeFolder ? "into:" : "before:") + id;
      else this.overNode = null;
      return;
    }
    const row = el?.closest<HTMLElement>(kind === "audio" ? "[data-drop-audio-playlist]" : "[data-drop-playlist]");
    const pl =
      kind === "audio" ? (row?.dataset.dropAudioPlaylist ?? null) : kind === "presentation" ? (row?.dataset.dropPlaylist ?? null) : null;
    if (pl !== this.overPlaylist) {
      this.overPlaylist = pl;
      if (this.spring) clearTimeout(this.spring);
      this.spring = pl ? setTimeout(() => pl === this.overPlaylist && this.handlers?.onSpring(pl), SPRING_MS) : null;
    }
    const list = el?.closest<HTMLElement>("[data-drop-list]");
    if (!list || pl || kind === "audio") {
      this.line = null;
      this.after = undefined;
      return;
    }
    const rows = [...list.querySelectorAll<HTMLElement>("[data-item-id]")];
    const box = list.getBoundingClientRect();
    let after: string | null = null;
    let lineY = box.top + 2;
    for (const r of rows) {
      const b = r.getBoundingClientRect();
      if (y < b.top + b.height / 2) break;
      after = r.dataset.itemId ?? null;
      lineY = b.bottom;
    }
    this.after = after;
    this.line = { y: lineY, left: box.left + 4, width: box.width - 8 };
  }

  private up = () => {
    const p = this.payload;
    const h = this.handlers;
    if (p && h) {
      this.swallow();
      if (p.kind === "media") {
        if (this.overSlide) h.onSlide?.(this.overSlide, p);
      } else if (p.kind === "node") {
        const o = this.overNode;
        if (o === "root") h.onNode?.({ root: true }, p);
        else if (o?.startsWith("into:")) h.onNode?.({ into: o.slice(5) }, p);
        else if (o?.startsWith("before:")) h.onNode?.({ before: o.slice(7) }, p);
      } else if (this.overPlaylist) h.onPlaylist(this.overPlaylist, p);
      else if (this.after !== undefined) h.onList(this.after, p);
    }
    this.end();
  };

  private end() {
    window.removeEventListener("pointermove", this.move);
    window.removeEventListener("pointerup", this.up);
    window.removeEventListener("pointercancel", this.cancel);
    window.removeEventListener("blur", this.cancel);
    window.removeEventListener("keydown", this.key, true);
    if (this.spring) clearTimeout(this.spring);
    this.spring = null;
    if (this.folderSpring) clearTimeout(this.folderSpring);
    this.folderSpring = null;
    this.overNode = null;
    this.overFolder = null;
    this.overSlide = null;
    this.pending = null;
    this.payload = null;
    this.overPlaylist = null;
    this.line = null;
    this.after = undefined;
  }
}

export const drag = new DragState();
