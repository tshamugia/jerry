import { create } from "zustand";
import { commands, type SavedSearch } from "../../ipc";
import type { SearchSpec } from "../../stores/folder";
import { closeOverlay, openOverlay } from "../../stores/overlay";
import { activeTab, openInNewTab, searchInNewTab } from "../../stores/tabs";
import { useWindow } from "../../stores/window";
import { focusListSoon } from "../filter/focus";
import { chosenItems } from "../navigation/open";
import { activeFolder, shownRows } from "../selection/commands";
import { searchKey } from "./query";

/**
 * Deep search (PRD «ღრმა ძებნა», #66): Ctrl+Shift+F opens the search window on the active pane's
 * folder; a search opens its results in a tab of their own, next to the one it started from, or
 * runs again in place from a results tab. The last 20 searches are kept (`search_history`).
 */

/** Searches the history shows, at most (the core keeps as many). */
export const HISTORY = 20;

/** Where the search window searches, and from which tab. */
export interface SearchPlace {
  /** The folder searched from, as the core's token. */
  folder: number;
  /** Its name, for "Projects and subfolders". */
  name: string;
  /** Its drive or share, for "Whole drive D:". */
  drive: string;
  /** The search a results tab shows, to edit; it runs again in that tab. */
  editing: SavedSearch | undefined;
}

/** What the search window searches: the active pane's folder, or its results tab's search. */
export function searchPlace(): SearchPlace | undefined {
  const tab = activeTab(useWindow.getState().activePane);
  const entry = tab?.current();
  if (!entry) {
    return undefined;
  }
  if (entry.search) {
    const { folder, name, drive, search } = entry.search;
    return { folder, name, drive, editing: search };
  }
  const drive = entry.crumbs[0]?.name ?? entry.path;
  return { folder: entry.folder, name: entry.name, drive, editing: undefined };
}

/** Ctrl+Shift+F: the search window, once the active pane shows a folder. */
export function openSearch(): void {
  openOverlay("search");
}

export const canSearch = () => searchPlace() !== undefined;

/**
 * Runs `search` from `place`: in the results tab it edits, or in a new tab after the active one.
 * It goes to the top of the history.
 */
export function runSearch(search: SavedSearch, place: SearchPlace): void {
  const pane = useWindow.getState().activePane;
  const spec: SearchSpec = { search, folder: place.folder, name: place.name, drive: place.drive };
  const tab = activeTab(pane);
  if (place.editing && tab?.current()?.search) {
    void tab.search(spec);
  } else {
    searchInNewTab(pane, spec);
  }
  closeOverlay();
  focusListSoon(pane);
  rememberSearch(search);
}

/** Whether the active pane shows a search that still runs. */
export function canStopSearch(): boolean {
  return activeFolder()?.store.getState().search?.phase === "searching";
}

/** Stop: the active pane's search ends with what it found. */
export function stopSearch(): void {
  activeFolder()?.stopSearch();
}

/** Whether the active pane shows search results with rows to open the folders of. */
export function canOpenContainingFolder(): boolean {
  const folder = activeFolder();
  const state = folder?.store.getState();
  return (
    folder !== undefined &&
    state?.search !== undefined &&
    state.listing !== undefined &&
    chosenItems(folder, shownRows(folder)).length > 0
  );
}

/**
 * Open file location (Ctrl+Shift+E): the folder of each chosen result in a new tab, with the
 * results in it selected and the first one focused. The first folder's tab shows; the others
 * open behind it.
 */
export function openContainingFolder(): void {
  const pane = useWindow.getState().activePane;
  const folder = activeFolder();
  const state = folder?.store.getState();
  if (!folder || state?.listing === undefined) {
    return;
  }
  const { listing, entries } = state;
  const byFolder = new Map<string, number[]>();
  for (const id of chosenItems(folder, shownRows(folder))) {
    const location = entries.location(id) ?? "";
    byFolder.set(location, [...(byFolder.get(location) ?? []), id]);
  }
  let first = true;
  for (const ids of byFolder.values()) {
    const [entry] = ids;
    if (entry === undefined) {
      continue;
    }
    const names = ids.map((id) => entries.names[id] ?? "");
    openInNewTab(
      pane,
      { kind: "containing", listing, entry },
      { activate: first, restore: { focus: names[0], selected: names } },
    );
    first = false;
  }
  focusListSoon(pane);
}

interface HistoryState {
  /** The last searches, newest first; undefined until the core answers. */
  searches: readonly SavedSearch[] | undefined;
}

/** The search history, as the search window lists it. */
export const useSearchHistory = create<HistoryState>(() => ({ searches: undefined }));

/** Reads the history from the core: when the search window opens. */
export async function loadSearchHistory(): Promise<void> {
  const result = await commands.searchHistoryList();
  if (result.status === "ok") {
    useSearchHistory.setState({ searches: result.data });
  }
}

/** `search` goes to the top of the history, here at once and in the core. */
function rememberSearch(search: SavedSearch): void {
  const key = searchKey(search);
  useSearchHistory.setState(({ searches = [] }) => ({
    searches: [search, ...searches.filter((each) => searchKey(each) !== key)].slice(0, HISTORY),
  }));
  void commands.searchHistoryAdd(search);
}

/** Takes `search` out of the history. */
export function forgetSearch(search: SavedSearch): void {
  const key = searchKey(search);
  useSearchHistory.setState(({ searches = [] }) => ({
    searches: searches.filter((each) => searchKey(each) !== key),
  }));
  void commands.searchHistoryRemove(search);
}
