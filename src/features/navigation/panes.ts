import { useEffect } from "react";
import { setSetting, useSettings } from "../../stores/settings";
import { activeTab, type Tab } from "../../stores/tabs";
import { activatePane, type PaneSide, useWindow } from "../../stores/window";
import { focusList } from "../filter/focus";

/** The pane across from `pane`. */
export const otherPane = (pane: PaneSide): PaneSide => (pane === "left" ? "right" : "left");

/** Two panes show: the "Dual pane" setting, on by default. */
export const isDual = () => useSettings.getState().settings?.dualPane ?? true;

/**
 * F3 and the status bar's switch: one pane or two. The panes keep their tabs either way; with
 * one pane the left one shows and is the active one (`useSinglePane`).
 */
export function toggleDualPane(): void {
  const settings = useSettings.getState().settings;
  if (settings) {
    void setSetting("dualPane", !settings.dualPane);
  }
}

/**
 * Tab: the other pane becomes the active one and its list takes the keyboard. Only from a file
 * list (or with nothing focused), so Tab still moves through the toolbar's controls.
 */
export function switchPane(): void {
  const pane = otherPane(useWindow.getState().activePane);
  activatePane(pane);
  focusList(pane);
}

export function canSwitchPane(): boolean {
  return (
    isDual() && (nothingFocused() || document.activeElement?.closest("[data-file-list]") != null)
  );
}

/** The keyboard is nowhere in particular: its control went away, or nothing took it yet. */
export function nothingFocused(): boolean {
  const focus = document.activeElement;
  return focus === null || focus === document.body;
}

/**
 * With one pane, the left one is the active one, and its list takes the keyboard if the right
 * one had it. Use once, in the frame.
 */
export function useSinglePane(dual: boolean): void {
  useEffect(() => {
    if (!dual && useWindow.getState().activePane === "right") {
      activatePane("left");
      if (nothingFocused()) {
        focusList("left");
      }
    }
  }, [dual]);
}

/**
 * "Same folder in other pane": the other pane's tab goes to `tab`'s folder (a new step in its
 * history). The active pane stays as it is, as in two-pane file managers.
 */
export function openInOtherPane(pane: PaneSide, tab: Tab): void {
  const current = tab.current();
  const other = activeTab(otherPane(pane));
  if (!current || !other) {
    return;
  }
  // Search results: the same search there.
  if (current.search) {
    void other.search(current.search);
  } else {
    void other.open({ kind: "folder", folder: current.folder });
  }
}

/** The command: from the active pane's tab. */
export function sameFolderInOtherPane(): void {
  const pane = useWindow.getState().activePane;
  const tab = activeTab(pane);
  if (tab) {
    openInOtherPane(pane, tab);
  }
}

export const canOpenInOtherPane = () =>
  isDual() && activeTab(useWindow.getState().activePane)?.current() !== undefined;
