import type { CSSProperties } from "react";
import type { ColumnId, ColumnLayout } from "../../stores/columns";

/**
 * Each column's width, the same in the header and every row. Name takes what the others leave,
 * down to its minimum; the others read their width from a CSS variable on the list
 * (`columnWidths`), so dragging a column's edge repaints no row.
 */
export const WIDTH_CLASS: Record<ColumnId, string> = {
  name: "min-w-col-name flex-1",
  modified: "w-(--col-modified) shrink-0",
  type: "w-(--col-type) shrink-0",
  size: "w-(--col-size) shrink-0",
  created: "w-(--col-created) shrink-0",
  extension: "w-(--col-extension) shrink-0",
  location: "w-(--col-location) shrink-0",
};

/** The design's widths (tokens), until the user drags a column. */
const DEFAULT_WIDTHS: Record<Exclude<ColumnId, "name">, string> = {
  modified: "var(--jerry-col-date-w)",
  type: "var(--jerry-col-type-w)",
  size: "var(--jerry-col-size-w)",
  created: "var(--jerry-col-date-w)",
  extension: "var(--jerry-col-ext-w)",
  location: "var(--jerry-col-location-w)",
};

/** The width variables `WIDTH_CLASS` reads, for the list's `style`. */
export function columnWidths(layout: ColumnLayout): CSSProperties {
  const style: Record<string, string> = {};
  for (const [column, fallback] of Object.entries(DEFAULT_WIDTHS)) {
    const dragged = layout.widths[column as ColumnId];
    style[`--col-${column}`] = dragged === undefined ? fallback : `${dragged}px`;
  }
  return style as CSSProperties;
}

/** Columns whose values are numbers: dates and sizes, in tabular figures. */
export const NUMERIC: ReadonlySet<ColumnId> = new Set(["modified", "created", "size"]);
