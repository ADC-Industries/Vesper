<script lang="ts">
  import Button from "../../ui/Button.svelte";
  // New / edit message: name, text with `${token}` placeholders, template (the built-in
  // lower third or a theme's template slide), and the tokens the text uses (text tokens with a
  // default value, or a timer / the clock).
  import type { MessageToken, TemplateRef } from "$protocol";
  import Dialog from "../../ui/Dialog.svelte";
  import { session } from "../../state/session.svelte";
  import { showDocsState } from "../../state/showDocs.svelte";
  import { timersState } from "../../state/timers.svelte";
  import { shell } from "../../state/shell.svelte";
  import { syncTokens } from "./messages";

  const entry = shell.messageEditor?.entry ?? null;
  let name = $state(entry?.name ?? "");
  let text = $state(entry?.text ?? "");
  let template = $state(entry?.template ? `${entry.template.theme.id}/${entry.template.slide}` : "");
  let tokens = $state<MessageToken[]>(entry?.tokens ?? []);
  let busy = $state(false);
  let error = $state<string | null>(null);

  let synced = $derived(syncTokens(text, tokens));

  function setKind(name: string, kind: string) {
    const next: MessageToken["kind"] =
      kind === "timer" ? { type: "timer" } : kind === "clock" ? { type: "clock" } : { type: "text", defaultValue: "" };
    tokens = [...synced.filter((t) => t.name !== name), { name, kind: next }];
  }

  function setTokenPatch(name: string, kind: MessageToken["kind"]) {
    tokens = [...synced.filter((t) => t.name !== name), { name, kind }];
  }

  function templateRef(): TemplateRef | undefined {
    if (!template) return undefined;
    const [theme, slide] = template.split("/");
    const th = showDocsState.themes.find((t) => t.id === theme);
    return { theme: { id: theme!, ...(th?.name ? { name: th.name } : {}) }, slide: slide! };
  }

  async function save() {
    const conn = session.conn;
    if (!conn) return;
    const n = name.trim();
    if (!n) {
      error = "Give the message a name.";
      return;
    }
    busy = true;
    error = null;
    const t = templateRef();
    const toks = synced;
    try {
      if (entry) {
        const { template: _t, ...rest } = entry;
        await conn.rest.updateMessage({ ...rest, name: n, text, tokens: toks, ...(t ? { template: t } : {}) }, entry.revision);
      } else {
        const e = await conn.rest.createMessage({ name: n, text, tokens: toks, ...(t ? { template: t } : {}) });
        showDocsState.selectedMessage = e.id;
      }
      await showDocsState.load("message");
      shell.messageEditor = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title={entry ? "Edit Message" : "New Message"} onclose={() => (shell.messageEditor = null)} width={520}>
  <div class="form" role="presentation" onkeydown={(e) => e.stopPropagation()}>
    <label><span>Name</span><input type="text" bind:value={name} spellcheck="false" data-testid="message-name" /></label>
    <label class="top">
      <span>Text</span>
      <textarea rows="3" bind:value={text} placeholder={"Parent of ${child}, please come to the nursery"} data-testid="message-text"></textarea>
    </label>
    <p class="hint">Write <code>{"${name}"}</code> where a value goes; each one becomes a token below.</p>
    <label>
      <span>Template</span>
      <select bind:value={template} aria-label="Template">
        <option value="">Built-in lower third</option>
        {#each showDocsState.themes as th (th.id)}
          <optgroup label={th.name}>
            {#each th.slides as s (s.id)}
              <option value={`${th.id}/${s.id}`}>{s.name || "Template slide"}</option>
            {/each}
          </optgroup>
        {/each}
      </select>
    </label>
    {#if synced.length}
      <div class="toks">
        <div class="th">Tokens</div>
        {#each synced as t (t.name)}
          <div class="tok">
            <code>{t.name}</code>
            <select value={t.kind?.type ?? "text"} onchange={(e) => setKind(t.name, e.currentTarget.value)} aria-label={`Token ${t.name} type`}>
              <option value="text">Text</option>
              <option value="timer">Timer</option>
              <option value="clock">Clock</option>
            </select>
            {#if !t.kind || t.kind.type === "text"}
              <input type="text" placeholder="Default value" value={t.kind?.type === "text" ? (t.kind.defaultValue ?? "") : ""} oninput={(e) => setTokenPatch(t.name, { type: "text", defaultValue: e.currentTarget.value })} aria-label={`Token ${t.name} default`} />
            {:else if t.kind.type === "timer"}
              {@const tid = t.kind.timer?.id ?? ""}
              <select value={tid} onchange={(e) => {
                const id = e.currentTarget.value;
                const doc = timersState.timerDocs.find((x) => x.id === id);
                setTokenPatch(t.name, id ? { type: "timer", timer: { id, ...(doc?.name ? { name: doc.name } : {}) } } : { type: "timer" });
              }} aria-label={`Token ${t.name} timer`}>
                <option value="">Most recently started</option>
                {#each timersState.timerDocs as d (d.id)}<option value={d.id}>{d.name || "Timer"}</option>{/each}
              </select>
            {:else}
              <span class="ro">Current time</span>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
    {#if error}<p class="err">{error}</p>{/if}
    <div class="buttons">
      <Button onclick={() => (shell.messageEditor = null)}>Cancel</Button>
      <Button variant="primary" onclick={save} disabled={busy} data-testid="message-save">{entry ? "Save" : "Create"}</Button>
    </div>
  </div>
</Dialog>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px 16px;
  }
  label {
    display: grid;
    grid-template-columns: 80px 1fr;
    align-items: center;
    gap: 10px;
  }
  label.top {
    align-items: start;
  }
  label > span {
    text-align: right;
    color: var(--text);
  }
  input,
  textarea,
  select {
    min-height: 26px;
    padding: 2px 6px;
    background: var(--surface-sunk);
    border: 1px solid var(--line-strong);
    border-radius: 2px;
    color: var(--text);
    font: inherit;
  }
  textarea {
    resize: vertical;
  }
  .hint {
    margin: -2px 0 0 90px;
    color: var(--text-faint);
    font-size: 12px;
  }
  .toks {
    margin-left: 90px;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .th {
    font-size: 12px;
    color: var(--text-label);
    text-transform: uppercase;
  }
  .tok {
    display: grid;
    grid-template-columns: 90px 90px 1fr;
    gap: 6px;
    align-items: center;
  }
  .ro {
    color: var(--text-label);
    font-style: italic;
  }
  .err {
    margin: 0;
    color: var(--danger);
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
</style>
