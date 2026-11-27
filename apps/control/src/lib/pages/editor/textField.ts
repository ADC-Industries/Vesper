// The text field under the canvas: the selected text object's text as plain lines. Saving
// rebuilds the text box so styles stay: an unchanged line keeps its runs, a changed one takes
// its paragraph's first run's style (a new line, the line before's).

import type { TextBox } from "$protocol";
import { editedFromTextBox, rebuildTextBox, type EditedParagraph } from "../../edit/text";

export function textBoxFromPlain(tb: TextBox, text: string): TextBox {
  const orig = editedFromTextBox(tb);
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  const edited: EditedParagraph[] = lines.map((line, i) => {
    const o = orig[i];
    if (o && o.pieces.map((p) => p.text).join("") === line) return o;
    const hasRun = !!tb.paragraphs?.[i]?.runs?.length;
    return { from: o ? i : null, pieces: [{ text: line, run: hasRun ? [i, 0] : null }] };
  });
  return rebuildTextBox(tb, edited);
}
