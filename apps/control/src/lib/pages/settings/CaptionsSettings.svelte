<script lang="ts">
  // Settings ▸ Captions (docs/CAPTIONS-PLAN.md): live captions on this server. Speech from the
  // program audio or a live source turned into text on the server (whisper, on its CPU), or
  // lines from another captioner; the language, the model, words to spell right, the box's
  // style and where captions go. Stored on the server and applied at once.
  import type { CaptionModel, CaptionsConfig, CaptionPosition, Color } from "$protocol";
  import Button from "../../ui/Button.svelte";
  import { ApiRequestError } from "../../server";
  import { session } from "../../state/session.svelte";
  import { sourcesState } from "../../state/sources.svelte";
  import { useLive } from "../../live";
  import { captionStatus, downloadBar, modelOptions, sourceOf, sourceValue, withDefaults } from "../show/captions";
  import { sourceName } from "../show/sources";

  const captions = useLive().captions;

  let saved = $state.raw<CaptionsConfig | null>(null);
  let draft = $state<CaptionsConfig | null>(null);
  let models = $state.raw<CaptionModel[]>([]);
  let loading = $state(true);
  let saving = $state(false);
  let error = $state<string | null>(null);

  let status = $derived(captionStatus($captions));
  let download = $derived(downloadBar($captions));
  let dirty = $derived(!!draft && !!saved && JSON.stringify(draft) !== JSON.stringify(saved));
  let source = $derived(sourceValue(draft?.source));
  let options = $derived(modelOptions(models, draft?.language ?? "en"));
  let sources = $derived.by(() => {
    const names = sourcesState.list.map((s) => s.name);
    const cur = draft?.source?.type === "liveSource" ? draft.source.name : null;
    if (cur && !names.includes(cur)) names.unshift(cur);
    return names;
  });

  function message(e: unknown): string {
    if (e instanceof ApiRequestError && (e.status === 404 || e.status === 405)) return "This server has no captions. Update the server.";
    return e instanceof Error ? e.message : String(e);
  }

  async function load() {
    const conn = session.conn;
    if (!conn) return;
    loading = true;
    error = null;
    try {
      const [cfg, list] = await Promise.all([conn.rest.captionsConfig(), conn.rest.captionModels()]);
      saved = withDefaults(cfg);
      draft = structuredClone(saved);
      models = list;
    } catch (e) {
      error = message(e);
    } finally {
      loading = false;
    }
  }
  void load();
  sourcesState.ensure();

  async function save() {
    const conn = session.conn;
    if (!conn || !draft) return;
    saving = true;
    error = null;
    try {
      saved = withDefaults(await conn.rest.saveCaptionsConfig($state.snapshot(draft)));
      draft = structuredClone(saved);
      conn.pushNotice("info", saved.enabled ? "Captions settings saved." : "Captions are off.");
    } catch (e) {
      error = message(e);
    } finally {
      saving = false;
    }
  }

  const hex = (c: Color | undefined, fallback: string) => {
    if (!c) return fallback;
    const h = (v: number) => Math.round(Math.min(1, Math.max(0, v)) * 255).toString(16).padStart(2, "0");
    return `#${h(c.r)}${h(c.g)}${h(c.b)}`;
  };
  const color = (v: string, a = 1): Color => ({ r: parseInt(v.slice(1, 3), 16) / 255, g: parseInt(v.slice(3, 5), 16) / 255, b: parseInt(v.slice(5, 7), 16) / 255, a });
  function style() {
    if (!draft) throw new Error("no draft");
    draft.style ??= {};
    return draft.style;
  }
  let boxAlpha = $derived(Math.round((draft?.style?.background?.a ?? 0.75) * 100));
</script>

<div class="cap">
  <div class="status {status.cls}" data-testid="captions-status">
    <span class="dot"></span>
    <span class="text">{status.text}{$captions?.device && $captions.status === "running" && $captions.model ? ` · ${$captions.device}` : ""}</span>
  </div>
  {#if download}
    <div class="download" data-testid="captions-download">
      <div
        class="bar"
        class:unknown={download.fraction == null}
        role="progressbar"
        aria-label="Model download"
        aria-valuemin="0"
        aria-valuemax="100"
        aria-valuenow={download.fraction != null ? Math.round(download.fraction * 100) : undefined}
      >
        <span style:width={download.fraction != null ? `${Math.round(download.fraction * 100)}%` : undefined}></span>
      </div>
      {#if download.text}<span class="amount">{download.text}</span>{/if}
    </div>
  {/if}
  {#if error}<p class="err" role="alert">{error}</p>{/if}

  {#if draft}
    <label class="check">
      <input type="checkbox" bind:checked={draft.enabled} data-testid="captions-enabled" />
      <span>Live captions on</span>
    </label>

    <div class="grid">
      <label class="fld">
        <span>Speech from</span>
        <select value={source} onchange={(e) => draft && (draft.source = sourceOf(e.currentTarget.value))} data-testid="captions-source">
          <option value="program">Program audio (everything Vesper plays)</option>
          {#each sources as n (n)}
            <option value={`live:${n}`}>Live source: {sourceName(n)}</option>
          {/each}
          <option value="external">Another captioner (lines sent to the server)</option>
        </select>
      </label>
      <label class="fld">
        <span>Language</span>
        <select bind:value={draft.language} onchange={() => draft && draft.language !== "en" && draft.model?.endsWith(".en") && (draft.model = "auto")}>
          {#each [["en", "English"], ["es", "Spanish"], ["fr", "French"], ["de", "German"], ["pt", "Portuguese"], ["ko", "Korean"], ["zh", "Chinese"]] as [code, name] (code)}
            <option value={code}>{name}</option>
          {/each}
        </select>
      </label>
      <label class="fld wide">
        <span>Model</span>
        <select bind:value={draft.model} disabled={source === "external"} data-testid="captions-model">
          {#each options as o (o.value)}
            <option value={o.value}>{o.label}</option>
          {/each}
        </select>
      </label>
      <label class="fld wide">
        <span>Names and words to spell right</span>
        <input type="text" bind:value={draft.words} placeholder="Pastor Example, Ephesians" maxlength="1000" disabled={source === "external"} />
      </label>
      <label class="check wide">
        <input type="checkbox" bind:checked={draft.keepMusic} disabled={source === "external"} />
        <span>Caption sung lines too (speech recognition invents words over music)</span>
      </label>
    </div>

    <h3>Look</h3>
    <div class="grid">
      <label class="fld">
        <span>Lines at once</span>
        <select bind:value={draft.maxLines}>
          {#each [1, 2, 3, 4] as n (n)}<option value={n}>{n}</option>{/each}
        </select>
      </label>
      <label class="fld">
        <span>Position</span>
        <select value={draft.style?.position ?? "bottom"} onchange={(e) => (style().position = e.currentTarget.value as CaptionPosition)}>
          <option value="bottom">Bottom</option>
          <option value="top">Top</option>
        </select>
      </label>
      <label class="fld">
        <span>Text size (px at 1080)</span>
        <input type="number" min="12" max="200" value={draft.style?.size ?? 56} onchange={(e) => (style().size = Number(e.currentTarget.value))} />
      </label>
      <label class="fld">
        <span>Font</span>
        <input type="text" value={draft.style?.font ?? ""} placeholder="The server's default" onchange={(e) => (style().font = e.currentTarget.value)} />
      </label>
      <label class="fld">
        <span>Text colour</span>
        <input type="color" value={hex(draft.style?.color, "#ffffff")} oninput={(e) => (style().color = color(e.currentTarget.value))} />
      </label>
      <label class="fld">
        <span>Box colour · opacity</span>
        <span class="line">
          <input type="color" value={hex(draft.style?.background, "#000000")} oninput={(e) => (style().background = color(e.currentTarget.value, boxAlpha / 100))} />
          <input type="range" min="0" max="100" value={boxAlpha} oninput={(e) => (style().background = color(hex(draft?.style?.background, "#000000"), Number(e.currentTarget.value) / 100))} aria-label="Box opacity" />
          <span class="unit">{boxAlpha} %</span>
        </span>
      </label>
    </div>

    <h3>Where</h3>
    <label class="check"><input type="checkbox" checked={draft.audience ?? true} onchange={(e) => draft && (draft.audience = e.currentTarget.checked)} /><span>Audience screens (looks can hide the Captions layer per screen)</span></label>
    <label class="check"><input type="checkbox" bind:checked={draft.stage} /><span>Stage screens too</span></label>
    <label class="check"><input type="checkbox" bind:checked={draft.network} /><span>Send on the network as an IDN Open Protocol (compatible with NDI®) text source called "Captions"</span></label>
  {/if}

  <div class="actions">
    <span class="grow"></span>
    {#if dirty}<Button onclick={() => load()} disabled={saving}>Revert</Button>{/if}
    <Button variant="primary" onclick={save} disabled={!dirty || saving || loading} data-testid="captions-save">{saving ? "Saving…" : "Save"}</Button>
  </div>

  <p class="note">
    Speech is recognised on the server; nothing but the text leaves it. The model is fetched once, when captions first start
    (75 MB to 1.6 GB, into the server's cache folder); Automatic measures the server and takes the largest model that keeps
    up. Languages other than English need the large model, which wants a fast machine.
  </p>
</div>

<style>
  .cap {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 640px;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dot {
    flex: none;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--line-hover);
  }
  .status.ok .dot {
    background: var(--ok);
  }
  .status.busy .dot {
    background: var(--accent);
  }
  .status.bad .dot {
    background: var(--danger);
  }
  .download {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: -4px;
  }
  .bar {
    flex: 1;
    min-width: 0;
    height: 6px;
    border-radius: 3px;
    background: var(--line);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.3s;
  }
  .bar.unknown span {
    width: 30%;
    animation: sweep 1.4s ease-in-out infinite;
  }
  @keyframes sweep {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(340%);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .bar.unknown span {
      animation: none;
    }
  }
  .amount {
    flex: none;
    font-size: 12px;
    color: var(--text-meta);
    font-variant-numeric: tabular-nums;
  }
  .status.bad .text,
  .err {
    color: var(--danger-text);
  }
  .err {
    margin: 0;
  }
  h3 {
    margin: 4px 0 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-label);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px 14px;
  }
  .wide {
    grid-column: 1 / -1;
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .fld > span:first-child {
    color: var(--text-2);
  }
  .fld select,
  .fld input[type="text"],
  .fld input[type="number"] {
    min-height: 28px;
    min-width: 0;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .unit {
    color: var(--text-label);
    font-variant-numeric: tabular-nums;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .grow {
    flex: 1;
  }
  .note {
    margin: 0;
    color: var(--text-label);
    font-size: 12.5px;
    line-height: 1.45;
  }
</style>
