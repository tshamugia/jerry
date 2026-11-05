import { memo } from "react";
import { useActiveTab } from "../../stores/tabs";
import type { PaneSide } from "../../stores/window";
import { FileList } from "./FileList";

/**
 * The file list of the tab `pane` shows. Each tab shown gets a list of its own (`key`), which
 * scrolls back to where that tab was.
 */
export const PaneList = memo(function PaneList({ pane }: { pane: PaneSide }) {
  const tab = useActiveTab(pane);
  return tab ? <FileList key={tab.id} tab={tab} pane={pane} /> : null;
});
