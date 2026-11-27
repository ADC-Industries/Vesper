// Copy / Paste Style's clipboard: one per app (survives switching slides, presentations and
// between the Edit page and the Library's song Edit).

import type { TextStyleClip } from "./styleClip";

export const styleClipboard = $state<{ clip: TextStyleClip | null }>({ clip: null });
