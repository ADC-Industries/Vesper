// Content editing actions with undo: every edit goes to the server, which answers with the
// ops (or documents) that undo it; this builds the undo entries from those answers.

import type {
  DeletedDocuments,
  DocumentRevision,
  PlaylistEditResult,
  PlaylistOp,
  PlaylistPurpose,
  Presentation,
  PresentationEditResult,
  PresentationOp,
} from "$protocol";
import type { RestClient } from "../server";
import { uuid } from "./ids";
import { UndoStack, type UndoEntry } from "./undo";

export class Editor {
  readonly clientId = uuid();
  readonly undo = new UndoStack();

  constructor(private readonly rest: RestClient) {
    rest.clientId = this.clientId;
  }

  // ---- op-based documents --------------------------------------------------------------------

  private presentationEntry(id: string, label: string, ops: PresentationOp[], mergeKey?: string): UndoEntry {
    return {
      label,
      mergeKey,
      run: async () => {
        const r = await this.rest.editPresentation(id, { ops, clientId: this.clientId });
        return this.presentationEntry(id, label, r.inverse);
      },
    };
  }

  private playlistEntry(id: string, label: string, ops: PlaylistOp[]): UndoEntry {
    return {
      label,
      run: async () => {
        const r = await this.rest.editPlaylist(id, { ops, clientId: this.clientId });
        return this.playlistEntry(id, label, r.inverse);
      },
    };
  }

  /** Records an edit already applied (the Reflow queue sends its own). */
  recordPresentation(id: string, label: string, r: PresentationEditResult, mergeKey?: string): void {
    if (r.inverse.length) this.undo.push(this.presentationEntry(id, label, r.inverse, mergeKey));
  }

  async editPresentation(id: string, label: string, ops: PresentationOp[]): Promise<PresentationEditResult> {
    const r = await this.rest.editPresentation(id, { ops, clientId: this.clientId });
    this.recordPresentation(id, label, r);
    return r;
  }

  async editPlaylist(id: string, label: string, ops: PlaylistOp[]): Promise<PlaylistEditResult> {
    const r = await this.rest.editPlaylist(id, { ops, clientId: this.clientId });
    if (r.inverse.length) this.undo.push(this.playlistEntry(id, label, r.inverse));
    return r;
  }

  // ---- whole documents -------------------------------------------------------------------------

  /** An entry that deletes a document; its reverse recreates what the delete returned. */
  private deleteEntry(label: string, del: () => Promise<DeletedDocuments>, create: (doc: unknown) => Promise<unknown>): UndoEntry {
    return {
      label,
      run: async () => {
        const d = await del();
        return this.recreateEntry(label, d.documents, del, create);
      },
    };
  }

  private recreateEntry(
    label: string,
    docs: unknown[],
    del: () => Promise<DeletedDocuments>,
    create: (doc: unknown) => Promise<unknown>,
  ): UndoEntry {
    return {
      label,
      run: async () => {
        for (const d of docs) await create(d);
        return this.deleteEntry(label, del, create);
      },
    };
  }

  private libraryOps(id: string) {
    return {
      del: () => this.rest.deleteLibrary(id),
      create: (doc: unknown) => this.rest.createLibrary({ document: doc as never }),
    };
  }

  private presentationOps(id: string) {
    return {
      del: () => this.rest.deletePresentation(id),
      create: (doc: unknown) => this.rest.createPresentation({ document: doc as never }),
    };
  }

  private playlistOps(id: string) {
    return {
      del: () => this.rest.deletePlaylist(id),
      create: (doc: unknown) => this.rest.createPlaylist({ document: doc as never }),
    };
  }

  async createLibrary(name: string): Promise<DocumentRevision> {
    const r = await this.rest.createLibrary({ name });
    const o = this.libraryOps(r.id);
    this.undo.push(this.deleteEntry("New Library", o.del, o.create));
    return r;
  }

  async renameLibrary(id: string, from: string, to: string): Promise<void> {
    await this.rest.renameLibrary(id, to);
    this.undo.push(this.renameEntry("Rename", (n) => this.rest.renameLibrary(id, n), from, to));
  }

  async deleteLibrary(id: string): Promise<void> {
    const o = this.libraryOps(id);
    const d = await o.del();
    this.undo.push(this.recreateEntry("Delete Library", d.documents, o.del, o.create));
  }

  async createPresentation(name: string, library: string | null): Promise<DocumentRevision> {
    const r = await this.rest.createPresentation({ name, ...(library ? { library } : {}) });
    const o = this.presentationOps(r.id);
    this.undo.push(this.deleteEntry("New Presentation", o.del, o.create));
    return r;
  }

  /** A whole new presentation (a ChordPro import), in `library`; undo deletes it. */
  async importPresentation(doc: Presentation, library: string | null): Promise<DocumentRevision> {
    const r = await this.rest.createPresentation({ document: { ...doc, ...(library ? { library } : {}) } });
    const o = this.presentationOps(r.id);
    this.undo.push(this.deleteEntry("Import Song", o.del, o.create));
    return r;
  }

  async duplicatePresentation(id: string): Promise<DocumentRevision> {
    const r = await this.rest.duplicatePresentation(id);
    const o = this.presentationOps(r.id);
    this.undo.push(this.deleteEntry("Duplicate", o.del, o.create));
    return r;
  }

  async renamePresentation(id: string, name: string): Promise<void> {
    await this.editPresentation(id, "Rename", [{ type: "setPresentation", name }]);
  }

  async movePresentation(id: string, from: string | null, to: string): Promise<void> {
    await this.rest.movePresentation(id, to);
    if (from) this.undo.push(this.renameEntry("Move to Library", (lib) => this.rest.movePresentation(id, lib), from, to));
  }

  async deletePresentation(id: string): Promise<void> {
    const o = this.presentationOps(id);
    const d = await o.del();
    this.undo.push(this.recreateEntry("Delete Presentation", d.documents, o.del, o.create));
  }

  async createPlaylist(name: string, folder: boolean, parent: string | null, purpose?: PlaylistPurpose): Promise<DocumentRevision> {
    const r = await this.rest.createPlaylist({
      name,
      kind: folder ? "folder" : "playlist",
      ...(parent ? { parent } : {}),
      ...(purpose && !folder ? { purpose } : {}),
    });
    const o = this.playlistOps(r.id);
    this.undo.push(this.deleteEntry(folder ? "New Folder" : "New Playlist", o.del, o.create));
    return r;
  }

  async renamePlaylist(id: string, from: string, to: string): Promise<void> {
    await this.rest.updatePlaylist(id, { name: to });
    this.undo.push(this.renameEntry("Rename", (n) => this.rest.updatePlaylist(id, { name: n }), from, to));
  }

  async duplicatePlaylist(id: string): Promise<DocumentRevision> {
    const r = await this.rest.duplicatePlaylist(id);
    const o = this.playlistOps(r.id);
    this.undo.push(this.deleteEntry("Duplicate", o.del, o.create));
    return r;
  }

  async deletePlaylist(id: string, folder: boolean): Promise<void> {
    const o = this.playlistOps(id);
    const d = await o.del();
    this.undo.push(this.recreateEntry(folder ? "Delete Folder" : "Delete Playlist", d.documents, o.del, o.create));
  }

  private renameEntry(label: string, set: (name: string) => Promise<unknown>, from: string, to: string): UndoEntry {
    return {
      label,
      run: async () => {
        await set(from);
        return this.renameEntry(label, set, to, from);
      },
    };
  }
}
