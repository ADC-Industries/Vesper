<script lang="ts">
  // Inspector, Slide tab: the slide's second language (docs/DUAL-LANGUAGE-PLAN.md): its second
  // text as it is, and "Translate to…" for the slide being edited: Reflow's dialog, preview and
  // Keep / Discard (components/TranslateDialog, TranslateBanner, edit/translatePreview). The
  // proposal shows here in the machine-translation style until it is kept or discarded; moving
  // to another slide discards it.
  import Sparkles from "lucide-svelte/icons/sparkles";
  import type { Slide } from "$protocol";
  import type { Connection } from "../../server";
  import type { EditState } from "../../edit/editState.svelte";
  import { isMachineTranslation, MACHINE_NOTICE, presentationLanguage, proposedText, secondText, slideLanguage } from "../../edit/language";
  import { primaryTextIndex } from "../../edit/reflow";
  import { TranslatePreview } from "../../edit/translatePreview.svelte";
  import { session } from "../../state/session.svelte";
  import TranslateDialog from "../../components/TranslateDialog.svelte";
  import TranslateBanner from "../../components/TranslateBanner.svelte";
  import { onDestroy, untrack } from "svelte";

  let { conn, st, slide }: { conn: Connection; st: EditState; slide: Slide } = $props();

  let canTranslate = $derived(!!session.server?.capabilities.includes("translate"));
  let elementId = $derived.by(() => {
    const i = primaryTextIndex(slide);
    return i >= 0 ? (slide.elements?.[i]?.id ?? null) : null;
  });
  let second = $derived(secondText(slide, elementId));
  let language = $derived(slideLanguage(slide) || presentationLanguage((st.presentation?.sections ?? []).flatMap((s) => s.slides ?? [])));
  let open = $state(false);
  // svelte-ignore state_referenced_locally
  const mt = new TranslatePreview(conn);
  let proposal = $derived(mt.proposals[slide.id]);

  // Another slide: the preview was for the one before.
  let shownFor: string | null = null;
  $effect.pre(() => {
    const id = slide.id;
    untrack(() => {
      if (shownFor !== null && id !== shownFor) mt.cancel();
      shownFor = id;
    });
  });
  onDestroy(() => mt.cancel());

  function start(choice: { from: string; to: string }) {
    const p = st.presentation;
    open = false;
    if (p) void mt.start({ presentation: p.id, slides: [slide.id], from: choice.from, to: choice.to });
  }

  function keep() {
    const slides = new Set((st.presentation?.sections ?? []).flatMap((s) => (s.slides ?? []).map((x) => x.id)));
    void mt.keep((ops) => st.run("Machine Translation", ops), (id) => slides.has(id));
  }
</script>

<section class="sec" data-testid="slide-language">
  <h4>Second language{#if language}<span class="code">{language}</span>{/if}</h4>
  {#if proposal}
    <textarea class="text machine" readonly rows="3" lang={mt.job?.to || undefined} value={proposedText(proposal, elementId)} title={MACHINE_NOTICE} aria-label="Machine translation of the slide (not saved)" data-testid="slide-proposal"></textarea>
  {:else if second}
    <div class="current">
      {#if isMachineTranslation(slide)}<span class="mttag" title="Machine translation, not checked yet: editing it in Reflow clears this mark">MT</span>{/if}
      <p class="text">{second}</p>
    </div>
  {:else if !elementId}
    <p class="none">The slide has no text.</p>
  {:else}
    <p class="none">No second-language text. Type one in Reflow ▸ Second language{canTranslate ? ", or translate the slide" : ""}.</p>
  {/if}
  <TranslateBanner preview={mt} compact where="shown above" onkeep={keep} />
  {#if canTranslate && !mt.job}
    <button type="button" class="tr" onclick={() => (open = true)} disabled={!elementId} title="Machine-translate this slide into a second language (offline): a preview you keep or discard" data-testid="slide-translate"><Sparkles size={14} strokeWidth={1.8} aria-hidden="true" />Translate to…</button>
  {/if}
</section>

{#if open}
  <TranslateDialog {conn} presentation={st.presentation} title="Translate this slide" what="The slide's text" {language} onstart={start} onclose={() => (open = false)} />
{/if}

<style>
  h4 .code {
    margin-left: 6px;
    padding: 0 5px;
    border-radius: 3px;
    background: var(--surface-raised);
    color: var(--text-meta);
    font-weight: 600;
    text-transform: none;
  }
  .current {
    position: relative;
  }
  .text {
    box-sizing: border-box;
    width: 100%;
    margin: 0 0 8px;
    padding: 6px 8px;
    border-radius: var(--r);
    background: #e9e4d2;
    color: #4a3f1f;
    font-size: var(--fs-sm);
    line-height: 1.4;
    white-space: pre-wrap;
    max-height: 140px;
    overflow: auto;
  }
  textarea.text {
    border: 0;
    resize: vertical;
    font-family: inherit;
  }
  /* A machine translation shown before it is kept: dashed, tinted, read-only (as in Reflow). */
  .text.machine {
    background: repeating-linear-gradient(135deg, #e6eef9 0 10px, #dde7f6 10px 20px);
    color: #1f3554;
    font-style: italic;
    outline: 2px dashed #5b84c4;
    outline-offset: -2px;
  }
  .mttag {
    position: absolute;
    top: 3px;
    right: 4px;
    padding: 0 4px;
    border-radius: 3px;
    background: #5b84c4;
    color: #fff;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
  }
  .none {
    margin: 0 0 8px;
    color: var(--text-meta);
    font-size: var(--fs-sm);
  }
  .tr {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-top: 8px;
    padding: 4px 10px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text);
    font-size: var(--fs-sm);
  }
  .tr:disabled {
    opacity: 0.5;
  }
</style>
