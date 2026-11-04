import { memo } from "react";
import { TabStrip } from "../features/tabs/TabStrip";
import { strings } from "../strings";
import { CaptionButtons } from "./CaptionButtons";

interface Props {
  /** The sidebar shows: the app mark and name sit over it. */
  sidebar: boolean;
  /** The preview shows: the caption buttons sit over it. */
  preview: boolean;
  /** Two panes: the strip splits at their divider. */
  dual: boolean;
}

/**
 * The 40 px title bar on Mica. Its columns are the body's (`frame-columns`), so each pane's tabs
 * sit right over that pane and the strip splits at the pane divider, following it when it is
 * dragged; in single-pane mode there is one strip. The whole bar is a drag region: WebView2
 * moves the window, snaps it, maximizes it on a double-click and opens the system menu on a
 * right-click, natively. Tabs and buttons opt out; the caption buttons are handled natively.
 */
export const TitleBar = memo(function TitleBar({ sidebar, preview, dual }: Props) {
  // The caption buttons lie over the last column: the preview (always wider than them) or,
  // without it, the last strip, which keeps its tabs clear of them.
  const last = preview ? "" : "pr-caption-btns";
  return (
    <header className="drag-region frame-columns relative grid h-titlebar shrink-0">
      {sidebar && (
        <div className="flex min-w-0 items-center gap-2.5 px-3.5 text-fg-2 text-small">
          <AppMark />
          <span className="truncate">{strings.appName}</span>
        </div>
      )}
      <TabStrip pane="left" className={dual ? "" : last} />
      {dual && (
        <>
          <div className="mt-2.5 bg-stroke" />
          <TabStrip pane="right" className={last} />
        </>
      )}
      {preview && <div />}
      <CaptionButtons />
    </header>
  );
});

/** Jerry's mark: a tile with two panes, in the app icon's blue (the accent is for selection). */
function AppMark() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" className="shrink-0" aria-hidden="true">
      <rect x="0.5" y="1.5" width="15" height="13" rx="3" className="fill-brand" />
      <rect x="3" y="4" width="4.3" height="8" rx="1" className="fill-on-brand" />
      <rect x="8.7" y="4" width="4.3" height="8" rx="1" className="fill-on-brand" opacity="0.7" />
    </svg>
  );
}
