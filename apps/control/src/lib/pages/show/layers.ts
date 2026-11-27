// What the Show page's layer buttons do: a lit button clears its layer; an empty one whose last
// cleared content the server keeps (the state's `restorableLayers`) brings it back
// (`restoreLayer`); otherwise nothing. The Slide button also covers the announcements layer.

import type { Command, Layer } from "$protocol";
import type { LayerActivity } from "../../model";

export type LayerButtonId = "media" | "slide" | "props" | "messages" | "audio";

export type LayerAction = { kind: "clear" | "restore" | "none"; commands: Command[] };

/** The layers a button stands for. */
function layersOf(id: LayerButtonId): Layer[] {
  return id === "slide" ? ["slide", "announcements"] : [id];
}

export function layerAction(id: LayerButtonId, activity: Pick<LayerActivity, LayerButtonId | "announcements">, restorable: readonly Layer[]): LayerAction {
  const layers = layersOf(id);
  const on = layers.filter((l) => activity[l as keyof typeof activity]);
  if (on.length) return { kind: "clear", commands: on.map((layer) => ({ type: "clearLayer", layer })) };
  const back = layers.filter((l) => restorable.includes(l));
  if (back.length) return { kind: "restore", commands: back.map((layer) => ({ type: "restoreLayer", layer })) };
  return { kind: "none", commands: [] };
}
