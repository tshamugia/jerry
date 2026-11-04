import { useStore } from "zustand";
import { createStore, type StoreApi } from "zustand/vanilla";
import {
  commands,
  type JerryError,
  type ListingMessage,
  type ListingTarget,
  type OpenListing,
  openListing,
  type Sort,
  type SortColumn,
} from "../ipc";
import { type ColumnLayout, DEFAULT_COLUMNS } from "./columns";
import { FolderEntries, visibleRows } from "./entries";
import type { PaneSide } from "./window";

/**
 * Where a folder view is: no folder yet, reading it (rows show as batches arrive), all in (and
 * live), or failed (reading it failed, or the folder went away).
 */
export type FolderStatus = "none" | "loading" | "ready" | "failed";

export interface FolderState {
  status: FolderStatus;
  /** The folder as display text, once the core has answered. */
  path: string | undefined;
  /** Why it failed. */
  error: JerryError | undefined;
  /** The open listing's id: entries' icons and subfolders need it. */
  listing: number | undefined;
  /** The sort asked for last; the rows follow it once its order arrives. */
  sort: Sort;
  /** Changed in place: `version` changes with it. */
  entries: FolderEntries;
  /** Every entry's id in display order, hidden ones too; until the first order, arrival order. */
  order: Uint32Array | undefined;
  /** Changes whenever `entries` or `order` change. */
  version: number;
  columns: ColumnLayout;
  /** The selected entries. A new set on every change (`src/features/selection/model.ts`). */
  selected: ReadonlySet<number>;
  /** Where Shift ranges start: the row last clicked or moved to without Shift. */
  anchor: number | undefined;
  /** The entry with the keyboard focus. */
  focused: number | undefined;
}

/** The selection part of a folder's state. */
export type Selection = Pick<FolderState, "selected" | "anchor" | "focused">;

const DEFAULT_SORT: Sort = { column: "name", descending: false, foldersFirst: true };

const sameSort = (a: Sort, b: Sort) =>
  a.column === b.column && a.descending === b.descending && a.foldersFirst === b.foldersFirst;

/** What a folder view shows before it has a folder, and right when it opens one. */
function emptyFolder() {
  return {
    path: undefined,
    error: undefined,
    listing: undefined,
    entries: new FolderEntries(),
    order: undefined,
    selected: new Set<number>(),
    anchor: undefined,
    focused: undefined,
  };
}

/** The shown rows, for the state they were computed from. */
interface ShownRows {
  version: number;
  failed: boolean;
  showHidden: boolean;
  rows: Uint32Array;
}

/**
 * One folder shown in a list: its live listing (`listing_open`), sort, columns and selection.
 * Messages from the core go into the store as they arrive: rows show batch by batch, then in the
 * core's order, and follow every change in the folder. A pane has one until tabs (#31) give
 * every tab its own.
 */
export class Folder {
  readonly store: StoreApi<FolderState>;
  private current: OpenListing | undefined;
  /** Counts opens, so messages and answers for an earlier folder are dropped. */
  private generation = 0;
  private target: ListingTarget | undefined;
  private readonly listeners = new Set<(message: ListingMessage) => void>();
  private shown: ShownRows | undefined;

  /** `tab`: the core keeps one listing per tab; opening another folder closes the previous. */
  constructor(readonly tab: number) {
    this.store = createStore<FolderState>(() => ({
      ...emptyFolder(),
      status: "none",
      sort: DEFAULT_SORT,
      version: 0,
      columns: DEFAULT_COLUMNS,
    }));
  }

  /** Opens `target` here, in place of the folder shown. Resolves once the core has answered. */
  async open(target: ListingTarget): Promise<void> {
    const previous = this.current;
    this.current = undefined;
    const mine = ++this.generation;
    this.target = target;
    const { sort, version } = this.store.getState();
    this.store.setState({ ...emptyFolder(), status: "loading", version: version + 1 });

    const result = await openListing(this.tab, target, sort, (message) =>
      this.receive(mine, message),
    );
    if (result.status === "error") {
      // The core didn't start the new listing, so the previous one is still open there.
      if (previous) {
        void commands.listingClose(previous.id);
      }
      if (mine === this.generation) {
        this.store.setState({ status: "failed", error: result.error });
      }
      return;
    }
    if (mine !== this.generation) {
      void result.data.close();
      return;
    }
    this.current = result.data;
    this.store.setState({ listing: result.data.id, path: result.data.path });
    // A sort asked for before the core answered: the stream's order follows it from now on.
    const wanted = this.store.getState().sort;
    if (!sameSort(wanted, sort)) {
      void result.data.sort(wanted);
    }
  }

  /** Opens the folder again: after an error, the next step the list offers. */
  retry(): Promise<void> {
    const { listing } = this.store.getState();
    const target = listing === undefined ? this.target : { kind: "listing" as const, listing };
    return target ? this.open(target) : Promise.resolve();
  }

  /** Opens entry `id` if it is a folder. Files open in their app with #31. */
  openEntry(id: number): void {
    const { entries, listing } = this.store.getState();
    if (listing !== undefined && entries.isFolder(id)) {
      void this.open({ kind: "entry", listing, entry: id });
    }
  }

  /** Like a column header: a new column sorts ascending, the sorted one flips direction. */
  sortBy(column: SortColumn): void {
    const { sort } = this.store.getState();
    this.setSort({
      ...sort,
      column,
      descending: column === sort.column ? !sort.descending : false,
    });
  }

  /** Follows the "Folders before files" setting. */
  setFoldersFirst(foldersFirst: boolean): void {
    const { sort } = this.store.getState();
    if (sort.foldersFirst !== foldersFirst) {
      this.setSort({ ...sort, foldersFirst });
    }
  }

  setSort(sort: Sort): void {
    this.store.setState({ sort });
    // Without a listing yet, `open` passes it on once the core answers.
    void this.current?.sort(sort);
  }

  setColumns(columns: ColumnLayout): void {
    this.store.setState({ columns });
  }

  /**
   * The rows the list shows, as entry ids in display order: what it draws, and all that the
   * selection works on (Ctrl+A selects only these). `showHidden` is the setting. The same array
   * until the folder or the setting changes.
   */
  rows(showHidden: boolean): Uint32Array {
    const { status, entries, order, version } = this.store.getState();
    const failed = status === "failed";
    const shown = this.shown;
    if (shown?.version === version && shown.failed === failed && shown.showHidden === showHidden) {
      return shown.rows;
    }
    const rows = failed ? new Uint32Array(0) : visibleRows(entries, order, showHidden);
    this.shown = { version, failed, showHidden, rows };
    return rows;
  }

  /** Puts in a new selection (`src/features/selection/model.ts` makes them). */
  setSelection(selection: Selection): void {
    const state = this.store.getState();
    if (
      selection.selected !== state.selected ||
      selection.anchor !== state.anchor ||
      selection.focused !== state.focused
    ) {
      this.store.setState(selection);
    }
  }

  /**
   * Drops the selection, anchor and focus on entries `rows` leaves out: hidden again, or filtered
   * out. Later operations never act on entries the list doesn't show.
   */
  keepShown(rows: Uint32Array): void {
    const { entries, selected, anchor, focused } = this.store.getState();
    if (selected.size === 0 && anchor === undefined && focused === undefined) {
      return;
    }
    const shown = new Uint8Array(entries.count);
    for (const id of rows) {
      shown[id] = 1;
    }
    const kept = (id: number | undefined) => (id !== undefined && shown[id] ? id : undefined);
    let dropped = false;
    for (const id of selected) {
      if (!shown[id]) {
        dropped = true;
        break;
      }
    }
    this.setSelection({
      selected: dropped ? new Set([...selected].filter((id) => shown[id])) : selected,
      anchor: kept(anchor),
      focused: kept(focused),
    });
  }

  /** Stops the listing: the view shows no folder. */
  close(): void {
    this.generation += 1;
    void this.current?.close();
    this.current = undefined;
    const { version } = this.store.getState();
    this.store.setState({ ...emptyFolder(), status: "none", version: version + 1 });
  }

  /** Calls `listener` with every message from the core, once the store has it. */
  listen(listener: (message: ListingMessage) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private receive(mine: number, message: ListingMessage): void {
    if (mine !== this.generation) {
      return;
    }
    const state = this.store.getState();
    const version = state.version + 1;
    switch (message.kind) {
      case "batch":
        state.entries.appendBatch(message);
        this.store.setState({ version });
        break;
      case "order":
        // An order for an earlier sort can still be on its way.
        if (sameSort(message.sort, state.sort)) {
          this.store.setState({ order: message.ids, version });
        }
        break;
      case "diff": {
        state.entries.applyDiff(message);
        const gone = new Set(message.removed);
        if (gone.size === 0) {
          this.store.setState({ version });
          break;
        }
        // The order that follows leaves them out; until it comes, so does this one.
        const order = state.order?.filter((id) => !gone.has(id));
        const selected = [...state.selected].some((id) => gone.has(id))
          ? new Set([...state.selected].filter((id) => !gone.has(id)))
          : state.selected;
        const before = this.shown;
        const lost = (id: number | undefined) => id !== undefined && gone.has(id);
        this.store.setState({
          order,
          selected,
          anchor: lost(state.anchor) ? undefined : state.anchor,
          focused: lost(state.focused) ? undefined : state.focused,
          version,
        });
        // As in Explorer, the focus goes to the row that takes the removed one's place.
        if (lost(state.focused) && before) {
          const at = before.rows.indexOf(state.focused ?? 0);
          const rows = this.rows(before.showHidden);
          const focused = at < 0 ? undefined : rows[Math.min(at, rows.length - 1)];
          this.store.setState({ focused, anchor: lost(state.anchor) ? focused : state.anchor });
        }
        break;
      }
      case "done":
        this.store.setState({ status: "ready" });
        break;
      case "error":
        this.store.setState({ status: "failed", error: message.error });
        break;
    }
    for (const listener of this.listeners) {
      listener(message);
    }
  }
}

/** Each pane's folder, until tabs (#31) give every tab its own. The core's tab ids 1 and 2. */
const paneFolders: Record<PaneSide, Folder> = { left: new Folder(1), right: new Folder(2) };

export function paneFolder(pane: PaneSide): Folder {
  return paneFolders[pane];
}

/** A slice of a folder's state; the component re-renders when it changes. */
export function useFolder<T>(folder: Folder, selector: (state: FolderState) => T): T {
  return useStore(folder.store, selector);
}
