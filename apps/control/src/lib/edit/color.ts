// Colours for the inspector's colour inputs (`<input type=color>` speaks #rrggbb).

import type { Color } from "$protocol";

export function toHex(c: Color | undefined | null): string {
  if (!c) return "#000000";
  const h = (v: number) =>
    Math.round(Math.max(0, Math.min(1, v)) * 255)
      .toString(16)
      .padStart(2, "0");
  return `#${h(c.r)}${h(c.g)}${h(c.b)}`;
}

/** A colour from #rrggbb, keeping `alpha` (omitted when opaque, as the model stores it). */
export function fromHex(hex: string, alpha = 1): Color {
  const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex.trim());
  const v = (i: number) => (m ? Math.round((parseInt(m[i]!, 16) / 255) * 10000) / 10000 : 0);
  const c: Color = { r: v(1), g: v(2), b: v(3) };
  if (alpha < 1) c.a = alpha;
  return c;
}
