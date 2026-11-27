<script lang="ts">
  // The text field under the canvas (plan question 2: in addition to typing in place): the
  // selected text object's text as plain lines. It saves on leaving the field (or Ctrl+Enter)
  // with the same check as in-place typing: when someone else changed the text meanwhile, the
  // conflict dialog asks which to keep.
  import { untrack } from "svelte";
  import type { TextBox } from "$protocol";
  import type { EditState } from "../../edit/editState.svelte";
  import { elementName } from "../../edit/elements";
  import { saveTextOps } from "../../edit/ops";
  import { plainText } from "../../edit/text";
  import { textBoxFromPlain } from "./textField";

  let { st }: { st: EditState } = $props();

  let el = $derived(st.single?.kind?.type === "text" ? st.single : null);
  let text = $derived(el?.kind?.type === "text" ? plainText(el.kind.text) : "");
  let label = $derived(el ? `Text · ${elementName(el)}` : "Text");
  let ta = $state<HTMLTextAreaElement | null>(null);
  // The text as it was when the operator started typing here. While the field has focus it
  // keeps what they type (a change from another client doesn't replace it), and saving checks
  // the server still has this text, so a change made meanwhile opens the conflict dialog.
  let base = $state<string | null>(null);

  /** What is being typed and where it goes, taken when the field got the focus: a click on the
   * canvas changes the selection (and replaces this field) before the field loses the focus,
   * so a save must not look at the selection then. */
  interface Typing {
    presentation: string;
    slide: string;
    element: string;
    box: TextBox;
    before: string;
    value: string;
  }
  let typing: Typing | null = null;

  function save(leaving = false): Promise<unknown> | undefined {
    const t = typing;
    if (!t) return;
    if (leaving) {
      typing = null;
      base = null;
    } else {
      base = t.value;
    }
    if (t.value === t.before) return;
    // The text box's styles as they are now, while it is still the one selected.
    const box = el?.id === t.element && el.kind?.type === "text" ? (el.kind.text ?? {}) : t.box;
    const ops = saveTextOps(t.slide, t.element, t.before, textBoxFromPlain(box, t.value));
    t.before = t.value;
    return st.run("Typing", ops, { presentation: t.presentation });
  }

  // Another object or slide selected while typing here: the typing goes to the object it was
  // typed into, before the field shows the new one.
  $effect.pre(() => {
    const now = `${st.slideId}/${el?.id ?? ""}`;
    untrack(() => {
      if (typing && now !== `${typing.slide}/${typing.element}`) void save(true);
    });
  });

  function key(e: KeyboardEvent) {
    // The canvas's editing keys stay out of the field.
    e.stopPropagation();
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      void save();
    } else if (e.key === "Escape" && ta) {
      e.preventDefault();
      ta.value = text;
      base = null;
      typing = null;
      ta.blur();
    }
  }
</script>

<div class="tfield">
  <label for="ed-text">{label}</label>
  {#key el?.id}
    <textarea
      id="ed-text"
      bind:this={ta}
      rows="2"
      value={base ?? text}
      disabled={!el || !!el.locked || st.editingText === el.id}
      placeholder={st.selection.length > 1 ? "Select one text object to edit its text here" : "Select a text object to edit its text"}
      onfocus={() => {
        void st.finishText?.();
        base = text;
        const p = st.presentation?.id;
        if (el?.kind?.type === "text" && st.slideId && p) typing = { presentation: p, slide: st.slideId, element: el.id, box: el.kind.text ?? {}, before: text, value: text };
      }}
      oninput={(e) => {
        if (typing) typing.value = e.currentTarget.value;
      }}
      onblur={() => void save(true)}
      onkeydown={key}
      data-text-field
    ></textarea>
  {/key}
</div>

<style>
  .tfield {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 12px 10px;
    border-top: 1px solid var(--line);
    background: var(--surface-panel);
  }
  label {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-label);
  }
  textarea {
    resize: vertical;
    min-height: 44px;
    padding: 6px 8px;
    background: var(--surface-raised);
    border: 1px solid var(--line);
    border-radius: var(--r);
    color: var(--text);
    font: inherit;
    font-size: 13px;
    line-height: 1.45;
    user-select: text;
  }
  textarea:focus {
    outline: none;
    border-color: var(--accent-ring);
  }
  textarea:disabled {
    opacity: 0.55;
  }
</style>
