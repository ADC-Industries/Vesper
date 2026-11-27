// Pure logic behind the shared components (keyboard handling, sorting), unit tested.

export function clamp(v: number, min = -Infinity, max = Infinity): number {
  return Math.min(max, Math.max(min, v));
}

export interface SplitterKeyOptions {
  /** "col" sits between columns (←/→), "row" between stacked panels (↑/↓). */
  dir: "col" | "row";
  /** Pixels per arrow press (Shift: five times as much). */
  step?: number;
  /** Bounds; Home / End jump to them when given. */
  min?: number;
  max?: number;
  /** The splitter's direction factor (−1 when the panel grows the other way). */
  scale?: number;
}

/** A splitter's new value for a key press, or `null` when the key isn't the splitter's. */
export function splitterKey(key: string, shift: boolean, value: number, o: SplitterKeyOptions): number | null {
  const step = (o.step ?? 10) * (shift ? 5 : 1) * (o.scale ?? 1);
  const back = o.dir === "col" ? "ArrowLeft" : "ArrowUp";
  const fwd = o.dir === "col" ? "ArrowRight" : "ArrowDown";
  if (key === back) return clamp(value - step, o.min, o.max);
  if (key === fwd) return clamp(value + step, o.min, o.max);
  if (key === "Home" && o.min !== undefined) return o.min;
  if (key === "End" && o.max !== undefined) return o.max;
  return null;
}

export interface MenuEntry {
  disabled?: boolean;
  separator?: boolean;
}

/** Whether a menu entry can take the keyboard focus. */
export const focusable = (e: MenuEntry | undefined): boolean => !!e && !e.separator && !e.disabled;

/**
 * The menu entry that a key moves to (↓ / ↑ wrap, Home / End), skipping separators and
 * disabled entries. Returns `current` when nothing else can take the focus, `null` for other keys.
 */
export function menuMove(items: MenuEntry[], current: number, key: string): number | null {
  const n = items.length;
  if (!n) return null;
  const first = () => items.findIndex(focusable);
  const last = () => {
    for (let i = n - 1; i >= 0; i--) if (focusable(items[i])) return i;
    return -1;
  };
  const step = (dir: 1 | -1) => {
    for (let k = 1; k <= n; k++) {
      const i = (((current + dir * k) % n) + n) % n;
      if (focusable(items[i])) return i;
    }
    return current;
  };
  switch (key) {
    case "ArrowDown":
      return current < 0 ? first() : step(1);
    case "ArrowUp":
      return current < 0 ? last() : step(-1);
    case "Home":
      return first();
    case "End":
      return last();
    default:
      return null;
  }
}

/** The entry whose label starts with the typed letter, after the current one (type-ahead). */
export function menuTypeahead(labels: (string | null)[], items: MenuEntry[], current: number, ch: string): number | null {
  const c = ch.toLowerCase();
  const n = labels.length;
  for (let k = 1; k <= n; k++) {
    const i = (current + k) % n;
    if (focusable(items[i]) && labels[i]?.toLowerCase().startsWith(c)) return i;
  }
  return null;
}

export interface SegOption<V> {
  value: V;
  disabled?: boolean;
}

/**
 * The option a key selects in a segmented control or tab strip (← / → and ↑ / ↓ wrap,
 * Home / End), skipping disabled options; `null` for other keys.
 */
export function segmentedKey<V>(options: SegOption<V>[], value: V, key: string): V | null {
  const enabled = options.filter((o) => !o.disabled);
  if (!enabled.length) return null;
  const i = enabled.findIndex((o) => o.value === value);
  switch (key) {
    case "ArrowRight":
    case "ArrowDown":
      return enabled[(i + 1 + enabled.length) % enabled.length]!.value;
    case "ArrowLeft":
    case "ArrowUp":
      return enabled[(i - 1 + enabled.length * 2) % enabled.length]!.value;
    case "Home":
      return enabled[0]!.value;
    case "End":
      return enabled[enabled.length - 1]!.value;
    default:
      return null;
  }
}

export type SortDir = "asc" | "desc";
export interface SortState {
  key: string;
  dir: SortDir;
}

/** Clicking a column header: the same column flips its direction, another starts ascending. */
export function nextSort(cur: SortState | null, key: string): SortState {
  if (cur && cur.key === key) return { key, dir: cur.dir === "asc" ? "desc" : "asc" };
  return { key, dir: "asc" };
}

export type SortValue = string | number | boolean | Date | null | undefined;

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

/** Compares two cell values: empty values last (in both directions), text by locale, numbers numerically. */
export function compareValues(a: SortValue, b: SortValue): number {
  const ea = a === null || a === undefined || a === "";
  const eb = b === null || b === undefined || b === "";
  if (ea || eb) return ea === eb ? 0 : ea ? 1 : -1;
  if (a instanceof Date || b instanceof Date) return +(a as Date) - +(b as Date);
  if (typeof a === "number" && typeof b === "number") return a - b;
  if (typeof a === "boolean" && typeof b === "boolean") return Number(a) - Number(b);
  return collator.compare(String(a), String(b));
}

/** Rows sorted by one column (stable; empty values stay at the end either way). */
export function sortRows<T>(rows: readonly T[], value: (row: T) => SortValue, dir: SortDir): T[] {
  const sign = dir === "asc" ? 1 : -1;
  return rows
    .map((row, i) => ({ row, i, v: value(row) }))
    .sort((x, y) => {
      const ex = x.v === null || x.v === undefined || x.v === "";
      const ey = y.v === null || y.v === undefined || y.v === "";
      if (ex || ey) return ex === ey ? x.i - y.i : ex ? 1 : -1;
      return compareValues(x.v, y.v) * sign || x.i - y.i;
    })
    .map((x) => x.row);
}
