<script lang="ts">
  // "Translate to…" (docs/DUAL-LANGUAGE-PLAN.md §5), shared by Reflow and the slide editor: the
  // source language (the presentation's own language, else what the operator last picked for
  // it, else its Scripture translation's language, else English: `translateSourceOrder`), the target among those the server's
  // models write from that source, the model with its licence and size ("downloads first" when
  // it isn't on the server yet), the machine-translation notice. Starts nothing itself:
  // `onstart` gets the choice (edit/translatePreview.svelte.ts runs the job).
  import Dialog from "../ui/Dialog.svelte";
  import { untrack } from "svelte";
  import Loader from "lucide-svelte/icons/loader-circle";
  import Sparkles from "lucide-svelte/icons/sparkles";
  import type { Presentation, TranslationModel } from "$protocol";
  import type { Connection } from "../server";
  import { ScriptureClient } from "../server/scripture";
  import { defaultSource, languageName, MACHINE_NOTICE, translateSourceOrder, translationSources, translationTargets } from "../edit/language";

  let {
    conn,
    presentation,
    title,
    what,
    language = "",
    onstart,
    onclose,
  }: {
    conn: Connection;
    /** The presentation translated (its language is the default source). */
    presentation: Presentation | null;
    title: string;
    /** What is translated, as the hint's subject ("The slide's text"). */
    what: string;
    /** The second language already used (preferred target). */
    language?: string;
    onstart: (choice: { from: string; to: string }) => void;
    onclose: () => void;
  } = $props();

  let models = $state<TranslationModel[] | null>(null);
  let error = $state("");
  let to = $state("");
  let from = $state("en");

  let sources = $derived(translationSources(models ?? []));
  let targets = $derived(translationTargets(models ?? [], from));
  let target = $derived(targets.find((t) => t.code === to) ?? null);
  let model = $derived(target ? (models ?? []).find((m) => m.id === target.pair) ?? null : null);

  /** What the operator last translated this presentation from (this browser). */
  const remembered = (id: string | undefined): string | null => {
    try {
      return id ? localStorage.getItem(`oriel.translateFrom.${id}`) : null;
    } catch {
      return null;
    }
  };

  /** The language of the presentation's Scripture translation, if it is a passage. */
  async function scriptureLanguage(p: Presentation | null): Promise<string | null> {
    const t = p?.scripture?.translation;
    if (!t) return null;
    try {
      return (await new ScriptureClient(conn.rest).translations()).find((x) => x.id === t)?.language ?? null;
    } catch {
      return null;
    }
  }

  /** Keeps the target when the new source offers it, else the second language, else the first. */
  function pickTarget() {
    const t = translationTargets(models ?? [], from);
    if (!t.some((x) => x.code === to)) to = t.find((x) => x.code === language)?.code ?? t[0]?.code ?? "";
  }

  // Once per presentation (not on every edit of it: the operator's choice stays). Only the id
  // is tracked: every refetch of the presentation is a new object.
  let presentationId = $derived(presentation?.id);
  $effect(() => {
    void presentationId;
    const p = untrack(() => presentation);
    Promise.all([conn.rest.translationModels(), scriptureLanguage(p)])
      .then(([m, scripture]) => {
        models = m.models;
        from = defaultSource(translationSources(m.models), translateSourceOrder({ presentation: p?.language, remembered: remembered(p?.id), scripture }));
        pickTarget();
      })
      .catch((e) => (error = e instanceof Error ? e.message : String(e)));
  });

  function start() {
    try {
      if (presentation) localStorage.setItem(`oriel.translateFrom.${presentation.id}`, from);
    } catch {
      /* private mode */
    }
    onstart({ from, to });
  }
</script>

<div class="wrap" role="presentation" onkeydown={(e) => e.stopPropagation()}>
  <Dialog {title} width={560} escape {onclose}>
    <div class="body" data-testid="translate-dialog">
      <p class="hint">{what} is translated offline on the server. You see the result first: nothing is saved until you keep it.</p>
      {#if error}
        <p class="err">Translation models could not be read: {error}</p>
      {:else if !models}
        <p class="hint"><Loader size={14} class="spin" /> Reading the translation models…</p>
      {:else if !sources.length}
        <p class="err">This server offers no translation models.</p>
      {:else}
        <label class="pick">From
          <select bind:value={from} onchange={pickTarget} aria-label="Translate from">
            {#each sources as s (s.code)}<option value={s.code}>{s.name}</option>{/each}
          </select>
        </label>
        {#if !targets.length}
          <p class="err">This server offers no translation from {languageName(from)}.</p>
        {/if}
        <label class="pick">Into
          <select bind:value={to} aria-label="Translate into">
            {#each targets as t (t.code)}<option value={t.code}>{t.name}{t.ready ? "" : " (model downloads first)"}</option>{/each}
          </select>
        </label>
        {#if model}
          <p class="hint">Model: {model.model} · {model.licence} · {model.status === "ready" ? "on this server" : `downloads ${Math.round(model.size / 1_000_000)} MB on first use`}</p>
        {/if}
      {/if}
      <p class="notice"><Sparkles size={14} strokeWidth={1.8} aria-hidden="true" /> {MACHINE_NOTICE}.</p>
    </div>
    {#snippet footer()}
      <button type="button" class="btn" onclick={onclose}>Cancel</button>
      <button type="button" class="btn primary" disabled={!target} onclick={start}>Translate</button>
    {/snippet}
  </Dialog>
</div>

<style>
  .wrap {
    display: contents;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .hint {
    margin: 0;
    color: var(--text-meta);
    font-size: var(--fs-sm);
  }
  .pick {
    display: grid;
    grid-template-columns: 44px auto;
    justify-content: start;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-sm);
  }
  .pick select {
    min-width: 220px;
  }

  .notice {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    padding: 6px 8px;
    border-radius: var(--r);
    background: #1d2c45;
    color: #dbe6f7;
    font-size: var(--fs-sm);
  }
  .err {
    margin: 0;
    color: var(--danger, #f2777a);
    font-size: var(--fs-sm);
  }
  .btn {
    flex: none;
    white-space: nowrap;
    padding: 5px 12px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text);
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  .btn.primary {
    background: var(--accent, #8b7cf6);
    border-color: var(--accent, #8b7cf6);
    color: #fff;
  }
  .btn:disabled {
    opacity: 0.5;
  }
</style>
