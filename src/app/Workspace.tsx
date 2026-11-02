import { type PointerEvent, useEffect, useReducer, useRef } from "react";
import { strings } from "../strings";
import { mockPlaces, type PaneTabs } from "./mockTabs";
import type { Pane } from "./TitleBar";
import type { Frame } from "./useFrame";

type Props = {
  frame: Frame;
  tabs: Record<Pane, PaneTabs>;
  activePane: Pane;
  onActivatePane: (pane: Pane) => void;
  /** The left pane's share of the two panes, 0..1. */
  onSplit: (share: number) => void;
};

/**
 * The body under the title bar, mocked for spike #17: sidebar on Mica, two panes split by a
 * draggable divider (the tab strip above follows it), and a preview column with the frame
 * readout used for the manual checks.
 */
export function Workspace({ frame, tabs, activePane, onActivatePane, onSplit }: Props) {
  const body = useRef<HTMLDivElement>(null);

  const drag = (e: PointerEvent<HTMLDivElement>) => {
    const panes = body.current?.children;
    const left = panes?.[1]?.getBoundingClientRect();
    const right = panes?.[3]?.getBoundingClientRect();
    if (!left || !right || !(e.buttons & 1)) return;
    const share = (e.clientX - left.left) / (right.right - left.left);
    onSplit(Math.min(0.8, Math.max(0.2, share)));
  };

  return (
    <div ref={body} className="frame-columns grid min-h-0 flex-1">
      <nav className="flex flex-col gap-px px-2 pt-10">
        {mockPlaces.map((place) => (
          <div
            key={place}
            className="flex h-nav items-center rounded-ctrl px-2 hover:bg-subtle-hover"
          >
            {place}
          </div>
        ))}
      </nav>
      <PaneMock
        name={activeName(tabs.left)}
        active={activePane === "left"}
        onActivate={() => onActivatePane("left")}
      />
      {/* biome-ignore lint/a11y/useSemanticElements: a draggable window splitter; <hr> cannot be one. */}
      <div
        role="separator"
        aria-orientation="vertical"
        className="relative bg-stroke"
        onPointerDown={(e) => e.currentTarget.setPointerCapture(e.pointerId)}
        onPointerMove={drag}
      >
        <div className="absolute inset-y-0 -left-1 w-2.25 cursor-col-resize" />
      </div>
      <PaneMock
        name={activeName(tabs.right)}
        active={activePane === "right"}
        onActivate={() => onActivatePane("right")}
      />
      <aside className="flex flex-col gap-3 bg-bar p-4">
        <SpikeReadout frame={frame} />
      </aside>
    </div>
  );
}

function activeName(pane: PaneTabs): string {
  return pane.tabs.find((t) => t.id === pane.activeId)?.name ?? "";
}

function PaneMock({
  name,
  active,
  onActivate,
}: {
  name: string;
  active: boolean;
  onActivate: () => void;
}) {
  return (
    // Focusing anything in a pane (or clicking it) makes it the active pane.
    // biome-ignore lint/a11y/noStaticElementInteractions: mock pane; real panes get focus handling in #22.
    <section tabIndex={-1} onFocus={onActivate} className="flex min-w-0 flex-col outline-none">
      <div className="flex flex-col gap-1.5 bg-bar px-2 pt-1.5 pb-2">
        <div className="flex h-address items-center rounded-ctrl border border-ctrl-border bg-ctrl px-2.5">
          <span className="truncate font-semibold">{name}</span>
        </div>
        <div className="h-filter w-48 rounded-ctrl border border-ctrl-border bg-ctrl" />
      </div>
      <div className={`h-0.5 ${active ? "bg-accent" : "bg-bar"}`} />
      <div className="flex-1 bg-pane" />
    </section>
  );
}

function SpikeReadout({ frame }: { frame: Frame }) {
  const s = strings.spike;
  useViewportChanges();
  const yesNo = (value: boolean | undefined) => (value === undefined ? "…" : value ? s.yes : s.no);
  const rows: [string, string][] = [
    [s.hovered, frame.state.hovered ?? s.none],
    [s.pressed, frame.state.pressed ?? s.none],
    [s.maximized, yesNo(frame.state.maximized)],
    [s.active, yesNo(frame.state.active)],
    [s.mica, yesNo(frame.info?.mica)],
    [s.snapLayouts, yesNo(frame.info?.snapLayouts)],
    [s.scale, `${window.devicePixelRatio * 100} %`],
    [s.size, `${document.documentElement.clientWidth} × ${document.documentElement.clientHeight}`],
  ];
  return (
    <>
      <h2 className="font-semibold text-title">{s.title}</h2>
      <dl className="grid grid-cols-2 gap-x-3 gap-y-1 tabular-nums">
        {rows.map(([term, value]) => (
          <div key={term} className="contents">
            <dt className="text-fg-2">{term}</dt>
            <dd className="m-0">{value}</dd>
          </div>
        ))}
      </dl>
      <p className="m-0 text-fg-3 text-small">{s.hint}</p>
    </>
  );
}

/** Re-renders on resize and on a scale change (moving to a monitor with another DPI). */
function useViewportChanges() {
  const [, rerender] = useReducer((n: number) => n + 1, 0);
  useEffect(() => {
    const query = () => window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
    let scale = query();
    const onScale = () => {
      rerender();
      scale.removeEventListener("change", onScale);
      scale = query();
      scale.addEventListener("change", onScale);
    };
    scale.addEventListener("change", onScale);
    window.addEventListener("resize", rerender);
    return () => {
      scale.removeEventListener("change", onScale);
      window.removeEventListener("resize", rerender);
    };
  }, []);
}
