<script lang="ts">
  // A theme's "second language" text style (batch 16, crates/model/src/language.rs): how
  // second-language text looks on slides styled from the theme when a screen shows both
  // languages (size relative to the first language, italic, colour, family, gap). Saved with
  // `PUT /themes/{id}` `{secondLanguage}` (If-Match); slides following the theme re-render.
  import type { SecondLanguageStyle } from "$protocol";
  import { session } from "../../state/session.svelte";
  import { fromHex, toHex } from "../../edit/color";
  import ColorWell from "../../ui/ColorWell.svelte";
  import Switch from "../../ui/Switch.svelte";
  import { errorText, SerialSaves } from "./secondLanguageStyle";
  import { untrack } from "svelte";

  /** `name`: the theme's name; a rename (a new revision on the server) reloads the style. */
  let { themeId, name = "", disabled = false }: { themeId: string; name?: string; disabled?: boolean } = $props();

  const DEFAULTS: Required<Omit<SecondLanguageStyle, "fontFamily">> = { sizeScale: 0.8, italic: true, color: { r: 1, g: 0.87, b: 0.56 }, gap: 24 };

  let style = $state<SecondLanguageStyle | null>(null);
  let problem = $state<string | null>(null);
  let saving = $state(false);
  // One save at a time, each with the revision the one before got (H11-8).
  const saves = new SerialSaves<{ id: string; style: SecondLanguageStyle }>(
    async (v, revision) => {
      const conn = session.conn;
      if (!conn) throw new Error("not connected");
      const r = await conn.rest.updateTheme(v.id, { secondLanguage: v.style }, revision);
      problem = null;
      return r.revision;
    },
    (e) => {
      problem = errorText(e);
      // Not saved: the panel shows the theme as the server has it again.
      load(themeId, true);
    },
    () => (saving = saves.pending > 0),
  );

  /** Loads the theme's style and revision (`keep`: keep the error shown). */
  function load(id: string, keep = false) {
    const conn = session.conn;
    if (!keep) problem = null;
    if (!conn) return;
    conn.rest
      .theme(id)
      .then((e) => {
        if (id !== themeId) return;
        saves.reset(e.revision);
        style = e.theme.secondLanguage ?? {};
      })
      .catch((e) => (problem = errorText(e)));
  }

  // A rename gives the theme a new revision: reload (the name is the sign of it).
  $effect(() => {
    const id = themeId;
    void name;
    void session.conn;
    style = null;
    untrack(() => load(id));
  });

  let scale = $derived(Math.round((style?.sizeScale ?? DEFAULTS.sizeScale) * 100));
  let italic = $derived(style?.italic ?? DEFAULTS.italic);
  let colour = $derived(toHex(style?.color ?? DEFAULTS.color));
  let gap = $derived(style?.gap ?? DEFAULTS.gap);

  function save(patch: Partial<SecondLanguageStyle>) {
    if (!session.conn || !style) return;
    const next = { ...style, ...patch };
    style = next;
    saving = true;
    void saves.save({ id: themeId, style: next });
  }
</script>

<section class="second" aria-label="Second language style" data-testid="theme-second-language">
  <h3 class="section-label">Second language</h3>
  <p class="hint">How a slide's second-language text looks when a screen shows both languages.</p>
  {#if style}
    <div class="grid">
      <label for="sl-size">Size</label>
      <span class="row"
        ><input id="sl-size" type="number" min="20" max="200" step="5" value={scale} {disabled} onchange={(e) => void save({ sizeScale: Math.min(200, Math.max(20, Number(e.currentTarget.value) || 80)) / 100 })} /> % of the first language</span
      >
      <span class="lab">Italic</span>
      <Switch label="Italic" checked={italic} disabled={disabled || saving} onchange={(c) => void save({ italic: c })} />
      <span class="lab">Colour</span>
      <ColorWell label="Second-language colour" value={colour} showHex {disabled} onchange={(v) => void save({ color: fromHex(v) })} />
      <label for="sl-gap">Gap</label>
      <span class="row"><input id="sl-gap" type="number" min="0" max="400" step="4" value={gap} {disabled} onchange={(e) => void save({ gap: Math.min(400, Math.max(0, Number(e.currentTarget.value) || 0)) })} /> px</span>
    </div>
  {:else if !problem}
    <p class="hint">Loading…</p>
  {/if}
  {#if problem}<p class="err" role="alert">{problem}</p>{/if}
</section>

<style>
  .second {
    margin-top: 14px;
    padding-top: 10px;
    border-top: 1px solid var(--line);
  }
  h3 {
    margin: 0 0 2px;
  }
  .hint {
    margin: 0 0 8px;
    color: var(--text-meta);
    font-size: var(--fs-xs);
  }
  .grid {
    display: grid;
    grid-template-columns: 70px auto;
    align-items: center;
    gap: 6px 10px;
    font-size: var(--fs-sm);
    color: var(--text-label);
  }
  .row {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  input[type="number"] {
    width: 64px;
    padding: 2px 6px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--surface-sunken, var(--surface-panel));
    color: var(--text);
  }
  .err {
    margin: 6px 0 0;
    color: var(--danger, #e5484d);
    font-size: var(--fs-sm);
  }
</style>
