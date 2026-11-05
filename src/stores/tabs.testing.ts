import type { Folder } from "./folder";
import { activeTab, addTab, resetTabs, type Tab } from "./tabs";
import type { PaneSide } from "./window";

/** Tests only: closes every tab, then gives each pane one tab without a folder yet. */
export function paneTabs(): Record<PaneSide, Tab> {
  resetTabs();
  return { left: addTab("left"), right: addTab("right") };
}

/** Tests only: the folder of the tab `pane` shows. */
export function paneFolder(pane: PaneSide): Folder {
  const tab = activeTab(pane);
  if (!tab) {
    throw new Error(`the ${pane} pane has no tab: call paneTabs() first`);
  }
  return tab.folder;
}
