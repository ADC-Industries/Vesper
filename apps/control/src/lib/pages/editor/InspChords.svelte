<script lang="ts">
  // Inspector, Text tab: the text object's chords (batch 16). Each line is shown with its chords
  // above the characters they sit over; click a character to put a chord over it, a chord to
  // change or remove it. "Show chords on the canvas" draws them on the editor's
  // picture of the slide (audience screens never show chords). The song's key is on the
  // Presentation tab; stage layouts choose how musicians read them (Stage page).
  import type { Element, TextBox } from "$protocol";
  import type { EditState } from "../../edit/editState.svelte";
  import { chordOnLine, paragraphText } from "../../edit/chords";
  import { shell } from "../../state/shell.svelte";

  let { st, el }: { st: EditState; el: Element } = $props();

  /** Bumped when the pointer or focus comes to the chart: while the text is being edited in
   *  place, the chart shows the typing so far (the editor's draft is not reactive). */
  let fresh = $state(0);
  let tb = $derived.by<TextBox>(() => {
    void fresh;
    const saved = el.kind?.type === "text" ? (el.kind.text ?? {}) : {};
    return (st.editingText === el.id ? st.textHooks?.draft() : null) ?? saved;
  });
  let lines = $derived(
    (tb.paragraphs ?? []).map((p) => {
      const chars = [...paragraphText(p)];
      const at = new Map<number, string>();
      for (const c of p.chords ?? []) {
        const i = Math.min(c.at ?? 0, chars.length);
        at.set(i, at.has(i) ? `${at.get(i)} ${c.chord}` : c.chord);
      }
      return { chars, at, count: p.chords?.length ?? 0 };
    }),
  );
  let total = $derived(lines.reduce((n, l) => n + l.count, 0));

  function save(line: number, at: number, chord: string, label: string) {
    // The line as the chart showed it: the chord goes over that character, or nowhere (H11-7).
    const expect = lines[line]?.chars.join("") ?? "";
    void st.editText(el, label, (target) => {
      const box = target.kind?.type === "text" ? (target.kind.text ?? {}) : {};
      const next = chordOnLine(box, line, expect, at, chord);
      if (!next) {
        fresh++;
        st.showInfo("The text changed while the chord was being placed: the chords show the new text now; click the letter again.");
        return [];
      }
      return [{ type: "replaceTextBox", slide: st.slideId!, element: el.id, textBox: next }];
    });
  }

  async function ask(line: number, at: number, current: string | undefined, over: string) {
    const what = over === "" ? "at the end of the line" : over === " " ? "over the space" : `over “${over}”`;
    const v = await shell.askText(current ? `Change the chord ${what}` : `Add a chord ${what}`, current ?? "", current ? "Change" : "Add", "Chord");
    if (v) save(line, at, v, current ? "Chord" : "Add Chord");
  }

  /** A letter: add (or change) its chord; a chord: Change… / Remove. */
  function click(e: MouseEvent, line: number, at: number, current: string | undefined, over: string) {
    if (!current) return void ask(line, at, current, over);
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    shell.contextMenu = {
      x: r.left,
      y: r.bottom + 2,
      items: [
        { type: "item", id: "change", label: "Change Chord…", run: () => void ask(line, at, current, over) },
        { type: "item", id: "remove", label: `Remove ${current}`, run: () => save(line, at, "", "Remove Chord") },
      ],
    };
  }
</script>

<section class="sec" data-testid="insp-chords" role="group" aria-label="Chords" onpointerenter={() => fresh++} onfocusin={() => fresh++}>
  <div class="line between">
    <h4>Chords</h4>
    <label class="check"><input type="checkbox" bind:checked={st.showChords} data-field="show-chords" />Show on canvas</label>
  </div>
  {#if lines.length}
    <div class="chart" role="group" aria-label="Chords of this text">
      {#each lines as l, i (i)}
        <div class="cline">
          {#each l.chars as ch, k (k)}
            <span class="cell">
              <button class="chord" class:set={l.at.has(k)} title={l.at.has(k) ? `Change ${l.at.get(k)}` : `Add a chord over “${ch}”`} onclick={(e) => click(e, i, k, l.at.get(k), ch)}>{l.at.get(k) ?? "+"}</button>
              <button class="ch" tabindex="-1" onclick={(e) => click(e, i, k, l.at.get(k), ch)}>{ch === " " ? " " : ch}</button>
            </span>
          {/each}
          <span class="cell">
            <button class="chord" class:set={l.at.has(l.chars.length)} title="A chord at the end of the line" onclick={(e) => click(e, i, l.chars.length, l.at.get(l.chars.length), "")}>{l.at.get(l.chars.length) ?? "+"}</button>
            <span class="ch end"> </span>
          </span>
        </div>
      {/each}
    </div>
    <p class="note">
      {total ? `${total} chord${total === 1 ? "" : "s"}.` : "No chords yet."} Click a letter to put a chord over it; click a chord to change or remove it. Chords show on musicians' stage layouts, never on the audience screens.
    </p>
  {:else}
    <p class="note">Type the words first, then add chords over them.</p>
  {/if}
</section>

<style>
  .chart {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 6px;
    border: 1px solid var(--line);
    border-radius: 4px;
    background: var(--surface-sunk);
    max-height: 260px;
    overflow: auto;
    font-size: 13px;
  }
  .cline {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
  }
  /* Each character with room above it for a chord; a chord doesn't widen its letter (it may
     run over the next ones, as on a chord sheet). */
  .cell {
    position: relative;
    display: inline-block;
    padding-top: 16px;
  }
  .chord {
    position: absolute;
    top: 0;
    left: 0;
    z-index: 1;
    min-height: 16px;
    min-width: 6px;
    padding: 0 2px;
    border: none;
    background: none;
    font: inherit;
    font-size: 11px;
    font-weight: 700;
    color: var(--warning, #e0a800);
    cursor: pointer;
    line-height: 16px;
    white-space: pre;
  }
  .chord:not(.set) {
    color: transparent;
    width: 100%;
  }
  .cell:hover .chord:not(.set) {
    color: var(--text-muted);
  }
  .chord.set {
    border-radius: 3px;
    background: color-mix(in srgb, var(--warning, #e0a800) 16%, var(--surface-sunk));
  }
  .chord.set:hover {
    outline: 1px solid var(--warning, #e0a800);
  }
  .ch {
    padding: 0;
    border: none;
    background: none;
    font: inherit;
    color: var(--text);
    cursor: pointer;
    white-space: pre;
    line-height: 18px;
  }
  .ch.end {
    width: 4px;
  }
</style>
