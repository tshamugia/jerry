import type { Folder, FolderState, Selection } from "../../stores/folder";
import { useSettings } from "../../stores/settings";
import { activeTab } from "../../stores/tabs";
import { useWindow } from "../../stores/window";
import { clear, invert, selectAll, toggleFocused } from "./model";

/** The folder the keyboard acts on: the active pane's tab's. */
export function activeFolder(): Folder | undefined {
  return activeTab(useWindow.getState().activePane)?.folder;
}

/** The rows `folder`'s list shows now: what every selection command works on. */
export function shownRows(folder: Folder): Uint32Array {
  return folder.shown(useSettings.getState().settings?.showHidden ?? false).rows;
}

export function selectionOf({ selected, anchor, focused }: FolderState): Selection {
  return { selected, anchor, focused };
}

/** Changes the active pane's selection with `update`. */
function change(update: (selection: Selection, rows: Uint32Array) => Selection): void {
  const folder = activeFolder();
  folder?.setSelection(update(selectionOf(folder.store.getState()), shownRows(folder)));
}

/** Ctrl+A. */
export const selectAllRows = () => change(selectAll);
/** Ctrl+I. */
export const invertSelection = () => change(invert);
/** Ctrl+Space. */
export const toggleFocusedRow = () => change(toggleFocused);
/** Esc. */
export const clearSelection = () => change(clear);
