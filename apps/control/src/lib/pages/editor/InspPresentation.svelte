<script lang="ts">
  // Inspector, Presentation tab: background, size, language (the presentation's own: the
  // Translate dialog's default source), transition, copyright (with the song's artwork, which
  // Song Artwork links show).
  import type { MediaBinItemInfo, Presentation, SongInfo } from "$protocol";
  import type { Connection } from "../../server";
  import Button from "../../ui/Button.svelte";
  import MediaPicker from "./MediaPicker.svelte";
  import type { EditState } from "../../edit/editState.svelte";
  import { allSlides, presentationBackground, presentationBackgroundOps, songInfoOp, transitionLabel } from "../../edit/ops";
  import { KEYS, playedKey } from "../../edit/chords";
  import { languageChoices, languageCode } from "../../edit/language";
  import { shell } from "../../state/shell.svelte";
  import TransitionGrid from "./TransitionGrid.svelte";
  import ColorIn from "./ColorIn.svelte";
  import Num from "./Num.svelte";

  let { st, pres, conn }: { st: EditState; pres: Presentation; conn: Connection } = $props();
  let pickingArt = $state(false);
  function pickArtwork(it: MediaBinItemInfo) {
    pickingArt = false;
    void st.run("Artwork", [songInfoOp(pres, "artwork", it.asset)]);
  }

  const SIZES = [
    { w: 1920, h: 1080 },
    { w: 1280, h: 720 },
    { w: 3840, h: 2160 },
    { w: 1024, h: 768 },
    { w: 1080, h: 1920 },
  ];

  let bg = $derived(presentationBackground(pres));
  let size = $derived({ w: pres.canvasSize?.width ?? 1920, h: pres.canvasSize?.height ?? 1080 });
  let preset = $derived(SIZES.some((s) => s.w === size.w && s.h === size.h) ? `${size.w}x${size.h}` : "custom");
  let custom = $state(false);
  let info = $derived<SongInfo>(pres.songInfo ?? {});

  function setBg(on: boolean) {
    void st.run(on ? "Background" : "No Background", presentationBackgroundOps(pres, on ? (bg && bg !== "mixed" ? bg : { r: 0, g: 0, b: 0 }) : null));
  }

  function setSize(v: string) {
    if (v === "custom") {
      custom = true;
      return;
    }
    custom = false;
    const [w, h] = v.split("x").map(Number);
    if (w && h) void st.run("Canvas Size", [{ type: "setPresentation", canvasSize: { width: w, height: h } }]);
  }

  function setCustom(field: "width" | "height", v: number) {
    const next = { width: size.w, height: size.h, [field]: Math.round(v) };
    void st.run("Canvas Size", [{ type: "setPresentation", canvasSize: next }]);
  }

  function song(field: keyof SongInfo, v: string | number | boolean | null) {
    void st.run(field === "originalKey" ? "Song Key" : "Copyright", [songInfoOp(pres, field, v)]);
  }

  // The presentation's own language: a common one, or "Other…" for any BCP 47 code.
  const OTHER = "__other";
  let otherOpen = $state(false);
  let otherCode = $state("");
  let otherError = $state("");
  let langChoices = $derived(languageChoices(pres.language));
  function pickLanguage(v: string) {
    otherError = "";
    if (v === OTHER) {
      otherOpen = true;
      otherCode = pres.language ?? "";
      return;
    }
    otherOpen = false;
    void st.run("Language", [{ type: "setPresentation", language: v || null }]);
  }
  function closeOther() {
    otherOpen = false;
    otherError = "";
  }
  function setOther() {
    const code = languageCode(otherCode);
    if (!code) {
      otherError = "Type a language code such as es, pt-BR or zh-Hant.";
      return;
    }
    otherOpen = false;
    otherError = "";
    void st.run("Language", [{ type: "setPresentation", language: code }]);
  }

  // Key and transposition (batch 16).
  let transpose = $derived(info.transpose ?? 0);
  let played = $derived(playedKey(info.originalKey, transpose));
  let chordLines = $derived(allSlides(pres).reduce((n, s) => n + (s.elements ?? []).reduce((m, e) => m + (e.kind?.type === "text" ? (e.kind.text?.paragraphs ?? []).filter((p) => p.chords?.length).length : 0), 0), 0));
  function setTranspose(n: number) {
    // -11..11, kept small: +7 is -5.
    let t = ((n % 12) + 12) % 12;
    if (t > 6) t -= 12;
    void st.run("Transpose", [songInfoOp(pres, "transpose", t || null)]);
  }

  const FIELDS: { key: keyof SongInfo; label: string }[] = [
    { key: "title", label: "Title" },
    { key: "artist", label: "Artist" },
    { key: "authors", label: "Author" },
    { key: "album", label: "Album" },
    { key: "publisher", label: "Publisher" },
    { key: "ccliNumber", label: "CCLI #" },
  ];
</script>

<section class="sec">
  <label class="check">
    <input type="checkbox" checked={bg !== null} indeterminate={bg === "mixed"} onchange={(e) => setBg(e.currentTarget.checked)} data-field="pres-background" />
    Background
  </label>
  {#if bg !== null}
    <div class="line">
      <ColorIn color={bg === "mixed" ? null : bg} label="Background colour (every slide)" onchange={(c) => void st.run("Background", presentationBackgroundOps(pres, c))} />
      {#if bg === "mixed"}<span class="note">Slides differ</span>{/if}
    </div>
  {/if}
</section>

<section class="sec">
  <h4>Size</h4>
  <select class="wide" value={custom ? "custom" : preset} onchange={(e) => setSize(e.currentTarget.value)} aria-label="Canvas size" data-field="pres-size">
    {#each SIZES as s (s.w + "x" + s.h)}<option value={`${s.w}x${s.h}`}>{s.w} x {s.h}</option>{/each}
    <option value="custom">Custom…</option>
  </select>
  {#if custom || preset === "custom"}
    <div class="line">
      <Num label="Canvas width" value={size.w} min={64} max={8192} digits={0} onchange={(v) => setCustom("width", v)} />
      <span>×</span>
      <Num label="Canvas height" value={size.h} min={64} max={8192} digits={0} onchange={(v) => setCustom("height", v)} />
    </div>
  {/if}
</section>

<section class="sec" data-testid="pres-language">
  <h4>Language</h4>
  <select class="wide" value={otherOpen ? OTHER : (pres.language ?? "")} onchange={(e) => pickLanguage(e.currentTarget.value)} aria-label="Presentation language" data-field="pres-language">
    <option value="">Not set</option>
    {#each langChoices as l (l.code)}<option value={l.code}>{l.name}</option>{/each}
    <option value={OTHER}>Other…</option>
  </select>
  {#if otherOpen}
    <div class="line other">
      <input
        type="text"
        bind:value={otherCode}
        placeholder="Code, e.g. pt-BR"
        aria-label="Language code"
        spellcheck="false"
        onkeydown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter") setOther();
          if (e.key === "Escape") closeOther();
        }}
        data-field="pres-language-code"
      />
      <Button size="sm" onclick={setOther}>Set</Button>
      <Button size="sm" variant="ghost" onclick={closeOther}>Cancel</Button>
    </div>
    {#if otherError}<p class="note err" role="alert">{otherError}</p>{/if}
  {/if}
  <p class="note">The language the slides are written in. Translate starts from it.</p>
</section>

<section class="sec">
  <h4>Transition</h4>
  <TransitionGrid transition={pres.transition ?? undefined} owner={pres.id} defaultLabel="Default" label="Presentation transition" onchange={(t) => st.run("Transition", [{ type: "setPresentation", transition: t }])} />
  <p class="cur">{pres.transition ? transitionLabel(pres.transition) : "The layer's default transition"}</p>
</section>

<section class="sec" data-testid="song-key">
  <h4>Key &amp; Chords</h4>
  <div class="form">
    <label for="song-key">Written in</label>
    <select id="song-key" value={info.originalKey ?? ""} onchange={(e) => song("originalKey", e.currentTarget.value || null)} data-field="song-originalKey">
      <option value="">No key</option>
      {#if info.originalKey && !KEYS.includes(info.originalKey)}<option value={info.originalKey}>{info.originalKey}</option>{/if}
      {#each KEYS as k (k)}<option value={k}>{k}</option>{/each}
    </select>
    <span class="lab">Plays in</span>
    <div class="line transpose">
      <button class="step" aria-label="Transpose down a semitone" title="Down a semitone" onclick={() => setTranspose(transpose - 1)} data-testid="transpose-down">−</button>
      <span class="played" data-testid="played-key">{played || "—"}{transpose ? ` (${transpose > 0 ? "+" : ""}${transpose})` : ""}</span>
      <button class="step" aria-label="Transpose up a semitone" title="Up a semitone" onclick={() => setTranspose(transpose + 1)} data-testid="transpose-up">+</button>
      {#if transpose}<button class="link" onclick={() => setTranspose(0)}>As written</button>{/if}
    </div>
  </div>
  <div class="line">
    <Button size="sm" onclick={() => (shell.chordPro = { mode: "apply", presentation: pres.id })} data-testid="add-chordpro">Add Chords from ChordPro…</Button>
    <a class="link" href={conn.rest.chordProUrl(pres.id)} download data-testid="export-chordpro">Export ChordPro</a>
  </div>
  <p class="note">{chordLines ? `${chordLines} line${chordLines === 1 ? "" : "s"} with chords.` : "No chords yet."} Musicians see them on chord stage layouts, transposed for their capo; the audience never does.</p>
</section>

<section class="sec">
  <div class="line between">
    <h4>Copyright</h4>
    <label class="check"><input type="checkbox" checked={!!info.display} onchange={(e) => song("display", e.currentTarget.checked || null)} data-field="ccli-display" />Show On Screen</label>
  </div>
  <div class="form">
    {#each FIELDS as f (f.key)}
      <label for={`ccli-${f.key}`}>{f.label}</label>
      <input
        id={`ccli-${f.key}`}
        type="text"
        value={(info[f.key] as string | undefined) ?? ""}
        onchange={(e) => song(f.key, e.currentTarget.value.trim())}
        onkeydown={(e) => e.stopPropagation()}
        data-field={`ccli-${f.key}`}
      />
    {/each}
    <label for="ccli-year">Year</label>
    <input
      id="ccli-year"
      type="number"
      value={info.copyrightYear ?? ""}
      onchange={(e) => song("copyrightYear", e.currentTarget.value ? Math.round(Number(e.currentTarget.value)) : null)}
      onkeydown={(e) => e.stopPropagation()}
      data-field="ccli-copyrightYear"
    />
  </div>
  <div class="line art" data-field="song-artwork">
    <span class="lab">Artwork</span>
    {#if info.artwork}
      <img class="art-thumb" src={conn.rest.mediaThumbnailUrl(info.artwork, 96)} alt="Song artwork" draggable="false" />
    {/if}
    <Button size="sm" onclick={() => (pickingArt = !pickingArt)}>{info.artwork ? "Change…" : "Choose…"}</Button>
    {#if info.artwork}<Button size="sm" variant="ghost" onclick={() => void st.run("Remove Artwork", [songInfoOp(pres, "artwork", null)])}>Remove</Button>{/if}
  </div>
  {#if pickingArt}<div class="pop-anchor"><MediaPicker {conn} align="right" onpick={pickArtwork} onclose={() => (pickingArt = false)} /></div>{/if}
  <p class="note">Song Artwork objects (Insert ▸ Live text) show this image on the screens.</p>
</section>

<style>
  .other input {
    flex: 1;
    min-width: 0;
  }
  /* Beats the inspector's grey `.note`. */
  .sec p.note.err {
    color: var(--danger, #f2777a);
  }
  .transpose {
    gap: 6px;
  }
  .step {
    width: 24px;
    height: 22px;
    border: 1px solid var(--line);
    border-radius: 4px;
    background: var(--surface-raised, var(--surface-panel));
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }
  .played {
    min-width: 64px;
    text-align: center;
    font-weight: 600;
  }
  .link {
    border: none;
    background: none;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
    padding: 0 4px;
  }
  .art-thumb {
    width: 40px;
    height: 40px;
    object-fit: cover;
    border-radius: 3px;
    border: 1px solid var(--line);
    background: var(--surface-sunk);
  }
</style>
