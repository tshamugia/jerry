import { useVirtualizer } from "@tanstack/react-virtual";
import {
  type KeyboardEvent,
  type MouseEvent,
  memo,
  type PointerEvent,
  type RefObject,
  useCallback,
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
import type { Tab } from "../../stores/tabs";
import { type PaneSide, useWindow } from "../../stores/window";
import { errorText, strings } from "../../strings";
import { ContextMenu, openContextMenuAt } from "../../ui/ContextMenu";
import { useCutRows } from "../clipboard/clipboard";
import { openItems } from "../navigation/open";
import { nothingFocused } from "../navigation/panes";
import { beginRename, cancelRename, useRename } from "../rename/rename";
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
import { dragOut } from "./dragOut";
import { FileRow } from "./FileRow";
import { locationText } from "./format";
import { ListMenu, type ListMenuKind } from "./ListMenu";
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
 * A second click on a selected name renames it once this has passed without a third: Windows'
 * double-click time, as Explorer waits it out.
 */
const SLOW_CLICK_MS = 500;
/** From the keyboard, the item menu opens under the item, past its icon; the folder menu at the top. */
const ITEM_MENU_X_PX = 32;
const MENU_INSET_PX = 8;
/** The inline rename field (`RENAME_SCOPE`): a right click there isn't the list's. */
const RENAME_FIELD = '[data-command-scope="rename"]';

/**
 * A pane's file list in the Details view: the column header and a virtualized list of rows with
 * the density's fixed height (30 / 24 px), so 100 000 entries scroll at 60 fps. Rows show as the
 * folder's batches arrive and follow every change in it. Selection works as in Explorer, with
 * the mouse (`usePointerSelection`; dragging a selected item takes the selection out of Jerry,
 * `dragOut`) and, while the list has the focus, with the arrows, Home,
 * End, PageUp and PageDown (Shift selects a range, Ctrl moves only the focus) and by typing the
 * first letters of a name. A double click (or a single one, by the "click to open" setting)
 * opens the row: a folder in the tab, a file in its program; a middle click opens a folder in a
 * new tab. A slow second click on the name of the one selected row renames it, as F2 does (the
 * field stays in the list while it scrolls). The list shows `tab`'s folder and keeps its scroll
 * position for when the tab shows again; the pane mounts one per tab shown (`key`).
 */
export const FileList = memo(function FileList({ tab, pane }: { tab: Tab; pane: PaneSide }) {
  const { folder } = tab;
  const status = useFolder(folder, (state) => state.status);
  const error = useFolder(folder, (state) => state.error);
  const entries = useFolder(folder, (state) => state.entries);
  const filter = useFolder(folder, (state) => state.filter);
  const listing = useFolder(folder, (state) => state.listing);
  const searching = useFolder(folder, (state) => state.search !== undefined);
  const root = useFolder(folder, (state) => (state.search ? state.path : undefined));
  const sort = useFolder(folder, (state) => state.sort);
  const layout = useFolder(folder, (state) => state.columns);
  const selected = useFolder(folder, (state) => state.selected);
  const focused = useFolder(folder, (state) => state.focused);
  const cutRows = useCutRows(listing, status === "ready");
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
  // A tab shown again scrolls back to where it was, before the virtualizer reads the position.
  useLayoutEffect(() => {
    if (scroller.current) {
      scroller.current.scrollTop = folder.scrollTop;
    }
  }, [folder]);
  useKeyboardHandOver(scroller, pane);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => rowHeight,
    overscan: OVERSCAN,
    scrollMargin: margin,
    initialOffset: () => folder.scrollTop,
  });
  // biome-ignore lint/correctness/useExhaustiveDependencies: measures again for the new row height.
  useEffect(() => virtualizer.measure(), [virtualizer, rowHeight]);
  // Another folder starts at the top; the folder the list mounted with keeps the tab's place.
  const shownEntries = useRef(entries);
  useEffect(() => {
    if (shownEntries.current !== entries && scroller.current) {
      scroller.current.scrollTop = 0;
    }
    shownEntries.current = entries;
  }, [entries]);
  const slow = useDelayed(status === "loading", LOADING_DELAY_MS);

  const pointer = usePointerSelection({
    folder,
    rows,
    rowHeight,
    scroller,
    body,
    margin,
    onDragOut: () => void dragOut(folder, rows),
  });
  const typeAhead = useMemo(() => new TypeAhead(), []);
  const rowIds = useId();

  // The entry being renamed here, and where it is among the rows.
  const editing = useRename((state) =>
    state.editing?.folder === folder ? state.editing.id : undefined,
  );
  const editingAt = useMemo(
    () => (editing === undefined ? undefined : indexOf(rows, editing)),
    [rows, editing],
  );
  useRenameEnd(scroller, editing, editingAt);
  const slowClick = useSlowClick(folder, activePane, clickToOpen === "double");

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
  /** Scrolls row `index` to the middle of the view, unless it is in view already. */
  const center = (index: number) => {
    const view = scroller.current;
    if (!view) {
      return;
    }
    const top = index * rowHeight;
    const height = view.clientHeight - margin;
    if (top < view.scrollTop || top + rowHeight > view.scrollTop + height) {
      view.scrollTop = Math.max(0, top - (height - rowHeight) / 2);
    }
  };
  // A folder opened again (Back, Up, Refresh) scrolls to where it was, or to its focused entry.
  const scrollTo = useFolder(folder, (state) => state.scrollTo);
  // biome-ignore lint/correctness/useExhaustiveDependencies: `center` reads the latest sizes.
  useLayoutEffect(() => {
    const view = scroller.current;
    if (!scrollTo || !view) {
      return;
    }
    if (scrollTo.kind === "top") {
      view.scrollTop = scrollTo.top;
    } else {
      const at = indexOf(rows, scrollTo.id);
      if (at !== undefined) center(at);
    }
    folder.scrolled();
  }, [folder, scrollTo, rows]);

  // The context menu: on the selected items, or on the folder (empty space). Shift+F10 and the
  // Menu key open it at the focused item, as in Explorer, not where the browser would.
  const [menu, setMenu] = useState<ListMenuKind>("items");
  const fromKeyboard = useRef(false);
  const contextMenu = (event: MouseEvent) => {
    const target = event.target as Element;
    if (event.defaultPrevented) {
      return; // The column header's own menu.
    }
    if (target.closest(RENAME_FIELD)) {
      event.preventDefault(); // The rename field: no menu of the list's.
      return;
    }
    const keyboard = fromKeyboard.current;
    fromKeyboard.current = false;
    const onItem = keyboard
      ? folder.store.getState().selected.size > 0
      : target.closest("[data-id]") !== null;
    setMenu(onItem ? "items" : "folder");
  };
  const menuFromKeyboard = () => {
    const view = scroller.current;
    if (!view) {
      return;
    }
    const { selected, focused } = folder.store.getState();
    const anchor =
      focused !== undefined && selected.has(focused)
        ? focused
        : rows.find((id) => selected.has(id));
    const at = indexOf(rows, anchor);
    const box = view.getBoundingClientRect();
    let x = box.left + MENU_INSET_PX;
    let y = box.top + margin + MENU_INSET_PX;
    if (at !== undefined) {
      reveal(at);
      x = box.left + ITEM_MENU_X_PX;
      y = box.top + margin + (at + 1) * rowHeight - view.scrollTop;
    }
    fromKeyboard.current = true;
    openContextMenuAt(view, x, y);
  };

  const keyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    slowClick.cancel();
    // The list's own keys work while it has the focus, not a header button in it.
    if (
      event.defaultPrevented ||
      event.nativeEvent.isComposing ||
      event.target !== scroller.current
    ) {
      return;
    }
    if (isMenuKey(event)) {
      event.preventDefault();
      typeAhead.reset();
      menuFromKeyboard();
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
      slowClick.cancel();
      return;
    }
    if (clickToOpen === "single") {
      openItems(pane, tab, [id]);
    } else {
      slowClick.clicked(id, event.detail);
    }
  };
  const doubleClick = (event: MouseEvent) => {
    slowClick.cancel();
    const id = rowAt(event);
    if (id !== undefined && clickToOpen === "double") {
      openItems(pane, tab, [id]);
    }
  };
  // A middle click on a folder opens it in a new tab, in the background, as a browser's link.
  const auxClick = (event: MouseEvent) => {
    const id = rowAt(event);
    if (event.button === 1 && id !== undefined && entries.isFolder(id)) {
      openItems(pane, tab, [id], { newTab: true });
    }
  };

  const name = pane === "left" ? strings.frame.leftPane : strings.frame.rightPane;
  const message =
    status === "failed" && error ? (
      <>
        <p>{errorText(error)}</p>
        <button
          type="button"
          onClick={() => void folder.refresh()}
          className="pointer-events-auto h-8 rounded-ctrl border border-ctrl-border bg-ctrl px-3 text-fg hover:bg-hover"
        >
          {strings.fileList.retry}
        </button>
      </>
    ) : rows.length > 0 ? null : filter !== "" && total > 0 ? (
      strings.fileList.noMatch
    ) : status === "ready" ? (
      searching ? (
        strings.search.nothingFound
      ) : (
        strings.fileList.empty
      )
    ) : slow ? (
      searching ? (
        strings.search.nothingYet
      ) : (
        strings.fileList.loading
      )
    ) : null;

  const items = virtualizer.getVirtualItems();
  const focusShown = items.some((item) => rows[item.index] === focused);
  const { rect } = pointer;
  // The row being renamed stays, scrolled out of view or not, so its field keeps the typing.
  const shown = items.map((item) => ({ index: item.index, top: item.start - margin }));
  if (editingAt !== undefined && !items.some((item) => item.index === editingAt)) {
    shown.push({ index: editingAt, top: editingAt * rowHeight });
  }

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <ContextMenu
        label={menu === "items" ? strings.fileList.menu.items : strings.fileList.menu.folder}
        menu={<ListMenu kind={menu} folder={folder} />}
      >
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
          onAuxClick={auxClick}
          onContextMenu={contextMenu}
          // No autoscroll on a middle press: it opens a folder in a new tab.
          onMouseDown={(event) => event.button === 1 && event.preventDefault()}
          {...pointer.handlers}
          onPointerDown={(event) => {
            slowClick.pressed(event);
            pointer.handlers.onPointerDown(event);
          }}
          onScroll={() => {
            folder.scrollTop = scroller.current?.scrollTop ?? 0;
            pointer.handlers.onScroll();
          }}
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
              {shown.map((item) => {
                const id = rows[item.index] ?? 0;
                const key = entries.iconKeys[id] ?? IconKey.file;
                const modified = entries.modified[id] ?? 0;
                const location = entries.location(id);
                return (
                  <FileRow
                    key={id}
                    id={id}
                    idPrefix={rowIds}
                    index={item.index}
                    top={item.top}
                    columns={stableColumns}
                    name={entries.names[id] ?? ""}
                    location={location}
                    locationShown={
                      location === undefined || root === undefined
                        ? location
                        : locationText(location, root)
                    }
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
                    editing={editing === id}
                    cut={cutRows.has(id)}
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
      </ContextMenu>
      {message && (
        // Over the list, under its header, as wide as the pane even when the columns are wider.
        <div className="pointer-events-none absolute inset-x-0 top-col-header flex flex-col items-center gap-3 px-4 pt-6 text-center text-fg-2">
          {message}
        </div>
      )}
    </div>
  );
});

/** Panes whose list had the keyboard when it unmounted: the next one there (another tab's) takes it. */
const handOver = new Set<PaneSide>();

/**
 * Hands the keyboard from a pane's list to the list that replaces it: switching tabs with the
 * keyboard (Ctrl+Tab) keeps typing in the list. A list mounting otherwise takes nothing.
 */
function useKeyboardHandOver(scroller: RefObject<HTMLDivElement | null>, pane: PaneSide): void {
  useLayoutEffect(() => {
    const view = scroller.current;
    // Only if the keyboard went nowhere meanwhile: back to two panes, it stays where it is.
    if (view && handOver.delete(pane) && nothingFocused()) {
      view.focus({ preventScroll: true });
    }
    // Before the list leaves the page, while it still has the focus.
    return () => {
      if (view && view === document.activeElement) {
        handOver.add(pane);
      }
    };
  }, [scroller, pane]);
}

/**
 * The end of a rename here gives the keyboard back to the list if the field had it, and an entry
 * the list no longer shows (filtered out, hidden) stops being renamed.
 */
function useRenameEnd(
  scroller: RefObject<HTMLDivElement | null>,
  editing: number | undefined,
  at: number | undefined,
): void {
  useEffect(() => {
    if (editing !== undefined && at === undefined) {
      cancelRename();
    }
  }, [editing, at]);
  const was = useRef<number | undefined>(undefined);
  useLayoutEffect(() => {
    const ended = was.current !== undefined && editing === undefined;
    was.current = editing;
    if (ended && nothingFocused()) {
      scroller.current?.focus({ preventScroll: true });
    }
  }, [scroller, editing]);
}

/**
 * Explorer's slow double click: a press on the name of the one selected row, which has the focus,
 * in the active pane, then no second click within `SLOW_CLICK_MS`, renames it. Keys, a double
 * click or another press call it off.
 */
function useSlowClick(folder: Folder, activePane: boolean, enabled: boolean) {
  const armed = useRef<number | undefined>(undefined);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const cancel = useCallback(() => {
    clearTimeout(timer.current);
    timer.current = undefined;
  }, []);
  useEffect(() => cancel, [cancel]);
  const alone = (id: number) => {
    const { selected, focused } = folder.store.getState();
    return focused === id && selected.size === 1 && selected.has(id);
  };
  return {
    cancel,
    pressed(event: PointerEvent) {
      cancel();
      armed.current = undefined;
      const target = event.target as Element;
      const row = target.closest<HTMLElement>("[data-id]");
      const plain = event.button === 0 && !event.ctrlKey && !event.shiftKey && !event.altKey;
      if (enabled && activePane && plain && row && target.closest("[data-name]")) {
        const id = Number(row.dataset.id);
        armed.current = alone(id) ? id : undefined;
      }
    },
    clicked(id: number, clicks: number) {
      const was = armed.current;
      armed.current = undefined;
      if (was === id && clicks === 1) {
        timer.current = setTimeout(() => {
          timer.current = undefined;
          if (alone(id)) beginRename(folder, id);
        }, SLOW_CLICK_MS);
      }
    },
  };
}

/** Shift+F10 or the Menu key: the context menu, from the keyboard. */
function isMenuKey(event: KeyboardEvent): boolean {
  if (event.ctrlKey || event.altKey || event.metaKey) {
    return false;
  }
  return event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey);
}

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
