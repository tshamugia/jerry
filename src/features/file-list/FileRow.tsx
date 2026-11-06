// biome-ignore-all lint/a11y/useSemanticElements: a virtualized ARIA grid; rows are absolutely positioned divs, which table elements can't be.
// biome-ignore-all lint/a11y/useFocusableInteractive: the grid holds the keyboard focus and marks the focused row, as the WAI-ARIA grid pattern does.
import { memo } from "react";
import type { ColumnId } from "../../stores/columns";
import { Attribute } from "../../stores/entries";
import { RenameField } from "../rename/RenameField";
import { NUMERIC, WIDTH_CLASS } from "./cells";
import { dateText, extensionOf, nameText, sizeCell, typeName } from "./format";

export interface FileRowProps {
  id: number;
  /** The list's prefix for row element ids, which `aria-activedescendant` points at. */
  idPrefix: string;
  /** Position among the rows, from 0. */
  index: number;
  /** Offset from the first row, in CSS px. */
  top: number;
  /** The columns that show, in order. */
  columns: readonly ColumnId[];
  name: string;
  size: number;
  modified: number;
  created: number;
  attributes: number;
  /** `icon://` URL; none while the entry's own icon needs a listing id that hasn't come yet. */
  icon: string | undefined;
  dateFormat: string;
  showExtensions: boolean;
  selected: boolean;
  /** Draws the keyboard focus ring while the list has keyboard focus. */
  focused: boolean;
  /** Selected rows are accent-tinted in the active pane, gray in the other one. */
  activePane: boolean;
  /** The name shows as the inline rename field (F2). */
  editing: boolean;
  /** Jerry cut it (Ctrl+X) and the clipboard still holds it: drawn half-transparent. */
  cut: boolean;
}

/**
 * One row of the Details view: fixed height (the density's), inset 4 px with a 4 px radius as
 * the design draws it. Memoized: a row repaints only when its own entry, state or columns change.
 * Clicks are handled by the list, which finds the row by `data-id`; the icon and the name are
 * the item itself (`data-item`), the rest of the row is where a marquee can start.
 */
export const FileRow = memo(function FileRow(props: FileRowProps) {
  const {
    id,
    idPrefix,
    index,
    top,
    columns,
    attributes,
    selected,
    focused,
    activePane,
    editing,
    cut,
  } = props;
  const folder = (attributes & Attribute.directory) !== 0;
  const background = selected ? (activePane ? "bg-sel" : "bg-sel-inactive") : "hover:bg-hover";
  // The cells, not the row: the selection's tint stays as it is.
  const faded = cut && !editing ? "opacity-50" : "";
  const focus = focused
    ? "group-focus-visible:-outline-offset-2 group-focus-visible:outline-2 group-focus-visible:outline-focus"
    : "";
  return (
    <div
      id={`${idPrefix}${id}`}
      role="row"
      aria-rowindex={index + 2}
      aria-selected={selected}
      data-id={id}
      // The rename hint can reach over the rows below.
      className={`absolute top-0 right-1 left-1 flex h-row items-center rounded-ctrl px-2 ${background} ${focus} ${editing ? "z-1" : ""}`}
      style={{ transform: `translateY(${top}px)` }}
    >
      {columns.map((column, at) =>
        column === "name" ? (
          <NameCell key={column} {...props} folder={folder} first={at === 0} faded={faded} />
        ) : (
          <div
            key={column}
            role="gridcell"
            className={`${WIDTH_CLASS[column]} truncate text-fg-2 text-small ${faded} ${at === 0 ? "" : "pl-2"} ${column === "size" ? "text-right" : ""} ${NUMERIC.has(column) ? "tabular-nums" : ""}`}
          >
            {cellText(column, props, folder)}
          </div>
        ),
      )}
    </div>
  );
});

function NameCell({
  name,
  icon,
  attributes,
  showExtensions,
  editing,
  folder,
  first,
  faded,
}: FileRowProps & { folder: boolean; first: boolean; faded: string }) {
  // Hidden and system entries show only with "Show hidden" on, dimmed.
  const dimmed = (attributes & (Attribute.hidden | Attribute.system)) !== 0 ? "opacity-60" : "";
  return (
    <div
      role="gridcell"
      // The rename field's hint shows past the cell.
      className={`${WIDTH_CLASS.name} flex items-center gap-2 ${editing ? "" : "overflow-hidden"} ${faded} ${first ? "" : "pl-2"}`}
    >
      {icon ? (
        <img
          src={icon}
          alt=""
          draggable={false}
          data-item
          className={`size-4 shrink-0 ${dimmed}`}
        />
      ) : (
        <span data-item className="size-4 shrink-0" />
      )}
      {editing ? (
        <RenameField folder={folder} />
      ) : (
        // `data-name`: a slow second click here renames. `elementtiming`: when rows were first
        // painted, for the cold-start budget (Element Timing, Tech PRD «წარმადობა»).
        <span data-item data-name elementtiming="row" className={`truncate ${dimmed}`}>
          {nameText(name, folder, showExtensions)}
        </span>
      )}
    </div>
  );
}

function cellText(column: ColumnId, row: FileRowProps, folder: boolean): string {
  switch (column) {
    case "modified":
      return dateText(row.modified, row.dateFormat);
    case "created":
      return dateText(row.created, row.dateFormat);
    case "type":
      return typeName(row.name, folder);
    case "size":
      return sizeCell(row.size, folder);
    case "extension":
      return extensionOf(row.name, folder);
    case "name":
      return row.name;
  }
}
