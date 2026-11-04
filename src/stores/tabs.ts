import { create } from "zustand";
import type { PaneSide } from "./window";

/** What a pane's tab strip shows of a tab. */
export interface TabView {
  id: number;
  /** The folder's name. */
  name: string;
  /** Pinned tabs are narrow, stay at the left and have no close button. */
  pinned: boolean;
}

export interface PaneTabs {
  /** Pinned tabs first. */
  tabs: TabView[];
  /** The tab the pane shows; `null` without tabs. */
  active: number | null;
}

type TabsState = Record<PaneSide, PaneTabs>;

const none: PaneTabs = { tabs: [], active: null };

/**
 * Each pane's tabs, as the title bar shows them. Tabs get their folders, history and views with
 * #31; until then the panes start without tabs.
 */
export const useTabs = create<TabsState>(() => ({ left: none, right: none }));

/** Replaces a pane's tabs; pinned ones move to the front. */
export function setTabs(pane: PaneSide, tabs: TabView[], active: number | null = null) {
  const ordered = [...tabs.filter((tab) => tab.pinned), ...tabs.filter((tab) => !tab.pinned)];
  const shown = ordered.some((tab) => tab.id === active) ? active : (ordered[0]?.id ?? null);
  useTabs.setState({ [pane]: { tabs: ordered, active: shown } });
}

/** Shows `id` in its pane. */
export function activateTab(pane: PaneSide, id: number) {
  const current = useTabs.getState()[pane];
  if (current.active !== id && current.tabs.some((tab) => tab.id === id)) {
    useTabs.setState({ [pane]: { ...current, active: id } });
  }
}

/**
 * Closes `id`; a pinned tab doesn't close by accident. If it was the shown tab, its right
 * neighbour shows, or else its left one.
 */
export function closeTab(pane: PaneSide, id: number) {
  const current = useTabs.getState()[pane];
  const index = current.tabs.findIndex((tab) => tab.id === id);
  if (index < 0 || current.tabs[index]?.pinned) {
    return;
  }
  const tabs = current.tabs.filter((tab) => tab.id !== id);
  const active =
    current.active === id ? ((tabs[index] ?? tabs[index - 1])?.id ?? null) : current.active;
  useTabs.setState({ [pane]: { tabs, active } });
}
