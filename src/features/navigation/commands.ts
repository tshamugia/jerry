import { activeTab, type Tab } from "../../stores/tabs";
import { useWindow } from "../../stores/window";
import { shownRows } from "../selection/commands";
import { chosenItems, openItems } from "./open";

/** The tab the keyboard and the toolbar act on: the active pane's. */
export function focusedTab(): Tab | undefined {
  return activeTab(useWindow.getState().activePane);
}

const state = () => focusedTab()?.store.getState();

export const canGoBack = () => (state()?.index ?? 0) > 0;
export const canGoForward = () => {
  const tab = state();
  return tab !== undefined && tab.index < tab.history.length - 1;
};
export const canGoUp = () => focusedTab()?.current()?.parent != null;
export const hasTab = () => focusedTab() !== undefined;

/** Alt+←, Backspace. */
export const goBack = () => focusedTab()?.back();
/** Alt+→. */
export const goForward = () => focusedTab()?.forward();
/** Alt+↑. */
export const goUp = () => focusedTab()?.up();
/** Ctrl+R. */
export const refresh = () => focusedTab()?.refresh();

/** Enter, or Ctrl+Enter (`newTab`): opens the selected rows, or else the focused one. */
export function openChosen(newTab = false): void {
  const pane = useWindow.getState().activePane;
  const tab = activeTab(pane);
  if (tab) {
    openItems(pane, tab, chosenItems(tab.folder, shownRows(tab.folder)), { newTab });
  }
}
