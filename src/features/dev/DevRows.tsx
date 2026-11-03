import { memo, type RefObject, useState } from "react";
import { entryIconUrl } from "../../ipc";
import { sizeText, strings } from "../../strings";

/** Icons are 16 px (design: «Icons»). */
const ICON_PX = 16;
/** Rows the view shows at once, and rows rendered past each edge. */
export const VIEW_ROWS = 16;
const OVERSCAN = 8;

/** A listing's entries as columns, filled batch by batch. */
export interface DevEntries {
  /** The listing's id, once `listing_open` has answered: entries with their own icon need it. */
  listing?: number;
  names: string[];
  sizes: number[];
  modified: number[];
  folders: boolean[];
  iconKeys: number[];
  /** Entry ids in display order, once the core has sorted them; until then, listing order. */
  order?: Uint32Array;
}

export function emptyEntries(): DevEntries {
  return { names: [], sizes: [], modified: [], folders: [], iconKeys: [] };
}

/**
 * Dev builds only: every entry of a listing in a virtualized list with fixed row heights, each
 * with its `icon://` icon, to check icons and measure scrolling. Not the file list (#29).
 */
export function DevRows({
  entries,
  count,
  rowHeight,
  viewport,
}: {
  entries: DevEntries;
  /** Entries in so far; a change re-renders. */
  count: number;
  rowHeight: number;
  viewport: RefObject<HTMLDivElement | null>;
}) {
  const [scrollTop, setScrollTop] = useState(0);
  const first = Math.max(0, Math.floor(scrollTop / rowHeight) - OVERSCAN);
  const last = Math.min(count, Math.ceil(scrollTop / rowHeight) + VIEW_ROWS + OVERSCAN);
  const rows = [];
  for (let position = first; position < last; position++) {
    const id = entries.order?.[position] ?? position;
    rows.push(
      <DevRow
        key={id}
        top={position * rowHeight}
        name={entries.names[id] ?? ""}
        size={entries.folders[id] ? undefined : (entries.sizes[id] ?? 0)}
        icon={iconFor(entries, id)}
      />,
    );
  }
  return (
    <div
      ref={viewport}
      onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
      className="overflow-y-auto rounded-ctrl border border-ctrl-border"
      style={{ height: VIEW_ROWS * rowHeight }}
    >
      <ul
        aria-label={strings.dev.entries}
        className="relative"
        style={{ height: count * rowHeight }}
      >
        {rows}
      </ul>
    </div>
  );
}

function iconFor(entries: DevEntries, id: number): string | undefined {
  const key = entries.iconKeys[id] ?? 0;
  const modified = entries.modified[id] ?? 0;
  if (entries.listing === undefined && key === 0) {
    return undefined;
  }
  return entryIconUrl({ listing: entries.listing ?? 0, id, key, modified }, ICON_PX);
}

const DevRow = memo(function DevRow({
  top,
  name,
  size,
  icon,
}: {
  top: number;
  name: string;
  size: number | undefined;
  icon: string | undefined;
}) {
  return (
    <li
      aria-label={name}
      className="absolute inset-x-0 flex h-row items-center gap-2 px-2"
      style={{ transform: `translateY(${top}px)` }}
    >
      {icon ? (
        <img src={icon} alt="" draggable={false} className="size-4 shrink-0" />
      ) : (
        <span className="size-4 shrink-0" />
      )}
      <span className="min-w-0 flex-1 truncate">{name}</span>
      <span className="text-fg-2 tabular-nums">{size === undefined ? "" : sizeText(size)}</span>
    </li>
  );
});
