import { useVirtualizer } from "@tanstack/react-virtual";
import {
  type MouseEvent,
  memo,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { type Density, entryIconUrl, IconKey } from "../../ipc";
import { shownColumns } from "../../stores/columns";
import { visibleRows } from "../../stores/entries";
import { type Folder, useFolder } from "../../stores/folder";
import { useSetting } from "../../stores/settings";
import { type PaneSide, useWindow } from "../../stores/window";
import { errorText, strings } from "../../strings";
import { ColumnHeader } from "./ColumnHeader";
import { columnWidths } from "./cells";
import { FileRow } from "./FileRow";

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
 * folder's batches arrive and follow every change in it. A click selects a row; a double click
 * (or a single one, by the "click to open" setting) opens a folder.
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
  const order = useFolder(folder, (state) => state.order);
  const version = useFolder(folder, (state) => state.version);
  const listing = useFolder(folder, (state) => state.listing);
  const sort = useFolder(folder, (state) => state.sort);
  const layout = useFolder(folder, (state) => state.columns);
  const selected = useFolder(folder, (state) => state.selected);
  const focused = useFolder(folder, (state) => state.focused);
  const activePane = useWindow((state) => state.activePane === pane);
  const showHidden = useSetting("showHidden") ?? false;
  const showExtensions = useSetting("showExtensions") ?? true;
  const foldersFirst = useSetting("foldersFirst") ?? true;
  const dateFormat = useSetting("dateFormat") ?? "DD.MM.YYYY";
  const clickToOpen = useSetting("clickToOpen") ?? "double";
  const density = useSetting("density") ?? "comfortable";

  useEffect(() => folder.setFoldersFirst(foldersFirst), [folder, foldersFirst]);

  // `entries` and `order` change in place: `version` says when.
  // biome-ignore lint/correctness/useExhaustiveDependencies: version tracks in-place changes.
  const rows = useMemo(
    () => (status === "failed" ? new Uint32Array(0) : visibleRows(entries, order, showHidden)),
    [status, entries, order, version, showHidden],
  );
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

  const rowAt = (event: MouseEvent) => {
    const row = (event.target as Element).closest<HTMLElement>("[data-id]");
    return row ? Number(row.dataset.id) : undefined;
  };
  const click = (event: MouseEvent) => {
    const id = rowAt(event);
    if (id !== undefined) {
      folder.select(id);
      if (clickToOpen === "single") {
        folder.openEntry(id);
      }
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
    ) : rows.length > 0 ? null : status === "ready" ? (
      strings.fileList.empty
    ) : slow ? (
      strings.fileList.loading
    ) : null;

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      {/* biome-ignore lint/a11y/useSemanticElements: a virtualized grid; see FileRow. */}
      {/* biome-ignore lint/a11y/useKeyWithClickEvents: rows are clicked; the keys come with selection (#30) and opening (#31). */}
      <div
        ref={scroller}
        role="grid"
        aria-label={strings.fileList.files(name)}
        aria-rowcount={rows.length + 1}
        aria-colcount={stableColumns.length}
        aria-busy={status === "loading"}
        tabIndex={0}
        data-file-list={pane}
        className={`group relative min-h-0 flex-1 overflow-auto ${focused === undefined ? "" : "focus-visible:outline-none"}`}
        style={columnWidths(layout)}
        onClick={click}
        onDoubleClick={doubleClick}
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
            {virtualizer.getVirtualItems().map((item) => {
              const id = rows[item.index] ?? 0;
              const key = entries.iconKeys[id] ?? IconKey.file;
              const modified = entries.modified[id] ?? 0;
              return (
                <FileRow
                  key={id}
                  id={id}
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
