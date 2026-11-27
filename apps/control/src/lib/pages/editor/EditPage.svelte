<script lang="ts">
  // The Edit page: the slide editor for the open presentation (or prop, or stage layout), or a
  // hint to pick one. The toolbar (Undo / Redo, inserts, Done) above the editor's body.
  import type { Connection } from "../../server";
  import { session } from "../../state/session.svelte";
  import { EditState } from "../../edit/editState.svelte";
  import EditView from "./EditView.svelte";
  import EditToolbar from "./EditToolbar.svelte";
  import EditPlaceholder from "./EditPlaceholder.svelte";
  import LockBanner from "../../components/LockBanner.svelte";
  import { copyTyping, presence } from "../../state/presence.svelte";
  import type { DocumentKind } from "$protocol";

  let { conn }: { conn: Connection } = $props();

  let kind = $derived(session.open?.stageLayout ? "Stage layout" : session.open?.theme ? "Theme" : session.open?.prop ? "Prop" : "");
  // One editor state per opening (a fresh one once nothing is open).
  let st = $state(new EditState());
  $effect(() => {
    if (!session.open) st = new EditState();
  });
  // An editor state that goes away (the page closes, or nothing is open any more) reports the
  // typing it kept from a lost edit lock, with its text (D10-1).
  $effect(() => {
    const s = st;
    return () => s.reportLocked();
  });
  let view = $state<{ editText: (id: string, selectAll?: boolean) => void } | null>(null);

  // Presence (G8): this page has the document open; someone else holding its edit lock makes
  // the canvas and inspector read-only.
  let docKind = $derived<DocumentKind>(session.open?.stageLayout ? "stageLayout" : session.open?.theme ? "theme" : session.open?.prop ? "prop" : "presentation");
  let docId = $derived(session.open?.presentation.id ?? null);
  $effect(() => {
    presence.setOpen("edit", docId ? [{ kind: docKind, id: docId, place: "edit" }] : null);
  });
  $effect(() => () => presence.setOpen("edit", null));
  let locked = $derived(!!presence.lockedBy(docKind, docId));
  // Typing refused by the lock: sent once nobody else holds it (taken back, or they closed it).
  $effect(() => {
    if (!locked && st.lockedEdits.length) void st.sendLocked();
  });
  const NOUN: Record<string, string> = { stageLayout: "stage layout", theme: "theme", prop: "prop", presentation: "presentation" };
</script>

{#if session.open}
  <section class="page" aria-label="Edit">
    <EditToolbar {conn} {st} title={session.open.presentation.name ?? "Untitled"} {kind} {docKind} {docId} onedit={(id) => view?.editText(id, true)} />
    <LockBanner kind={docKind} id={docId} noun={NOUN[docKind] ?? "document"} kept={st.lockedEdits.length} oncopy={() => void copyTyping(st.takeLockedText())} />
    <div class="body"><EditView {conn} {st} {locked} bind:this={view} /></div>
  </section>
{:else}
  <EditPlaceholder />
{/if}

<style>
  .page {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .body {
    flex: 1;
    min-height: 0;
  }
</style>
