// The per-client undo / redo stack. The server answers every edit with the ops that undo it
// (`inverse`); an entry's `run` sends them and returns the entry that redoes it (built from
// the answer's own `inverse`), so undo and redo are the same mechanism. CRUD actions (create,
// rename, delete) build their entries the same way from the documents the server returns.

export interface UndoEntry {
  /** What the menu shows after "Undo" / "Redo" ("Rename", "Typing", "Delete Playlist"). */
  label: string;
  /** Performs this step; returns the entry that reverts it again (null: cannot be reverted). */
  run(): Promise<UndoEntry | null>;
  /** Consecutive entries with the same key within `MERGE_MS` collapse into the first one. */
  mergeKey?: string;
}

export interface UndoState {
  undoLabel: string | null;
  redoLabel: string | null;
  busy: boolean;
}

/** Typing in the same slide within this time is one undo step. */
export const MERGE_MS = 2000;

export class UndoStack {
  private undos: { entry: UndoEntry; at: number }[] = [];
  private redos: UndoEntry[] = [];
  private busy = false;
  private listeners = new Set<(s: UndoState) => void>();

  constructor(
    private readonly limit = 100,
    private readonly now: () => number = () => Date.now(),
  ) {}

  /** Records a new action (clears the redo list). */
  push(entry: UndoEntry): void {
    const top = this.undos[this.undos.length - 1];
    const t = this.now();
    if (top && entry.mergeKey && top.entry.mergeKey === entry.mergeKey && t - top.at < MERGE_MS) {
      // The first entry's undo restores the state before the whole run of typing.
      top.at = t;
    } else {
      this.undos.push({ entry, at: t });
      if (this.undos.length > this.limit) this.undos.shift();
    }
    this.redos = [];
    this.emit();
  }

  get state(): UndoState {
    return {
      undoLabel: this.undos[this.undos.length - 1]?.entry.label ?? null,
      redoLabel: this.redos[this.redos.length - 1]?.label ?? null,
      busy: this.busy,
    };
  }

  canUndo(): boolean {
    return !this.busy && this.undos.length > 0;
  }

  canRedo(): boolean {
    return !this.busy && this.redos.length > 0;
  }

  /** Undoes the last action. Rejects (and drops the entry) when the server refuses it. */
  async undo(): Promise<string | null> {
    if (!this.canUndo()) return null;
    const { entry } = this.undos.pop()!;
    return this.step(entry, (r) => this.redos.push(r));
  }

  /** Redoes the last undone action. */
  async redo(): Promise<string | null> {
    if (!this.canRedo()) return null;
    const entry = this.redos.pop()!;
    return this.step(entry, (r) => this.undos.push({ entry: r, at: 0 }));
  }

  private async step(entry: UndoEntry, keep: (r: UndoEntry) => void): Promise<string> {
    this.busy = true;
    this.emit();
    try {
      const back = await entry.run();
      if (back) keep(back);
      return entry.label;
    } finally {
      this.busy = false;
      this.emit();
    }
  }

  clear(): void {
    this.undos = [];
    this.redos = [];
    this.emit();
  }

  /** Svelte store contract. */
  subscribe(fn: (s: UndoState) => void): () => void {
    this.listeners.add(fn);
    fn(this.state);
    return () => this.listeners.delete(fn);
  }

  private emit() {
    const s = this.state;
    for (const l of this.listeners) l(s);
  }
}
