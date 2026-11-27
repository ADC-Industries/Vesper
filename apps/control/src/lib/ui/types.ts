// Types shared by the components in lib/ui.

import type { SortValue } from "./logic";

/** A lucide-svelte icon component (`import Plus from "lucide-svelte/icons/plus"`). */
export type IconComponent = typeof import("lucide-svelte/icons/x").default;

/** Icons are drawn at 16 px with a 1.8 stroke everywhere (the canvas's icon style). */
export const ICON_SIZE = 16;
export const ICON_STROKE = 1.8;

export type ButtonVariant = "primary" | "secondary" | "outline" | "danger" | "live" | "ghost";
export type ChipKind = "live" | "next" | "disabled" | "edited" | "test" | "inset" | "neutral";
export type Status = "ok" | "off" | "live" | "warn" | "error" | "info";

/** A DataTable column. */
export interface Column<R> {
  key: string;
  label: string;
  /** The cell's text (also the sort value unless `sortValue` is given). */
  text?: (row: R) => string;
  sortValue?: (row: R) => SortValue;
  sortable?: boolean;
  align?: "left" | "right" | "center";
  width?: string;
}

/** A Menu entry. */
export interface MenuItem {
  id: string;
  label?: string;
  shortcut?: string;
  icon?: IconComponent;
  /** A check mark (and `menuitemcheckbox`) when not undefined. */
  checked?: boolean;
  disabled?: boolean;
  /** Why it is disabled ("coming later"), shown after the label. */
  note?: string;
  danger?: boolean;
  separator?: boolean;
}
