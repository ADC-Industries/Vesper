<script lang="ts" generics="T">
  // A sortable table with a sticky header (the canvas's `.st-th` / `.st-td`). Columns sort by
  // `sortValue` (or the cell's `text`); `cell` renders custom cells. Rows can be selected by
  // click or Enter.
  import type { Snippet } from "svelte";
  import ArrowDown from "lucide-svelte/icons/arrow-down";
  import ArrowUp from "lucide-svelte/icons/arrow-up";
  import { nextSort, sortRows, type SortState } from "./logic";
  import type { Column } from "./types";


  let {
    columns,
    rows,
    rowKey,
    label,
    sort = $bindable(null),
    selected = null,
    onselect,
    cell,
    empty = "Nothing here yet.",
  }: {
    columns: Column<T>[];
    rows: T[];
    rowKey: (row: T) => string;
    label: string;
    sort?: SortState | null;
    selected?: string | null;
    onselect?: (row: T) => void;
    cell?: Snippet<[T, Column<T>]>;
    empty?: string;
  } = $props();

  let sorted = $derived.by(() => {
    const s = sort;
    const col = s && columns.find((c) => c.key === s.key);
    if (!s || !col) return rows;
    const value = col.sortValue ?? col.text ?? (() => null);
    return sortRows(rows, value, s.dir);
  });

  function ariaSort(c: Column<T>): "ascending" | "descending" | "none" | undefined {
    if (c.sortable === false) return undefined;
    if (sort?.key !== c.key) return "none";
    return sort.dir === "asc" ? "ascending" : "descending";
  }
</script>

<div class="ui-table-wrap">
  <table class="ui-table" aria-label={label}>
    <thead>
      <tr>
        {#each columns as c (c.key)}
          <th scope="col" aria-sort={ariaSort(c)} style:width={c.width} style:text-align={c.align}>
            {#if c.sortable === false}
              {c.label}
            {:else}
              <button type="button" class="sort" onclick={() => (sort = nextSort(sort, c.key))}>
                {c.label}
                {#if sort?.key === c.key}
                  {#if sort.dir === "asc"}<ArrowUp size={12} strokeWidth={2} aria-hidden="true" />{:else}<ArrowDown
                      size={12}
                      strokeWidth={2}
                      aria-hidden="true"
                    />{/if}
                {/if}
              </button>
            {/if}
          </th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each sorted as row (rowKey(row))}
        {@const key = rowKey(row)}
        <tr
          class:selected={selected === key}
          class:clickable={!!onselect}
          aria-selected={onselect ? selected === key : undefined}
          tabindex={onselect ? 0 : undefined}
          onclick={() => onselect?.(row)}
          onkeydown={(e) => {
            if (onselect && (e.key === "Enter" || e.key === " ")) {
              e.preventDefault();
              onselect(row);
            }
          }}
        >
          {#each columns as c (c.key)}
            <td style:text-align={c.align}>
              {#if cell}{@render cell(row, c)}{:else}{c.text?.(row) ?? ""}{/if}
            </td>
          {/each}
        </tr>
      {:else}
        <tr><td class="empty" colspan={columns.length}>{empty}</td></tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .ui-table-wrap {
    min-height: 0;
    overflow: auto;
  }
  .ui-table {
    width: 100%;
    border-collapse: separate;
    border-spacing: 0;
    font-size: var(--fs);
  }
  th {
    position: sticky;
    top: 0;
    z-index: 1;
    padding: 8px 10px;
    background: var(--surface-panel);
    border-bottom: 1px solid var(--line);
    text-align: left;
    font-size: var(--fs-xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-label);
    white-space: nowrap;
  }
  .sort {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    letter-spacing: inherit;
    text-transform: inherit;
  }
  .sort:hover {
    color: var(--text);
  }
  td {
    padding: 9px 10px;
    border-bottom: 1px solid var(--surface-raised);
    vertical-align: middle;
    color: var(--text);
  }
  tr.clickable {
    cursor: pointer;
  }
  tr.clickable:hover td {
    background: var(--surface-raised);
  }
  tr.selected td,
  tr.selected:hover td {
    background: var(--accent-sel);
  }
  tr:focus-visible {
    outline: 2px solid var(--accent-ring);
    outline-offset: -2px;
  }
  .empty {
    padding: 24px 10px;
    text-align: center;
    color: var(--text-meta);
  }
</style>
