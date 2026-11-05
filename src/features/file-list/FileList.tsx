import { useVirtualizer } from "@tanstack/react-virtual";
import {
  type KeyboardEvent,
  type MouseEvent,
  memo,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { keyTarget } from "../../commands/dispatch";
import { lookup } from "../../commands/keymap";
import { isEnabled } from "../../commands/registry";
import { type Density, entryIconUrl, IconKey } from "../../ipc";
import { shownColumns } from "../../stores/columns";
import { type Folder, useFolder } from "../../stores/folder";
import { useKeymap } from "../../stores/keymap";
import { useSetting } from "../../stores/settings";
import { type PaneSide, useWindow } from "../../stores/window";
import { errorText, strings } from "../../strings";
import { selectionOf } from "../selection/commands";
import {
  clickRow,
  indexOf,
  isMoveKey,
  moveTo,
  NO_MODIFIERS,
  type Page,
  rowForKey,
} from "../selection/model";
import { findByPrefix, TypeAhead } from "../selection/typeahead";
import { usePointerSelection } from "../selection/usePointerSelection";
import { ColumnHeader } from "./ColumnHeader";
import { columnWidths } from "./cells";
import { FileRow } from "./FileRow";
import { useShownRows } from "./rows";

/** Rows rendered past each edge of the view. */
const OVERSCAN = 8;
/** Icons are 16 px (design: «Icons»). */
const ICON_PX = 16;
/** A folder that takes longer than this to read shows the loading bar. */
const LOADING_DELAY_MS = 200;
/** Comfortable rows, if the density token can't be read (tests). */
const FALLBACK_ROW_PX = 30;

/**
 * A pane's file list in the Details view: the column header and a virtualized list of rows with
 * the density's fixed height (30 / 24 px), so 100 000 entries scroll at 60 fps. Rows show as the
 * folder's batches arrive and follow every change in it. Selection works as in Explorer, with
 * the mouse (`usePointerSelection`) and, while the list has the focus, with the arrows, Home,
 * End, PageUp and PageDown (Shift selects a range, Ctrl moves only the focus) and by typing the
 * first letters of a name. A double click (or a single one, by the "click to open" setting)
 * opens a folder.
 */
export const FileList = memo(function FileList({
  folder,
  pane,
}: {
  folder: Folder;
  pane: PaneSide;
}) {
  const status = useFolder(folder, (state) => state.status);
  const error = useFolder(folder, (state) => state.error);
  const entries = useFolder(folder, (state) => state.entries);
  const filter = useFolder(folder, (state) => state.filter);
  const listing = useFolder(folder, (state) => state.listing);
  const sort = useFolder(folder, (state) => state.sort);
  const layout = useFolder(folder, (state) => state.columns);
  const selected = useFolder(folder, (state) => state.selected);
  const focused = useFolder(folder, (state) => state.focused);
  const activePane = useWindow((state) => state.activePane === pane);
  const showExtensions = useSetting("showExtensions") ?? true;
  const foldersFirst = useSetting("foldersFirst") ?? true;
  const dateFormat = useSetting("dateFormat") ?? "DD.MM.YYYY";
  const clickToOpen = useSetting("clickToOpen") ?? "double";
  const density = useSetting("density") ?? "comfortable";

  useEffect(() => folder.setFoldersFirst(foldersFirst), [folder, foldersFirst]);

  // Selection (#30) works on these rows, so it never reaches entries the filter hides.
  const { rows, total } = useShownRows(folder);
  // Rows hidden again or filtered out leave the selection.
  useEffect(() => folder.keepShown(rows), [folder, rows]);

  const columns = useMemo(() => shownColumns(layout), [layout]);
  // The same array while only widths change, so dragging an edge repaints no row.
  // biome-ignore lint/correctness/useExhaustiveDependencies: compared by content.
  const stableColumns = useMemo(() => columns, [columns.join()]);
  const rowHeight = useRowHeight(density);

  const scroller = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const [margin, setMargin] = useState(0);
  useLayoutEffect(() => setMargin(body.current?.offsetTop ?? 0), []);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => rowHeight,
    overscan: OVERSCAN,
    scrollMargin: margin,
  });
  // biome-ignore lint/correctness/useExhaustiveDependencies: measures again for the new row height.
  useEffect(() => virtualizer.measure(), [virtualizer, rowHeight]);
  // Another folder starts at the top.
  // biome-ignore lint/correctness/useExhaustiveDependencies: runs for every folder opened.
  useEffect(() => {
    if (scroller.current) {
      scroller.current.scrollTop = 0;
    }
  }, [entries]);
  const slow = useDelayed(status === "loading", LOADING_DELAY_MS);

  const pointer = usePointerSelection({ folder, rows, rowHeight, scroller, body, margin });
  const typeAhead = useMemo(() => new TypeAhead(), []);
  const rowIds = useId();

  /** The rows wholly in view: under the header, above the bottom edge. */
  const page = (): Page => {
    const view = scroller.current;
    const top = view?.scrollTop ?? 0;
    const height = (view?.clientHeight ?? 0) - margin;
    const first = Math.ceil(top / rowHeight);
    return { first, last: Math.max(first, Math.floor((top + height) / rowHeight) - 1) };
  };
  /** Scrolls row `index` into view, as little as it takes. */
  const reveal = (index: number) => {
    const view = scroller.current;
    if (!view) {
      return;
    }
    const top = index * rowHeight;
    const height = view.clientHeight - margin;
    if (top < view.scrollTop) {
      view.scrollTop = top;
    } else if (top + rowHeight > view.scrollTop + height) {
      view.scrollTop = top + rowHeight - height;
    }
  };

  const keyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    // The list's own keys work while it has the focus, not a header button in it.
    if (
      event.defaultPrevented ||
      event.nativeEvent.isComposing ||
      event.target !== scroller.current
    ) {
      return;
    }
    const selection = selectionOf(folder.store.getState());
    const at = indexOf(rows, selection.focused);
    // Navigation keys by `key`: they are the same on every layout, and the numpad's arrows
    // (NumLock off) come as `key` "ArrowDown" with `code` "Numpad2".
    if (isMoveKey(event.key) && !event.altKey && !event.metaKey) {
      event.preventDefault();
      typeAhead.reset();
      const to = rowForKey(event.key, at, rows.length, page());
      if (to !== undefined) {
        const modifiers = { ctrl: event.ctrlKey, shift: event.shiftKey };
        folder.setSelection(moveTo(selection, rows, to, modifiers));
        reveal(to);
      }
      return;
    }
    // Type-to-select takes typed characters, by `key`: on the Georgian layout they are Georgian.
    const now = performance.now();
    const typed = [...event.key].length === 1 && !event.ctrlKey && !event.altKey && !event.metaKey;
    const space = event.key === " ";
    if (!typed || (space ? !typeAhead.active(now) : isShortcut(event))) {
      return;
    }
    event.preventDefault();
    const { prefix, after } = typeAhead.type(event.key, now);
    const start = at === undefined ? 0 : after ? at + 1 : at;
    const found = findByPrefix(rows, entries.names, prefix, rows.length ? start % rows.length : 0);
    if (found !== undefined) {
      folder.setSelection(clickRow(selection, rows, found, NO_MODIFIERS));
      reveal(found);
    }
  };

  const rowAt = (event: MouseEvent) => {
    const row = (event.target as Element).closest<HTMLElement>("[data-id]");
    return row ? Number(row.dataset.id) : undefined;
  };
  const click = (event: MouseEvent) => {
    const id = rowAt(event);
    if (pointer.wasDrag(event) || id === undefined || event.ctrlKey || event.shiftKey) {
      return;
    }
    if (clickToOpen === "single") {
      folder.openEntry(id);
    }
  };
  const doubleClick = (event: MouseEvent) => {
    const id = rowAt(event);
    if (id !== undefined && clickToOpen === "double") {
      folder.openEntry(id);
    }
  };

  const name = pane === "left" ? strings.frame.leftPane : strings.frame.rightPane;
  const message =
    status === "failed" && error ? (
      <>
        <p>{errorText(error)}</p>
        <button
          type="button"
          onClick={() => void folder.retry()}
          className="pointer-events-auto h-8 rounded-ctrl border border-ctrl-border bg-ctrl px-3 text-fg hover:bg-hover"
        >
          {strings.fileList.retry}
        </button>
      </>
    ) : rows.length > 0 ? null : filter !== "" && total > 0 ? (
      strings.fileList.noMatch
    ) : status === "ready" ? (
      strings.fileList.empty
    ) : slow ? (
      strings.fileList.loading
    ) : null;

  const items = virtualizer.getVirtualItems();
  const focusShown = items.some((item) => rows[item.index] === focused);
  const { rect } = pointer;

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      {/* biome-ignore lint/a11y/useSemanticElements: a virtualized grid; see FileRow. */}
      <div
        ref={scroller}
        role="grid"
        aria-label={strings.fileList.files(name)}
        aria-rowcount={rows.length + 1}
        aria-colcount={stableColumns.length}
        aria-multiselectable
        aria-activedescendant={focusShown ? `${rowIds}${focused}` : undefined}
        aria-busy={status === "loading"}
        tabIndex={0}
        data-file-list={pane}
        className={`group relative min-h-0 flex-1 overflow-auto ${focused === undefined ? "" : "focus-visible:outline-none"}`}
        style={columnWidths(layout)}
        onKeyDown={keyDown}
        onClick={click}
        onDoubleClick={doubleClick}
        {...pointer.handlers}
      >
        <div className="flex min-h-full w-max min-w-full flex-col">
          <ColumnHeader
            folder={folder}
            layout={layout}
            columns={stableColumns}
            sort={sort}
            loading={slow}
          />
          <div
            ref={body}
            className="relative mt-1 shrink-0"
            style={{ height: virtualizer.getTotalSize() }}
          >
            {items.map((item) => {
              const id = rows[item.index] ?? 0;
              const key = entries.iconKeys[id] ?? IconKey.file;
              const modified = entries.modified[id] ?? 0;
              return (
                <FileRow
                  key={id}
                  id={id}
                  idPrefix={rowIds}
                  index={item.index}
                  top={item.start - margin}
                  columns={stableColumns}
                  name={entries.names[id] ?? ""}
                  size={entries.sizes[id] ?? 0}
                  modified={modified}
                  created={entries.created[id] ?? 0}
                  attributes={entries.attributes[id] ?? 0}
                  icon={
                    key === IconKey.own && listing === undefined
                      ? undefined
                      : entryIconUrl({ listing: listing ?? 0, id, key, modified }, ICON_PX)
                  }
                  dateFormat={dateFormat}
                  showExtensions={showExtensions}
                  selected={selected.has(id)}
                  focused={focused === id}
                  activePane={activePane}
                />
              );
            })}
            {rect && (
              <div
                data-marquee
                className="pointer-events-none absolute border border-accent bg-marquee"
                style={rect}
              />
            )}
          </div>
        </div>
      </div>
      {message && (
        // Over the list, under its header, as wide as the pane even when the columns are wider.
        <div className="pointer-events-none absolute inset-x-0 top-col-header flex flex-col items-center gap-3 px-4 pt-6 text-center text-fg-2">
          {message}
        </div>
      )}
    </div>
  );
});

/** Whether a key press runs a command where it lands ("?" opens the shortcut table). */
function isShortcut(event: KeyboardEvent): boolean {
  const command = lookup(useKeymap.getState().keymap, event.nativeEvent, keyTarget(event.target));
  return command !== undefined && isEnabled(command);
}

/** The density's row height in CSS px, from the design tokens (`--jerry-row-h`). */
function useRowHeight(density: Density): number {
  // biome-ignore lint/correctness/useExhaustiveDependencies: the token follows the density.
  return useMemo(() => {
    const token = getComputedStyle(document.documentElement).getPropertyValue("--jerry-row-h");
    return Number.parseFloat(token) || FALLBACK_ROW_PX;
  }, [density]);
}

/** `on`, once it has stayed on for `ms`; off at once. */
function useDelayed(on: boolean, ms: number): boolean {
  const [late, setLate] = useState(false);
  useEffect(() => {
    if (!on) {
      setLate(false);
      return;
    }
    const timer = setTimeout(() => setLate(true), ms);
    return () => clearTimeout(timer);
  }, [on, ms]);
  return on && late;
}
