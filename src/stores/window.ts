import { create } from "zustand";

export type PaneSide = "left" | "right";

interface WindowState {
  /** The pane the toolbar buttons and the keyboard act on; it gets the accent bar. */
  activePane: PaneSide;
}

/** This window's own state. Panes get their tabs with #23 and #29. */
export const useWindow = create<WindowState>(() => ({ activePane: "left" }));

export function activatePane(pane: PaneSide) {
  useWindow.setState({ activePane: pane });
}
