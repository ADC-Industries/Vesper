<script lang="ts">
  // The right column's Messages tab: every message with one field per token, Show (or Update
  // while it is up) and Hide. + makes a message (the message dialog); the pencil or a
  // right-click edits, renames, duplicates or deletes one.
  import Plus from "lucide-svelte/icons/plus";
  import Pencil from "lucide-svelte/icons/pencil";
  import type { MessageEntry, MessageToken } from "$protocol";
  import type { Connection } from "../../server";
  import { session } from "../../state/session.svelte";
  import { showDocsState } from "../../state/showDocs.svelte";
  import { timersState } from "../../state/timers.svelte";
  import { shell } from "../../state/shell.svelte";
  import { useLive } from "../../live";
  import type { MenuItem } from "../../shell/menu";
  import Button from "../../ui/Button.svelte";
  import Chip from "../../ui/Chip.svelte";
  import { defaultValue, preview, syncTokens, tokenValues } from "./messages";

  let { conn }: { conn: Connection } = $props();
  const live = useLive();
  const layers = live.layers;

  let liveIds = $derived(new Map(($layers?.messages ?? []).map((m) => [m.message, m])));

  /** Token values typed per message (kept while the tab is open). */
  let typed = $state<Record<string, Record<string, string>>>({});

  /** A message's tokens: its own definitions, else the names used in its text. */
  function tokensOf(m: MessageEntry): MessageToken[] {
    return m.tokens?.length ? m.tokens : syncTokens(m.text ?? "", []);
  }

  function valuesOf(m: MessageEntry): Record<string, string> {
    const out: Record<string, string> = {};
    const shown = liveIds.get(m.id);
    for (const t of tokensOf(m)) out[t.name] = shown?.tokens.find((v) => v.name === t.name)?.value ?? defaultValue(t);
    return { ...out, ...(typed[m.id] ?? {}) };
  }

  function setVal(m: MessageEntry, name: string, v: string) {
    typed[m.id] = { ...(typed[m.id] ?? {}), [name]: v };
  }

  function show(m: MessageEntry) {
    void session.run({ type: "messageShow", message: m.id, tokens: tokenValues(tokensOf(m), valuesOf(m)) });
  }

  async function guard(what: string, fn: () => Promise<unknown>) {
    try {
      await fn();
    } catch (e) {
      conn.pushNotice("error", `${what} failed: ${e instanceof Error ? e.message : String(e)}`);
    }
    await showDocsState.load("message");
  }

  function menu(e: MouseEvent, m: MessageEntry) {
    e.preventDefault();
    showDocsState.selectedMessage = m.id;
    const items: MenuItem[] = [
      { type: "item", id: "edit", label: "Edit…", run: () => (shell.messageEditor = { entry: m }) },
      {
        type: "item",
        id: "rename",
        label: "Rename…",
        run: async () => {
          const n = await shell.askText("Rename Message", m.name ?? "", "Rename");
          if (n && n !== m.name) await guard("Rename", () => conn.rest.updateMessage({ ...m, name: n }, m.revision));
        },
      },
      {
        type: "item",
        id: "dup",
        label: "Duplicate",
        run: () => {
          const { id: _id, revision: _r, sortKey: _s, ...rest } = m;
          void guard("Duplicate", () => conn.rest.createMessage({ ...rest, name: `${m.name ?? "Message"} copy` }));
        },
      },
      { type: "separator" },
      {
        type: "item",
        id: "del",
        label: "Delete",
        run: async () => {
          if (await shell.confirm("Delete Message", `Delete “${m.name || "Message"}”?`)) await guard("Delete", () => conn.rest.deleteMessage(m.id, m.revision));
        },
      },
    ];
    shell.contextMenu = { x: e.clientX, y: e.clientY, items };
  }
</script>

<div class="messages" data-testid="messages-panel">
  {#each showDocsState.messageDocs as m (m.id)}
    {@const l = liveIds.get(m.id)}
    {@const vals = valuesOf(m)}
    {@const toks = tokensOf(m)}
    <section class="msg" class:up={!!l} oncontextmenu={(e) => menu(e, m)} aria-label={m.name || "Message"} data-testid="message-row">
      <div class="mhead">
        <span class="name">{m.name || "Message"}</span>
        {#if l}<Chip kind={l.visible ? "live" : "neutral"} title={l.visible ? "On screen" : "Showing under a later message"}>{l.visible ? "LIVE" : "UP"}</Chip>{/if}
        <button class="ibtn" onclick={() => (shell.messageEditor = { entry: m })} title="Edit the message" aria-label={`Edit ${m.name}`}><Pencil size={13} /></button>
      </div>
      {#each toks as t (t.name)}
        <label class="tok">
          <span>{t.name}</span>
          {#if !t.kind || t.kind.type === "text"}
            <input
              type="text"
              value={vals[t.name] ?? ""}
              placeholder={t.name}
              oninput={(e) => setVal(m, t.name, e.currentTarget.value)}
              onkeydown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") show(m);
              }}
              spellcheck="false"
              aria-label={`${m.name}: ${t.name}`}
              data-testid="message-token"
            />
          {:else if t.kind.type === "timer"}
            {@const tid = t.kind.timer?.id}
            <span class="ro">Timer: {timersState.timerDocs.find((x) => x.id === tid)?.name ?? "most recently started"}</span>
          {:else if t.kind.type === "clock"}
            <span class="ro">Clock</span>
          {:else}
            <span class="ro">Not supported</span>
          {/if}
        </label>
      {/each}
      {#if !toks.length}<p class="text">{m.text}</p>{/if}
      {#if l && toks.length}<p class="text" title="On screen">{l.text}</p>{:else if toks.length}<p class="text dim">{preview(m.text ?? "", toks, vals)}</p>{/if}
      <div class="actions">
        <Button variant={l ? "live" : "primary"} size="sm" onclick={() => show(m)} data-testid="message-show">{l ? "Update" : "Show message"}</Button>
        {#if l}<Button size="sm" onclick={() => session.run({ type: "messageHide", message: m.id })} data-testid="message-hide">Hide</Button>{/if}
      </div>
    </section>
  {:else}
    <p class="none">No messages yet.</p>
  {/each}
  <div class="foot">
    <Button size="sm" variant="ghost" icon={Plus} onclick={() => (shell.messageEditor = { entry: null })} data-testid="new-message">New Message</Button>
    {#if ($layers?.messages.length ?? 0) > 1}
      <Button size="sm" onclick={() => session.run({ type: "messageHide" })}>Hide All</Button>
    {/if}
  </div>
</div>

<style>
  .messages {
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .msg {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border: 1px solid var(--line);
    border-radius: var(--r-card);
    background: var(--surface-raised);
  }
  .msg.up {
    border-color: var(--layer-messages);
  }
  .mhead {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .name {
    flex: 1;
    min-width: 0;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tok {
    display: flex;
    flex-direction: column;
    gap: 3px;
    color: var(--text-2);
    font-size: 12px;
  }
  .tok input {
    width: 100%;
  }
  .ro {
    color: var(--text-meta);
  }
  .text {
    margin: 0;
    color: var(--text-2);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .text.dim {
    color: var(--text-meta);
  }
  .actions {
    display: flex;
    gap: 6px;
  }
  .none {
    margin: 0;
    color: var(--text-label);
    font-size: 12px;
  }
  .foot {
    display: flex;
    gap: 6px;
    justify-content: space-between;
  }
</style>
