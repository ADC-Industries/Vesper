import { describe, expect, it } from "vitest";
import { UndoStack, type UndoEntry } from "./undo";

/** A fake document: a value, with entries that set it and return the reverse. */
function setter(doc: { v: number }, label: string, to: number, from: number, mergeKey?: string): UndoEntry {
  return {
    label,
    mergeKey,
    run: async () => {
      doc.v = to;
      return setter(doc, label, from, to);
    },
  };
}

describe("UndoStack", () => {
  it("undoes and redoes through the entries the server answers with", async () => {
    const doc = { v: 0 };
    const s = new UndoStack();
    doc.v = 1;
    s.push(setter(doc, "Rename", 0, 1));
    doc.v = 2;
    s.push(setter(doc, "Typing", 1, 2));
    expect(s.state.undoLabel).toBe("Typing");
    expect(await s.undo()).toBe("Typing");
    expect(doc.v).toBe(1);
    expect(s.state).toMatchObject({ undoLabel: "Rename", redoLabel: "Typing" });
    await s.undo();
    expect(doc.v).toBe(0);
    expect(await s.undo()).toBeNull();
    await s.redo();
    await s.redo();
    expect(doc.v).toBe(2);
    expect(s.state).toMatchObject({ undoLabel: "Typing", redoLabel: null });
  });

  it("a new action clears the redo list", async () => {
    const doc = { v: 0 };
    const s = new UndoStack();
    s.push(setter(doc, "A", 0, 1));
    await s.undo();
    expect(s.canRedo()).toBe(true);
    s.push(setter(doc, "B", 0, 2));
    expect(s.canRedo()).toBe(false);
  });

  it("merges typing in the same slide within the window", async () => {
    let t = 0;
    const doc = { v: 3 };
    const s = new UndoStack(100, () => t);
    s.push(setter(doc, "Typing", 0, 1, "text:a"));
    t = 500;
    s.push(setter(doc, "Typing", 1, 2, "text:a"));
    t = 900;
    s.push(setter(doc, "Typing", 2, 3, "text:a"));
    t = 5000;
    s.push(setter(doc, "Typing", 3, 4, "text:a"));
    // Two steps: the first restores the state before the whole first run of typing.
    await s.undo();
    expect(doc.v).toBe(3);
    await s.undo();
    expect(doc.v).toBe(0);
    expect(s.canUndo()).toBe(false);
  });

  it("drops an entry the server refuses and reports the error", async () => {
    const s = new UndoStack();
    s.push({ label: "Delete", run: async () => Promise.reject(new Error("gone")) });
    await expect(s.undo()).rejects.toThrow("gone");
    expect(s.state).toMatchObject({ undoLabel: null, busy: false });
  });

  it("keeps a bounded history and notifies subscribers", () => {
    const s = new UndoStack(3);
    const seen: (string | null)[] = [];
    s.subscribe((st) => seen.push(st.undoLabel));
    for (let i = 0; i < 5; i++) s.push(setter({ v: 0 }, `E${i}`, 0, 0));
    expect(seen.at(-1)).toBe("E4");
    let n = 0;
    while (s.canUndo()) {
      void s.undo();
      n++;
      if (n > 10) break;
    }
    expect(n).toBeLessThanOrEqual(3);
  });
});
