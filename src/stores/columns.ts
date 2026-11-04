import type { SortColumn } from "../ipc";

/** A Details view column; each one sorts by its own `SortColumn`. */
export type ColumnId = SortColumn;

/** Narrowest a column can be dragged, in CSS px. */
export const MIN_COLUMN_PX = 40;

/**
 * The Details view's columns: their order, which ones show and the widths the user dragged.
 * Name always shows and takes the width the others leave; the others have the design's widths
 * (tokens) until dragged.
 */
export interface ColumnLayout {
  /** Every column, in display order. */
  order: readonly ColumnId[];
  /** The columns that don't show. Never Name. */
  hidden: readonly ColumnId[];
  /** Widths the user dragged, in CSS px. */
  widths: Readonly<Partial<Record<ColumnId, number>>>;
}

/** The PRD's Details view: Name, Date modified, Type, Size; Date created and Extension on demand. */
export const DEFAULT_COLUMNS: ColumnLayout = {
  order: ["name", "modified", "type", "size", "created", "extension"],
  hidden: ["created", "extension"],
  widths: {},
};

/** The columns that show, in display order. */
export function shownColumns(layout: ColumnLayout): ColumnId[] {
  return layout.order.filter((id) => !layout.hidden.includes(id));
}

/** Shows `id` if it is hidden, hides it otherwise. Name can't hide. */
export function toggleColumn(layout: ColumnLayout, id: ColumnId): ColumnLayout {
  if (id === "name") {
    return layout;
  }
  const hidden = layout.hidden.includes(id)
    ? layout.hidden.filter((other) => other !== id)
    : [...layout.hidden, id];
  return { ...layout, hidden };
}

/**
 * Swaps `id` with the shown column next to it, to the left (`-1`) or right (`1`). Returns the
 * layout as it was when there is no such column.
 */
export function moveColumn(layout: ColumnLayout, id: ColumnId, by: -1 | 1): ColumnLayout {
  const shown = shownColumns(layout);
  const neighbour = shown[shown.indexOf(id) + by];
  if (!shown.includes(id) || neighbour === undefined) {
    return layout;
  }
  const order = layout.order.map((other) =>
    other === id ? neighbour : other === neighbour ? id : other,
  );
  return { ...layout, order };
}

/** Whether `id` can move one shown column to the left (`-1`) or right (`1`). */
export function canMoveColumn(layout: ColumnLayout, id: ColumnId, by: -1 | 1): boolean {
  return moveColumn(layout, id, by) !== layout;
}

/** Sets a column's width, no narrower than `MIN_COLUMN_PX`. Name takes what is left. */
export function resizeColumn(layout: ColumnLayout, id: ColumnId, width: number): ColumnLayout {
  if (id === "name") {
    return layout;
  }
  const px = Math.max(MIN_COLUMN_PX, Math.round(width));
  return { ...layout, widths: { ...layout.widths, [id]: px } };
}
