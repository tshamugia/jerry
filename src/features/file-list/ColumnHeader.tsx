// biome-ignore-all lint/a11y/useSemanticElements: a virtualized ARIA grid; rows are absolutely positioned divs, which table elements can't be.
// biome-ignore-all lint/a11y/useFocusableInteractive: the grid holds the keyboard focus and marks the focused row, as the WAI-ARIA grid pattern does.
import { type KeyboardEvent, memo, type PointerEvent, useRef, useState } from "react";
import type { Sort } from "../../ipc";
import {
  type ColumnId,
  type ColumnLayout,
  canMoveColumn,
  DEFAULT_COLUMNS,
  moveColumn,
  resizeColumn,
  toggleColumn,
} from "../../stores/columns";
import type { Folder } from "../../stores/folder";
import { strings } from "../../strings";
import { ContextMenu, MenuCheckbox, MenuItem, MenuSeparator } from "../../ui/ContextMenu";
import { SortIcon } from "../../ui/icons";
import { WIDTH_CLASS } from "./cells";

/** How far an arrow key moves a column's edge, in CSS px. */
const RESIZE_STEP_PX = 8;

const titles = strings.fileList.columns;

interface Props {
  folder: Folder;
  layout: ColumnLayout;
  /** The columns that show, in order. */
  columns: readonly ColumnId[];
  sort: Sort;
  /** Shows the loading bar along the header's bottom edge. */
  loading: boolean;
}

/**
 * The Details view's 28 px column header. Clicking a column sorts by it, clicking it again flips
 * the direction; dragging a column's edge resizes it; the right-click menu shows and hides
 * columns, moves them and resets them. It sticks to the top while the rows scroll under it.
 */
export const ColumnHeader = memo(function ColumnHeader({
  folder,
  layout,
  columns,
  sort,
  loading,
}: Props) {
  const [menuColumn, setMenuColumn] = useState<ColumnId>();
  const nameAt = columns.indexOf("name");
  const change = (next: (current: ColumnLayout) => ColumnLayout) =>
    folder.setColumns(next(folder.store.getState().columns));
  const move = (by: -1 | 1) => {
    if (menuColumn) {
      change((current) => moveColumn(current, menuColumn, by));
    }
  };

  const menu = (
    <>
      {DEFAULT_COLUMNS.order.map((column) => (
        <MenuCheckbox
          key={column}
          checked={!layout.hidden.includes(column)}
          disabled={column === "name"}
          onCheckedChange={() => change((current) => toggleColumn(current, column))}
        >
          {titles[column]}
        </MenuCheckbox>
      ))}
      <MenuSeparator />
      <MenuItem
        disabled={!menuColumn || !canMoveColumn(layout, menuColumn, -1)}
        onSelect={() => move(-1)}
      >
        {strings.fileList.moveLeft}
      </MenuItem>
      <MenuItem
        disabled={!menuColumn || !canMoveColumn(layout, menuColumn, 1)}
        onSelect={() => move(1)}
      >
        {strings.fileList.moveRight}
      </MenuItem>
      <MenuSeparator />
      <MenuItem onSelect={() => folder.setColumns(DEFAULT_COLUMNS)}>
        {strings.fileList.resetColumns}
      </MenuItem>
    </>
  );

  return (
    <ContextMenu label={strings.fileList.columnsMenu} menu={menu}>
      <div
        role="row"
        aria-rowindex={1}
        className="sticky top-0 z-10 flex h-col-header shrink-0 border-stroke border-b bg-pane px-3 text-fg-2 text-small"
        // The menu's Move left and Move right act on the column it opened on.
        onContextMenu={(event) => setMenuColumn(columnAt(event.target))}
      >
        {columns.map((column, at) => {
          const sorted = sort.column === column;
          return (
            <div
              key={column}
              role="columnheader"
              aria-sort={sorted ? (sort.descending ? "descending" : "ascending") : "none"}
              data-column={column}
              className={`relative flex items-center ${WIDTH_CLASS[column]} ${at === 0 ? "" : "pl-2"}`}
            >
              {at > 0 && <span className="absolute inset-y-1.5 left-0 w-px bg-stroke" />}
              <button
                type="button"
                onClick={() => folder.sortBy(column)}
                className={`flex h-full min-w-0 flex-1 items-center gap-1.5 hover:text-fg ${column === "size" ? "justify-end" : ""}`}
              >
                <span className="truncate">{titles[column]}</span>
                {sorted && <SortIcon descending={sort.descending} />}
              </button>
              {column !== "name" && (
                <ResizeHandle
                  folder={folder}
                  column={column}
                  // Name takes what the others leave, so each column's edge on Name's side moves.
                  edge={at > nameAt ? "left" : "right"}
                  width={layout.widths[column]}
                />
              )}
            </div>
          );
        })}
        {loading && (
          <div className="absolute inset-x-0 -bottom-px h-0.5 overflow-hidden">
            <div className="h-full w-1/3 animate-loading bg-accent motion-reduce:animate-none" />
          </div>
        )}
      </div>
    </ContextMenu>
  );
});

/** The column of the header cell `target` is in. */
function columnAt(target: EventTarget): ColumnId | undefined {
  const cell = target instanceof Element ? target.closest<HTMLElement>("[data-column]") : null;
  return cell?.dataset.column as ColumnId | undefined;
}

/**
 * The 9 px handle over a column's edge: dragging it, or the arrow keys while it has the focus,
 * resize the column. A handle on the column's left edge grows it to the left.
 */
function ResizeHandle({
  folder,
  column,
  edge,
  width,
}: {
  folder: Folder;
  column: ColumnId;
  edge: "left" | "right";
  /** The dragged width; undefined while the column has the design's. */
  width: number | undefined;
}) {
  const drag = useRef<{ x: number; width: number }>(undefined);
  const grow = edge === "left" ? -1 : 1;
  const resize = (px: number) =>
    folder.setColumns(resizeColumn(folder.store.getState().columns, column, px));
  const measured = (handle: HTMLElement) =>
    width ?? handle.parentElement?.getBoundingClientRect().width ?? 0;

  const end = (event: PointerEvent<HTMLDivElement>) => {
    if (drag.current) {
      drag.current = undefined;
      event.currentTarget.releasePointerCapture?.(event.pointerId);
    }
  };
  const keys = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = { ArrowLeft: -RESIZE_STEP_PX, ArrowRight: RESIZE_STEP_PX }[event.key];
    if (step !== undefined) {
      event.preventDefault();
      resize(measured(event.currentTarget) + grow * step);
    }
  };

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={strings.fileList.resizeColumn(titles[column])}
      aria-valuenow={width}
      tabIndex={0}
      className={`absolute inset-y-0 z-10 w-2.25 cursor-col-resize ${edge === "left" ? "-left-1" : "-right-1"}`}
      onPointerDown={(event) => {
        if (event.button !== 0) {
          return;
        }
        event.preventDefault();
        drag.current = { x: event.clientX, width: measured(event.currentTarget) };
        event.currentTarget.setPointerCapture?.(event.pointerId);
      }}
      onPointerMove={(event) => {
        if (drag.current) {
          resize(drag.current.width + grow * (event.clientX - drag.current.x));
        }
      }}
      onPointerUp={end}
      onPointerCancel={end}
      onKeyDown={keys}
    />
  );
}
