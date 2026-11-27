<script lang="ts">
  // Copy Style / Paste Style (Text tab): the text box's look onto other text objects, as one
  // undo step for all the selected ones.
  import type { EditState } from "../../edit/editState.svelte";
  import Button from "../../ui/Button.svelte";
  import { styleClipboard } from "./clipboard.svelte";
  import { copyTextStyle, pasteTextStyleOps } from "./styleClip";

  let { st }: { st: EditState } = $props();

  let texts = $derived(st.selected.filter((e) => e.kind?.type === "text"));
  let copied = $state(false);

  function copy() {
    const el = texts[0];
    if (!el) return;
    styleClipboard.clip = copyTextStyle(el);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }

  async function paste() {
    const clip = styleClipboard.clip;
    const slide = st.slideId;
    if (!clip || !slide) return;
    await st.finishText?.();
    void st.run(
      "Paste Style",
      texts.filter((e) => !e.locked).flatMap((e) => pasteTextStyleOps(slide, e, clip)),
    );
  }
</script>

<section class="sec">
  <div class="btn2" data-style-clip>
    <Button size="sm" variant="outline" onclick={copy} disabled={!texts.length} title="Copy this text box's look: font, size, colour, alignment, outline, shadow">{copied ? "Copied" : "Copy Style"}</Button>
    <Button size="sm" variant="outline" onclick={() => void paste()} disabled={!styleClipboard.clip || !texts.some((e) => !e.locked)} title={styleClipboard.clip ? "Give the selected text the copied look" : "Copy a text box's style first"}>Paste Style</Button>
  </div>
</section>
