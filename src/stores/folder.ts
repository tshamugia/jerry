import { useStore } from "zustand";
import { createStore, type StoreApi } from "zustand/vanilla";
import { wildcardMatcher } from "../features/filter/wildcard";
import {
  commands,
  type JerryError,
  type ListingMessage,
  type ListingOpened,
  type ListingTarget,
  type OpenListing,
  openListing,
  type Sort,
  type SortColumn,
} from "../ipc";
import { type ColumnLayout, DEFAULT_COLUMNS } from "./columns";
import { FolderEntries, matchingRows, visibleRows } from "./entries";

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
  /** The quick filter's pattern (`wildcardMatcher`); "" when off. Another folder clears it. */
  filter: string;
  /**
   * Where the list should scroll, once: set when a folder opened again (Back, Up, Refresh) has its
   * entries in; the list scrolls and calls `scrolled`.
   */
  scrollTo: ScrollTo | undefined;
}

/** A scroll the list should make: to a position, or until an entry is in view. */
export type ScrollTo = { kind: "top"; top: number } | { kind: "reveal"; id: number };

/**
 * What a folder opened again gets back once its entries are in, by name: entries keep their names
 * but get new ids in a new listing. Only if nothing was selected meanwhile.
 */
export interface Restore {
  /** The entry to focus: the folder Up came from, the entry focused when Back left. */
  focus?: string;
  /** The entries to select (Refresh); without them, the focused entry is selected. */
  selected?: readonly string[];
  /** Where the list was scrolled (Refresh); without it, the focused entry is scrolled to. */
  scrollTop?: number;
  /** The quick filter to keep (Refresh); another folder starts without one. */
  filter?: string;
}

/** What the list shows: its rows, and how many there would be without the filter. */
export interface ShownRows {
  /** Entry ids in display order. */
  rows: Uint32Array;
  /** The rows without the filter: "Showing 12 of 340". */
  total: number;
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
    filter: "",
    scrollTo: undefined,
  };
}

/**
 * One tab's folder view: its live listing (`listing_open`), sort, columns, quick filter,
 * selection and scroll position. Messages from the core go into the store as they arrive: rows
 * show batch by batch, then in the core's order, and follow every change in the folder. Every
 * tab has one (`Tab.folder`, `src/stores/tabs.ts`), which keeps its sort, columns and scroll as
 * the tab goes from folder to folder.
 */
export class Folder {
  readonly store: StoreApi<FolderState>;
  /**
   * How far the list was scrolled, in px. Not state: the list writes it as it scrolls and reads
   * it back when the tab shows again.
   */
  scrollTop = 0;
  private current: OpenListing | undefined;
  /** Counts opens, so messages and answers for an earlier folder are dropped. */
  private generation = 0;
  private target: ListingTarget | undefined;
  private readonly listeners = new Set<(message: ListingMessage) => void>();
  private shownCache:
    | { key: readonly unknown[]; showHidden: boolean; shown: ShownRows }
    | undefined;
  /** What the folder opening now gets back once its entries are in. */
  private restore: Restore | undefined;

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

  /**
   * Opens `target` here, in place of the folder shown, and once its entries are in gives it back
   * what `restore` names. Resolves once the core has answered: with what it opened, or nothing if
   * it failed (the list shows why) or another open came after this one.
   */
  async open(target: ListingTarget, restore?: Restore): Promise<ListingOpened | undefined> {
    const previous = this.current;
    this.current = undefined;
    const mine = ++this.generation;
    this.target = target;
    this.restore = restore;
    this.scrollTop = 0;
    const { sort, version } = this.store.getState();
    this.store.setState({
      ...emptyFolder(),
      filter: restore?.filter ?? "",
      status: "loading",
      version: version + 1,
    });

    const result = await openListing(this.tab, target, sort, (message) =>
      this.receive(mine, message),
    );
    if (result.status === "error") {
      // The core didn't start the new listing, so the previous one is still open there.
      if (previous) {
        void commands.listingClose(previous.id);
      }
      if (mine === this.generation) {
        this.restore = undefined;
        this.store.setState({ status: "failed", error: result.error });
      }
      return undefined;
    }
    if (mine !== this.generation) {
      void result.data.close();
      return undefined;
    }
    this.current = result.data;
    this.store.setState({ listing: result.data.id, path: result.data.path });
    // A sort asked for before the core answered: the stream's order follows it from now on.
    const wanted = this.store.getState().sort;
    if (!sameSort(wanted, sort)) {
      void result.data.sort(wanted);
    }
    const { id, path, name, folder, parent, crumbs } = result.data;
    return { id, path, name, folder, parent, crumbs };
  }

  /**
   * Opens the folder again, keeping the selection, the focus, the filter and the scroll position
   * by name: Ctrl+R, and Retry after an error.
   */
  refresh(): Promise<ListingOpened | undefined> {
    const { listing, entries, selected, focused, filter } = this.store.getState();
    if (listing === undefined) {
      return this.target ? this.open(this.target) : Promise.resolve(undefined);
    }
    const name = (id: number) => entries.names[id] ?? "";
    return this.open(
      { kind: "listing", listing },
      {
        selected: [...selected].map(name),
        focus: focused === undefined ? undefined : name(focused),
        scrollTop: this.scrollTop,
        filter,
      },
    );
  }

  /** Asks the list to scroll until entry `id` is in view: a new folder, a renamed entry. */
  reveal(id: number): void {
    this.store.setState({ scrollTo: { kind: "reveal", id } });
  }

  /** The list made the scroll `scrollTo` asked for. */
  scrolled(): void {
    if (this.store.getState().scrollTo !== undefined) {
      this.store.setState({ scrollTo: undefined });
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

  /** Puts in a new selection (`src/features/selection/model.ts` makes them). */
  setSelection(selection: Selection): void {
    const state = this.store.getState();
    if (
      selection.selected !== state.selected ||
      selection.anchor !== state.anchor ||
      selection.focused !== state.focused
    ) {
      // The user's own selection wins over one a reopened folder would get back.
      this.restore = undefined;
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

  /** Shows only the entries whose names match `filter` (`wildcardMatcher`); "" shows them all. */
  setFilter(filter: string): void {
    if (filter !== this.store.getState().filter) {
      this.store.setState({ filter });
      this.unselectFiltered();
    }
  }

  /**
   * The rows the list shows: `visibleRows` narrowed by the filter, none after a failure. The same
   * object until the folder, its filter or `showHidden` change, so the list, the filter field and
   * the status bar share one pass over the entries.
   */
  shown(showHidden: boolean): ShownRows {
    const { status, entries, order, version, filter } = this.store.getState();
    const key = [status, entries, order, version, filter, showHidden];
    const cached = this.shownCache;
    if (cached?.key.every((part, i) => part === key[i])) {
      return cached.shown;
    }
    const all = status === "failed" ? new Uint32Array(0) : visibleRows(entries, order, showHidden);
    const rows = filter === "" ? all : matchingRows(entries, all, wildcardMatcher(filter));
    const shown = { rows, total: all.length };
    this.shownCache = { key, showHidden, shown };
    return shown;
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
        // The removed ids are few; the selection can be the whole folder.
        let selected = state.selected;
        if (message.removed.some((id) => state.selected.has(id))) {
          const kept = new Set(state.selected);
          for (const id of gone) {
            kept.delete(id);
          }
          selected = kept;
        }
        const before = this.shownCache;
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
          const at = before.shown.rows.indexOf(state.focused ?? 0);
          const { rows } = this.shown(before.showHidden);
          const focused = at < 0 ? undefined : rows[Math.min(at, rows.length - 1)];
          this.store.setState({ focused, anchor: lost(state.anchor) ? focused : state.anchor });
        }
        break;
      }
      case "done":
        this.store.setState({ status: "ready" });
        this.giveBack();
        break;
      case "error":
        this.restore = undefined;
        this.store.setState({ status: "failed", error: message.error });
        break;
    }
    if (message.kind === "diff") {
      // A selected entry renamed (in Explorer, say) to a name the filter hides.
      this.unselectFiltered();
    }
    for (const listener of this.listeners) {
      listener(message);
    }
  }

  /**
   * Entries the filter hides leave the selection and lose the focus, so a file operation never
   * acts on files the list doesn't show.
   */
  private unselectFiltered(): void {
    const { filter, entries, selected, focused } = this.store.getState();
    if (filter === "") {
      return;
    }
    const matches = wildcardMatcher(filter);
    const shown = (id: number) => matches(entries.names[id] ?? "");
    const kept = [...selected].filter(shown);
    const keepFocus = focused === undefined || shown(focused);
    if (kept.length < selected.size || !keepFocus) {
      this.store.setState({
        selected: kept.length < selected.size ? new Set(kept) : selected,
        focused: keepFocus ? focused : undefined,
      });
    }
  }

  /**
   * The folder's entries are all in: gives back what `open` was asked to restore, by name. The
   * focused entry is selected too unless `selected` says otherwise, and the list scrolls to where
   * it was or to the focused entry. Names the folder no longer has are left out.
   */
  private giveBack(): void {
    const restore = this.restore;
    this.restore = undefined;
    if (!restore) {
      return;
    }
    const { entries } = this.store.getState();
    const wanted = new Set(restore.selected ?? []);
    if (restore.focus !== undefined) {
      wanted.add(restore.focus);
    }
    const ids = new Map<string, number>();
    for (let id = 0; id < entries.count && ids.size < wanted.size; id++) {
      const name = entries.names[id];
      if (name !== undefined && wanted.has(name) && !entries.removed.has(id)) {
        ids.set(name, id);
      }
    }
    const focused = restore.focus === undefined ? undefined : ids.get(restore.focus);
    const names = restore.selected ?? (restore.focus === undefined ? [] : [restore.focus]);
    const selected = new Set<number>();
    for (const name of names) {
      const id = ids.get(name);
      if (id !== undefined) selected.add(id);
    }
    const scrollTo: ScrollTo | undefined =
      restore.scrollTop !== undefined
        ? { kind: "top", top: restore.scrollTop }
        : focused !== undefined
          ? { kind: "reveal", id: focused }
          : undefined;
    this.store.setState({ selected, anchor: focused, focused, scrollTo });
  }
}

/** A slice of a folder's state; the component re-renders when it changes. */
export function useFolder<T>(folder: Folder, selector: (state: FolderState) => T): T {
  return useStore(folder.store, selector);
}
