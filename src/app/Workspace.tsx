import type { PointerEvent, Ref } from "react";
import { useRef } from "react";
import { FileList } from "../features/file-list/FileList";
import { useListing } from "../features/file-list/store";
import { Readout } from "../features/georgian/Readout";
import { Breadcrumb } from "../features/navigation/Breadcrumb";
import { strings } from "../strings";
import { mockPlaces, mockRightPath } from "./mockTabs";
import type { Pane } from "./TitleBar";

type Props = {
  /** The Georgian folder's display path (left pane). */
  path: string;
  activePane: Pane;
  onActivatePane: (pane: Pane) => void;
  /** The left pane's share of the two panes, 0..1. */
  onSplit: (share: number) => void;
  /** Receives each pane's quick filter field, for Ctrl+F. */
  filterRef: (pane: Pane) => Ref<HTMLInputElement>;
};

/**
 * The body under the title bar for spike #18: sidebar on Mica, two panes (breadcrumb, quick
 * filter and the Georgian folder's list) split by a draggable divider, and the spike readout in
 * the preview column.
 */
export function Workspace({ path, activePane, onActivatePane, onSplit, filterRef }: Props) {
  const body = useRef<HTMLDivElement>(null);

  const drag = (e: PointerEvent<HTMLDivElement>) => {
    const panes = body.current?.children;
    const left = panes?.[1]?.getBoundingClientRect();
    const right = panes?.[3]?.getBoundingClientRect();
    if (!left || !right || !(e.buttons & 1)) return;
    const share = (e.clientX - left.left) / (right.right - left.left);
    onSplit(Math.min(0.8, Math.max(0.2, share)));
  };

  const pane = (which: Pane, panePath: string) => (
    <PaneView
      pane={which}
      path={panePath}
      active={activePane === which}
      onActivate={() => onActivatePane(which)}
      filterRef={filterRef(which)}
    />
  );

  return (
    <div ref={body} className="frame-columns grid min-h-0 flex-1">
      <nav data-zone="sidebar" className="flex flex-col gap-px px-2 pt-10">
        {mockPlaces.map((place) => (
          <div
            key={place}
            className="flex h-nav items-center rounded-ctrl px-2 hover:bg-subtle-hover"
          >
            <span data-name className="truncate">
              {place}
            </span>
          </div>
        ))}
      </nav>
      {pane("left", path)}
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
      {pane("right", mockRightPath)}
      <aside className="flex min-h-0 flex-col gap-4 overflow-y-auto bg-bar p-4">
        <Readout />
      </aside>
    </div>
  );
}

type PaneProps = {
  pane: Pane;
  path: string;
  active: boolean;
  onActivate: () => void;
  filterRef: Ref<HTMLInputElement>;
};

function PaneView({ pane, path, active, onActivate, filterRef }: PaneProps) {
  const filter = useListing((s) => s.filter);
  const setFilter = useListing((s) => s.setFilter);
  return (
    // Focusing anything in a pane (or clicking it) makes it the active pane.
    <section
      data-pane={pane}
      aria-label={pane === "left" ? strings.pane.leftPane : strings.pane.rightPane}
      tabIndex={-1}
      onFocus={onActivate}
      className="flex min-w-0 flex-col outline-none"
    >
      <div className="flex flex-col gap-1.5 bg-bar px-2 pt-1.5 pb-2">
        <Breadcrumb path={path} />
        <input
          ref={filterRef}
          data-filter
          type="search"
          value={filter}
          placeholder={strings.pane.filter}
          aria-label={strings.pane.filter}
          onChange={(e) => setFilter(e.target.value)}
          onKeyDown={(e) => {
            if (e.code === "Escape") setFilter("");
          }}
          className="h-filter w-56 rounded-ctrl border border-ctrl-border bg-ctrl px-2 text-fg placeholder:text-fg-3"
        />
      </div>
      <div className={`h-0.5 ${active ? "bg-accent" : "bg-bar"}`} />
      <FileList />
    </section>
  );
}
