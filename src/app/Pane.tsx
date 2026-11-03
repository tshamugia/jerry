import { memo, type ReactNode } from "react";
import { activatePane, type PaneSide, useWindow } from "../stores/window";
import { strings } from "../strings";
import { PaneToolbar } from "./PaneToolbar";

interface Props {
  side: PaneSide;
  /** A 1 px line on the left edge: the left pane's, next to the sidebar. */
  edge?: boolean;
  /** The resize handle on the left edge (the sidebar's). */
  handle?: ReactNode;
  /** The pane's content (the file list, #29). */
  children?: ReactNode;
}

/**
 * One of the two panes: its toolbar, the 2 px bar under it (the accent on the active pane, the
 * toolbar's color on the other) and its content. Clicking or focusing anything in a pane makes it
 * the active pane.
 */
export const Pane = memo(function Pane({ side, edge = false, handle, children }: Props) {
  const active = useWindow((state) => state.activePane === side);
  const name = side === "left" ? strings.frame.leftPane : strings.frame.rightPane;
  return (
    <section
      aria-label={active ? strings.frame.activePane(name) : name}
      className={`relative flex min-w-0 flex-col bg-pane ${edge ? "border-l border-stroke" : ""}`}
      onPointerDown={() => activatePane(side)}
      onFocus={() => activatePane(side)}
    >
      {handle}
      <PaneToolbar />
      <div className={`h-active-pane-bar shrink-0 ${active ? "bg-accent" : "bg-bar"}`} />
      <div className="min-h-0 flex-1 overflow-auto">{children}</div>
    </section>
  );
});
