import { create } from "zustand";
import { commands } from "../../ipc";
import type { Folder } from "../../stores/folder";
import { openInNewTab, type Tab } from "../../stores/tabs";
import type { PaneSide } from "../../stores/window";

/** Opening more items than this at once asks first, as Explorer does. */
export const MANY_ITEMS = 15;

interface OpenManyState {
  /** Items waiting for the user's yes: how many, and what opens them. */
  pending: { count: number; run: () => void } | undefined;
}

/** The question before opening many items (`OpenManyDialog`). */
export const useOpenMany = create<OpenManyState>(() => ({ pending: undefined }));

/**
 * What Enter opens in `folder`'s list: the selected rows in display order, or else the focused
 * one. Only rows the list shows (`rows`), so nothing hidden or filtered out opens.
 */
export function chosenItems(folder: Folder, rows: Uint32Array): number[] {
  const { selected, focused } = folder.store.getState();
  if (selected.size > 0) {
    return Array.from(rows).filter((id) => selected.has(id));
  }
  return focused !== undefined && rows.includes(focused) ? [focused] : [];
}

/**
 * Opens entries `ids` of `tab`'s folder, as Explorer does: one folder opens in the tab itself;
 * otherwise folders open in new tabs in the background and files in their programs (Windows'
 * default). `newTab` (Ctrl+Enter, a middle click) opens a folder in a new tab even alone. More
 * than `MANY_ITEMS` at once asks first.
 */
export function openItems(
  pane: PaneSide,
  tab: Tab,
  ids: readonly number[],
  { newTab = false }: { newTab?: boolean } = {},
): void {
  const { entries, listing } = tab.folder.store.getState();
  if (listing === undefined || ids.length === 0) {
    return;
  }
  const run = () => {
    const folders = ids.filter((id) => entries.isFolder(id));
    const files = ids.filter((id) => !entries.isFolder(id));
    const [only] = folders;
    if (!newTab && ids.length === 1 && only !== undefined) {
      void tab.open({ kind: "entry", listing, entry: only });
      return;
    }
    for (const id of folders) {
      openInNewTab(pane, { kind: "entry", listing, entry: id }, { activate: false });
    }
    if (files.length > 0) {
      // Windows says itself when a file can't be opened, over Jerry's window, as for Explorer.
      void commands.openEntries(listing, files);
    }
  };
  if (ids.length > MANY_ITEMS) {
    useOpenMany.setState({ pending: { count: ids.length, run } });
  } else {
    run();
  }
}
