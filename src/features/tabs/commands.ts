import {
  activeTab,
  closeTab,
  openInNewTab,
  reopenClosedTab,
  showTabNumber,
  stepTab,
  useTabs,
} from "../../stores/tabs";
import { activatePane, type PaneSide, useWindow } from "../../stores/window";
import { focusListSoon } from "../filter/focus";

const pane = () => useWindow.getState().activePane;

/**
 * Ctrl+T and the + button: a new tab after `inPane`'s active one, on the same folder, which it
 * shows.
 * There is no empty "new tab" page: a file manager's tab is always somewhere.
 */
export function newTab(inPane: PaneSide = pane()): void {
  const current = activeTab(inPane)?.current();
  openInNewTab(inPane, current ? { kind: "folder", folder: current.folder } : { kind: "home" });
  activatePane(inPane);
  focusListSoon(inPane);
}

/** Ctrl+W: the active pane's tab, unless it is pinned or the pane's last one. */
export function closeActiveTab(): void {
  const { active } = useTabs.getState()[pane()];
  if (active !== undefined) {
    closeTab(pane(), active);
  }
}

export const canCloseTab = () => {
  const { tabs, active } = useTabs.getState()[pane()];
  return tabs.length > 1 && tabs.some((tab) => tab.id === active && !tab.pinned);
};

/** Ctrl+Shift+T: the last closed tab, in its own pane, which becomes active. */
export function reopenTab(): void {
  const closed = useTabs.getState().closed.at(-1);
  if (closed && reopenClosedTab()) {
    activatePane(closed.pane);
  }
}

export const canReopenTab = () => useTabs.getState().closed.length > 0;

/** Ctrl+Tab. */
export const nextTab = () => stepTab(pane(), 1);
/** Ctrl+Shift+Tab. */
export const previousTab = () => stepTab(pane(), -1);
/** Ctrl+1…9. */
export const showTab = (n: number) => () => showTabNumber(pane(), n);
