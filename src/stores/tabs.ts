import { create } from "zustand";
import { createStore, type StoreApi } from "zustand/vanilla";
import type { Crumb, ListingOpened, ListingTarget } from "../ipc";
import { Folder, type Restore } from "./folder";
import type { PaneSide } from "./window";

/** A folder in a tab's history. */
export interface HistoryEntry {
  /** The core's token for the folder: it opens again by its exact path (`ListingTarget` folder). */
  folder: number;
  /** The folder as display text: the tab's tooltip, the address bar. */
  path: string;
  /** The tab's title. */
  name: string;
  /** The token of the folder it is in, for Up; `null` at a root. */
  parent: number | null;
  /** The address bar's segments, from the root to the folder. */
  crumbs: readonly Crumb[];
  /** The entry that had the focus when the tab left the folder: it gets it back on return. */
  focus?: string;
}

export interface TabState {
  /** The folders the tab went through, oldest first; at most `MAX_HISTORY`. */
  history: readonly HistoryEntry[];
  /** The folder shown: its place in `history`, -1 before the first folder opens. */
  index: number;
}

/** Folders a tab's history keeps; past it the oldest go. */
export const MAX_HISTORY = 100;

/** Closed tabs Ctrl+Shift+T can bring back. */
export const MAX_CLOSED = 25;

/** What a new tab takes from the tab it opens from, so the folder looks the same there. */
type Look = Pick<ReturnType<Folder["store"]["getState"]>, "sort" | "columns">;

const entryOf = (opened: ListingOpened): HistoryEntry => ({
  folder: opened.folder,
  path: opened.path,
  name: opened.name,
  parent: opened.parent,
  crumbs: opened.crumbs,
});

/**
 * A tab: its own folder view (`folder`: listing, sort, columns, filter, selection, scroll) and
 * its history of folders, with Back, Forward and Up as in a browser and Explorer. Opening a new
 * folder drops the history after the folder shown.
 */
export class Tab {
  readonly folder: Folder;
  readonly store: StoreApi<TabState>;
  /** Counts navigations: the answer to an earlier one doesn't touch the history. */
  private navigation = 0;

  constructor(
    readonly id: number,
    from?: { look?: Look; history?: readonly HistoryEntry[]; index?: number },
  ) {
    this.folder = new Folder(id);
    if (from?.look) {
      this.folder.applyLook(from.look);
    }
    const history = from?.history ?? [];
    const index = Math.min(from?.index ?? history.length - 1, history.length - 1);
    this.store = createStore<TabState>(() => ({ history, index }));
  }

  /** The folder shown, once one has opened. */
  current(): HistoryEntry | undefined {
    const { history, index } = this.store.getState();
    return history[index];
  }

  /** The sort and columns, for a tab opened from this one. */
  look(): Look {
    const { sort, columns } = this.folder.store.getState();
    return { sort, columns };
  }

  /**
   * Opens `target` here as a new step in the history: the steps after the folder shown go.
   * `restore` names what the folder gets back once its entries are in.
   */
  async open(target: ListingTarget, restore?: Restore): Promise<void> {
    const mine = ++this.navigation;
    this.rememberFocus();
    const opened = await this.folder.open(target, restore);
    if (!opened || mine !== this.navigation) {
      return;
    }
    const { history, index } = this.store.getState();
    const entry = entryOf(opened);
    // Opening the folder shown again (its path typed, say) isn't a step.
    if (history[index]?.folder === entry.folder) {
      this.store.setState({ history: history.with(index, entry) });
      return;
    }
    const kept = [...history.slice(0, index + 1), entry].slice(-MAX_HISTORY);
    this.store.setState({ history: kept, index: kept.length - 1 });
  }

  /** Goes to the folder at `index` in the history, focusing the entry focused when it left. */
  go(index: number): void {
    const { history, index: at } = this.store.getState();
    const entry = history[index];
    if (!entry || index === at) {
      return;
    }
    this.rememberFocus();
    this.navigation++;
    // At once, so Back held down steps back once per repeat.
    this.store.setState({ index });
    void this.folder.open({ kind: "folder", folder: entry.folder }, { focus: entry.focus });
  }

  /** Alt+←, Backspace. */
  back(): void {
    this.go(this.store.getState().index - 1);
  }

  /** Alt+→. */
  forward(): void {
    this.go(this.store.getState().index + 1);
  }

  /** Alt+↑: the folder this one is in, with this one focused, as in Explorer. */
  up(): void {
    const current = this.current();
    if (current?.parent != null) {
      void this.open({ kind: "folder", folder: current.parent }, { focus: current.name });
    }
  }

  /** Ctrl+R: the same folder, read again, keeping the selection and the scroll position. */
  refresh(): void {
    void this.folder.refresh();
  }

  /**
   * Opens the folder shown, as a restored tab does (#42), focusing the entry focused when it was
   * left. If the folder no longer exists, the tab goes to the nearest folder that does, in its
   * place in the history, and calls `moved` with the folder that went away and the one shown.
   */
  restore(moved?: (gone: HistoryEntry, shown: HistoryEntry) => void): void {
    const entry = this.current();
    if (!entry) {
      return;
    }
    const mine = ++this.navigation;
    const stop = this.folder.listen((message) => {
      if (message.kind !== "done" && message.kind !== "error") {
        return;
      }
      stop();
      if (
        message.kind === "error" &&
        message.error.code === "NotFound" &&
        mine === this.navigation
      ) {
        void this.openNearest(entry, moved);
      }
    });
    void this.folder
      .open({ kind: "folder", folder: entry.folder }, { focus: entry.focus })
      .then((opened) => {
        if (!opened) stop();
      });
  }

  /** The nearest folder to `gone` that still exists, in its place in the history. */
  private async openNearest(
    gone: HistoryEntry,
    moved?: (gone: HistoryEntry, shown: HistoryEntry) => void,
  ): Promise<void> {
    const mine = ++this.navigation;
    const opened = await this.folder.open({ kind: "nearest", folder: gone.folder });
    if (!opened || mine !== this.navigation) {
      return;
    }
    const shown = entryOf(opened);
    const { history, index } = this.store.getState();
    // Gone from the folder it was opened from: that one is already the step before.
    this.store.setState(
      history[index - 1]?.folder === shown.folder
        ? { history: history.toSpliced(index, 1), index: index - 1 }
        : { history: history.with(index, shown) },
    );
    if (shown.folder !== gone.folder) {
      moved?.(gone, shown);
    }
  }

  /** Stops the tab's listing. */
  close(): void {
    this.navigation++;
    this.folder.close();
  }

  /**
   * Notes the focused entry in the history, for when the tab comes back to this folder: before it
   * leaves it, closes or is copied.
   */
  rememberFocus(): void {
    const { history, index } = this.store.getState();
    const entry = history[index];
    if (!entry) {
      return;
    }
    const { focused, entries } = this.folder.store.getState();
    const focus = focused === undefined ? undefined : entries.names[focused];
    if (focus !== entry.focus) {
      this.store.setState({ history: history.with(index, { ...entry, focus }) });
    }
  }
}

/** A tab in its pane's strip. */
export interface PaneTab {
  id: number;
  /** Pinned tabs are narrow, stay at the left and don't close by accident. */
  pinned: boolean;
}

export interface PaneTabs {
  /** Pinned tabs first. */
  tabs: readonly PaneTab[];
  /** The tab the pane shows; `undefined` only before the first tab. */
  active: number | undefined;
  /**
   * Tabs opened in the background from the active tab since it became active: the next one goes
   * after them, so they stay in the order they were opened (as in browsers).
   */
  opened: number;
}

/** A closed tab, for Ctrl+Shift+T. */
interface ClosedTab {
  pane: PaneSide;
  /** Its place in the strip. */
  index: number;
  pinned: boolean;
  history: readonly HistoryEntry[];
  at: number;
  look: Look;
}

interface TabsState {
  left: PaneTabs;
  right: PaneTabs;
  /** The last closed tabs, latest last. */
  closed: readonly ClosedTab[];
}

const noTabs: PaneTabs = { tabs: [], active: undefined, opened: 0 };

/** Each pane's tabs, as the title bar shows them. Each tab's folder and history are its `Tab`. */
export const useTabs = create<TabsState>(() => ({ left: noTabs, right: noTabs, closed: [] }));

const registry = new Map<number, Tab>();
let lastId = 0;

/** The tab with id `id`, while it is open. */
export function tabById(id: number | undefined): Tab | undefined {
  return id === undefined ? undefined : registry.get(id);
}

/** The tab `pane` shows. */
export function activeTab(pane: PaneSide): Tab | undefined {
  return tabById(useTabs.getState()[pane].active);
}

/** The tab `pane` shows; the component re-renders when another one shows. */
export function useActiveTab(pane: PaneSide): Tab | undefined {
  return tabById(useTabs((state) => state[pane].active));
}

function setPane(pane: PaneSide, update: Partial<PaneTabs>): void {
  useTabs.setState((state) => ({ [pane]: { ...state[pane], ...update } }));
}

/** Pinned tabs first, each group in its own order. */
function pinnedFirst(tabs: readonly PaneTab[]): PaneTab[] {
  return [...tabs.filter((tab) => tab.pinned), ...tabs.filter((tab) => !tab.pinned)];
}

/** Where a new tab goes: right after `index`, kept out of the pinned group. */
function insertAt(tabs: readonly PaneTab[], index: number, tab: PaneTab): PaneTab[] {
  const pinned = tabs.filter((each) => each.pinned).length;
  const at = tab.pinned ? Math.min(index, pinned) : Math.max(index, pinned);
  return [...tabs.slice(0, at), tab, ...tabs.slice(at)];
}

/**
 * Adds a tab to `pane` at `index` (default: the end) and returns it; `activate` shows it. The
 * caller opens its folder. A restored tab keeps its `id` (its listing may have started already);
 * others get the next one.
 */
export function addTab(
  pane: PaneSide,
  options: {
    id?: number;
    index?: number;
    activate?: boolean;
    pinned?: boolean;
    from?: ConstructorParameters<typeof Tab>[1];
  } = {},
): Tab {
  const id = options.id ?? lastId + 1;
  lastId = Math.max(lastId, id);
  const tab = new Tab(id, options.from);
  registry.set(tab.id, tab);
  const current = useTabs.getState()[pane];
  const entry = { id: tab.id, pinned: options.pinned ?? false };
  const tabs = insertAt(current.tabs, options.index ?? current.tabs.length, entry);
  const activate = options.activate ?? current.active === undefined;
  setPane(pane, activate ? { tabs, active: tab.id, opened: 0 } : { tabs });
  return tab;
}

/**
 * Opens `target` in a new tab right after `pane`'s active one, taking its sort and columns:
 * Ctrl+T, the + button, a middle click or Ctrl+Enter on a folder, `jerry.exe <path>`. In the
 * background, a tab goes after the ones opened before it from the same tab. `restore` names what
 * the folder gets once its entries are in (a file to select).
 */
export function openInNewTab(
  pane: PaneSide,
  target: ListingTarget,
  { activate = true, restore }: { activate?: boolean; restore?: Restore } = {},
): Tab {
  const { tabs, active, opened } = useTabs.getState()[pane];
  const at = tabs.findIndex((tab) => tab.id === active);
  const look = activeTab(pane)?.look();
  const tab = addTab(pane, {
    index: at + 1 + (activate ? 0 : opened),
    activate,
    from: { look },
  });
  if (!activate) {
    setPane(pane, { opened: opened + 1 });
  }
  void tab.open(target, restore);
  return tab;
}

/** Shows `id` in `pane`. */
export function activateTab(pane: PaneSide, id: number): void {
  const current = useTabs.getState()[pane];
  if (current.active !== id && current.tabs.some((tab) => tab.id === id)) {
    setPane(pane, { active: id, opened: 0 });
  }
}

/**
 * Closes `id`. A pinned tab doesn't close by accident (unpin it first), and a pane keeps its last
 * tab. If it was the shown tab, its right neighbour shows, or else its left one. Ctrl+Shift+T
 * brings it back.
 */
export function closeTab(pane: PaneSide, id: number): void {
  const { tabs, active } = useTabs.getState()[pane];
  const index = tabs.findIndex((tab) => tab.id === id);
  const tab = registry.get(id);
  if (index < 0 || tabs[index]?.pinned || tabs.length === 1 || !tab) {
    return;
  }
  const rest = tabs.filter((each) => each.id !== id);
  const shown = active === id ? (rest[index] ?? rest[index - 1])?.id : active;
  remember(pane, index, tabs[index]?.pinned ?? false, tab);
  setPane(pane, { tabs: rest, active: shown, ...(active === id ? { opened: 0 } : {}) });
  registry.delete(id);
  tab.close();
}

function remember(pane: PaneSide, index: number, pinned: boolean, tab: Tab): void {
  tab.rememberFocus();
  const { history, index: at } = tab.store.getState();
  if (history.length === 0) {
    return;
  }
  const closed = { pane, index, pinned, history, at, look: tab.look() };
  useTabs.setState((state) => ({ closed: [...state.closed, closed].slice(-MAX_CLOSED) }));
}

/** Ctrl+Shift+T: the last closed tab, back where it was, with its history; it shows. */
export function reopenClosedTab(): Tab | undefined {
  const closed = useTabs.getState().closed.at(-1);
  if (!closed) {
    return undefined;
  }
  useTabs.setState((state) => ({ closed: state.closed.slice(0, -1) }));
  const tab = addTab(closed.pane, {
    index: closed.index,
    activate: true,
    pinned: closed.pinned,
    from: { look: closed.look, history: closed.history, index: closed.at },
  });
  const entry = tab.current();
  if (entry) {
    void tab.folder.open({ kind: "folder", folder: entry.folder }, { focus: entry.focus });
  }
  return tab;
}

/** "Close other tabs": every tab but `id` and the pinned ones. */
export function closeOtherTabs(pane: PaneSide, id: number): void {
  activateTab(pane, id);
  for (const tab of useTabs.getState()[pane].tabs) {
    if (tab.id !== id) closeTab(pane, tab.id);
  }
}

/** "Close tabs to the right": the unpinned tabs after `id`. */
export function closeTabsToRight(pane: PaneSide, id: number): void {
  const { tabs } = useTabs.getState()[pane];
  const index = tabs.findIndex((tab) => tab.id === id);
  if (index < 0) {
    return;
  }
  const { active } = useTabs.getState()[pane];
  if (tabs.slice(index + 1).some((tab) => tab.id === active)) {
    activateTab(pane, id);
  }
  for (const tab of tabs.slice(index + 1)) {
    closeTab(pane, tab.id);
  }
}

/** Pins or unpins `id`: it moves to the end of the pinned tabs, or to the front of the others. */
export function pinTab(pane: PaneSide, id: number, pinned: boolean): void {
  const { tabs } = useTabs.getState()[pane];
  const tab = tabs.find((each) => each.id === id);
  if (!tab || tab.pinned === pinned) {
    return;
  }
  const rest = tabs.filter((each) => each.id !== id);
  const pinnedCount = rest.filter((each) => each.pinned).length;
  setPane(pane, { tabs: insertAt(rest, pinnedCount, { id, pinned }) });
}

/**
 * Moves `id` to `index` in the strip (a drag); a tab stays in its group, pinned or not.
 */
export function moveTab(pane: PaneSide, id: number, index: number): void {
  const { tabs } = useTabs.getState()[pane];
  const tab = tabs.find((each) => each.id === id);
  if (!tab) {
    return;
  }
  const rest = tabs.filter((each) => each.id !== id);
  const pinnedCount = rest.filter((each) => each.pinned).length;
  const at = tab.pinned
    ? Math.min(Math.max(index, 0), pinnedCount)
    : Math.min(Math.max(index, pinnedCount), rest.length);
  setPane(pane, { tabs: pinnedFirst([...rest.slice(0, at), tab, ...rest.slice(at)]) });
}

/** "Duplicate": a copy of `id` right after it, with its history and look; the copy shows. */
export function duplicateTab(pane: PaneSide, id: number): Tab | undefined {
  const source = registry.get(id);
  const { tabs } = useTabs.getState()[pane];
  const index = tabs.findIndex((tab) => tab.id === id);
  const entry = source?.current();
  if (!source || index < 0 || !entry) {
    return undefined;
  }
  source.rememberFocus();
  const { history, index: at } = source.store.getState();
  const tab = addTab(pane, {
    index: index + 1,
    activate: true,
    from: { look: source.look(), history, index: at },
  });
  void tab.folder.open(
    { kind: "folder", folder: entry.folder },
    { filter: source.folder.store.getState().filter, focus: history[at]?.focus },
  );
  return tab;
}

/** Ctrl+Tab and Ctrl+Shift+Tab: the next or previous tab in `pane`, round the strip. */
export function stepTab(pane: PaneSide, step: 1 | -1): void {
  const { tabs, active } = useTabs.getState()[pane];
  const at = tabs.findIndex((tab) => tab.id === active);
  const next = tabs[(at + step + tabs.length) % tabs.length];
  if (next && tabs.length > 1) {
    activateTab(pane, next.id);
  }
}

/** Ctrl+1…8: tab `n` in `pane`, if there is one; Ctrl+9: the last tab, as in browsers. */
export function showTabNumber(pane: PaneSide, n: number): void {
  const { tabs } = useTabs.getState()[pane];
  const tab = n === 9 ? tabs.at(-1) : tabs[n - 1];
  if (tab) {
    activateTab(pane, tab.id);
  }
}

/** Tests only: closes every tab and forgets closed ones. */
export function resetTabs(): void {
  for (const tab of registry.values()) {
    tab.close();
  }
  registry.clear();
  lastId = 0;
  useTabs.setState({ left: noTabs, right: noTabs, closed: [] });
}
