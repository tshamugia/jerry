import { strings } from "../strings";
import { CaptionButtons } from "./CaptionButtons";
import type { PaneTabs } from "./mockTabs";
import { TabStrip } from "./TabStrip";
import type { Frame } from "./useFrame";

export type Pane = "left" | "right";

type Props = {
  frame: Frame;
  tabs: Record<Pane, PaneTabs>;
  activePane: Pane;
  onActivate: (pane: Pane, id: number) => void;
  onClose: (pane: Pane, id: number) => void;
  onNew: (pane: Pane) => void;
};

/**
 * The 40 px title bar on Mica. Its columns are the body's (`frame-columns`), so each pane's tabs
 * sit right above that pane and the strip splits at the pane divider. The whole bar is a drag
 * region; tabs and buttons opt out, and the caption buttons are handled natively.
 */
export function TitleBar({ frame, tabs, activePane, onActivate, onClose, onNew }: Props) {
  const strip = (pane: Pane) => (
    <TabStrip
      label={pane === "left" ? strings.titleBar.leftTabs : strings.titleBar.rightTabs}
      tabs={tabs[pane]}
      activePane={activePane === pane}
      onActivate={(id) => onActivate(pane, id)}
      onClose={(id) => onClose(pane, id)}
      onNew={() => onNew(pane)}
    />
  );

  return (
    <header className="drag-region frame-columns grid h-titlebar shrink-0 bg-mica">
      <div className="flex items-center gap-2.5 px-3.5">
        <AppMark />
        <span className="text-small text-fg-2">{strings.appName}</span>
      </div>
      {strip("left")}
      <div className="mt-2.5 bg-stroke" />
      {strip("right")}
      <div className="flex min-w-0 justify-end">
        <CaptionButtons state={frame.state} />
      </div>
    </header>
  );
}

function AppMark() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" className="shrink-0" aria-hidden="true">
      <rect x="0.5" y="1.5" width="15" height="13" rx="3" className="fill-accent" />
      <rect x="3" y="4" width="4.3" height="8" rx="1" className="fill-on-accent" />
      <rect x="8.7" y="4" width="4.3" height="8" rx="1" className="fill-on-accent" opacity="0.7" />
    </svg>
  );
}
