import {
  type CSSProperties,
  memo,
  type ReactNode,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import type { Settings } from "../ipc";
import { setSetting } from "../stores/settings";
import { strings } from "../strings";
import { dragPreview, dragSidebar, dragSplit, frameColumns, layoutZones } from "./layout";
import { Pane } from "./Pane";
import { Splitter } from "./Splitter";
import { StatusBar } from "./StatusBar";
import { TitleBar } from "./TitleBar";

interface Props {
  settings: Settings;
  /** Each pane's content: its file list. */
  left?: ReactNode;
  right?: ReactNode;
  /** The preview panel's content (dev builds: the dev panel, until previews come in v0.3). */
  previewContent?: ReactNode;
  /** The transfer panel's content; the 56 px zone shows only while a transfer runs. */
  transfers?: ReactNode;
}

/**
 * The window's zones, top to bottom: the 40 px title bar (each pane's tabs and the caption
 * buttons), the body (sidebar, two panes or one, preview), the transfer panel while a transfer
 * runs, and the status bar. The sidebar, the pane divider and the preview resize by drag; the
 * sizes are saved in the settings. In a narrow window the preview and then the sidebar hide
 * (`layout.ts`).
 */
export function Frame({ settings, left, right, previewContent, transfers }: Props) {
  const width = useWindowWidth();
  const sidebar = useDraggedSize("sidebarWidth", settings.sidebarWidth);
  const preview = useDraggedSize("previewWidth", settings.previewWidth);
  const split = useDraggedSize("paneSplit", settings.paneSplit);
  const zones = layoutZones(
    width,
    { sidebar: sidebar.value, preview: preview.value, split: split.value },
    settings.dualPane,
  );
  const style = { "--frame-columns": frameColumns(zones) } as CSSProperties;

  return (
    <div className="flex h-full flex-col bg-mica text-fg" style={style}>
      <TitleBar sidebar={zones.sidebar > 0} preview={zones.preview > 0} dual={zones.dual} />
      <div className="frame-columns grid min-h-0 flex-1">
        {zones.sidebar > 0 && <Sidebar />}
        <Pane
          side="left"
          edge={zones.sidebar > 0}
          handle={
            zones.sidebar > 0 && (
              <Splitter
                label={strings.frame.resizeSidebar}
                className="-left-1"
                onDrag={(x) => sidebar.drag(dragSidebar(x, width, zones))}
                onDrop={sidebar.drop}
              />
            )
          }
        >
          {left}
        </Pane>
        {zones.dual && (
          <>
            <div className="relative bg-stroke">
              <Splitter
                label={strings.frame.resizePanes}
                className="-left-1"
                onDrag={(x) => split.drag(dragSplit(x, width, zones))}
                onDrop={split.drop}
              />
            </div>
            <Pane side="right">{right}</Pane>
          </>
        )}
        {zones.preview > 0 && (
          <Preview content={previewContent}>
            <Splitter
              label={strings.frame.resizePreview}
              className="-left-1"
              onDrag={(x) => preview.drag(dragPreview(x, width, zones))}
              onDrop={preview.drop}
            />
          </Preview>
        )}
      </div>
      {transfers && (
        <section
          aria-label={strings.frame.transfers}
          className="flex h-transfer shrink-0 items-center border-stroke border-t bg-bar"
        >
          {transfers}
        </section>
      )}
      <StatusBar density={settings.density} />
    </div>
  );
}

/** The sidebar on Mica; its favorites, places and drives arrive with #34. */
const Sidebar = memo(function Sidebar() {
  return <nav aria-label={strings.frame.sidebar} className="min-w-0" />;
});

/** The preview panel; it shows the selected file from v0.3. */
function Preview({ children, content }: { children: ReactNode; content: ReactNode }) {
  return (
    <aside
      aria-label={strings.frame.preview}
      className="relative flex min-h-0 min-w-0 flex-col border-stroke border-l bg-bar"
    >
      {children}
      <div className="flex h-11 shrink-0 items-center pr-2 pl-4 font-semibold text-fg-2 text-small">
        {strings.frame.preview}
      </div>
      {content && <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3">{content}</div>}
    </aside>
  );
}

type SizeKey = "sidebarWidth" | "previewWidth" | "paneSplit";

/**
 * A zone size the user can drag. While dragging, and until the saved value comes back from the
 * core (`settings:changed`), the dragged size shows; the drop saves it.
 */
function useDraggedSize(key: SizeKey, saved: number) {
  const [dragged, setDragged] = useState<number>();
  const latest = useRef<number | undefined>(undefined);

  // A saved value that arrives (this drop's, or another window's) replaces the dragged one.
  // biome-ignore lint/correctness/useExhaustiveDependencies: runs when the saved value changes.
  useEffect(() => {
    latest.current = undefined;
    setDragged(undefined);
  }, [saved]);

  return {
    value: dragged ?? saved,
    drag(value: number) {
      latest.current = value;
      setDragged(value);
    },
    drop() {
      const value = latest.current;
      if (value === undefined || value === saved) {
        latest.current = undefined;
        setDragged(undefined);
        return;
      }
      void setSetting(key, value);
    },
  };
}

/** The window's width in CSS pixels, following resizes. */
function useWindowWidth(): number {
  return useSyncExternalStore(subscribeResize, () => window.innerWidth);
}

function subscribeResize(onResize: () => void): () => void {
  window.addEventListener("resize", onResize);
  return () => window.removeEventListener("resize", onResize);
}
