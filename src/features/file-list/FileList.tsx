import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useLayoutEffect, useRef, useState } from "react";
import type { SortBy } from "../../ipc";
import { strings } from "../../strings";
import { stamp, stampNextFrame } from "../startup/stamps";
import { ATTRIBUTE_DIRECTORY } from "./columns";
import { formatDate, formatSize, formatType } from "./format";
import { useListing } from "./store";

/** Row height in px from the density token (`--jerry-row-h`); TanStack Virtual needs a number. */
export function rowHeight(): number {
  const value = getComputedStyle(document.documentElement).getPropertyValue("--jerry-row-h");
  return Number.parseFloat(value) || 30;
}

/** A minimal Details view: virtualized, fixed row height, memoized rows. */
export function FileList() {
  const count = useListing((s) => s.count);
  // Columns are mutated in place; `version` re-renders the list on every change.
  useListing((s) => s.version);
  const order = useListing((s) => s.order);
  const columns = useListing((s) => s.columns);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [height] = useState(rowHeight);

  const virtualizer = useVirtualizer({
    count,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => height,
    overscan: 8,
  });

  // Spike #11: the first commit that has rows, and the frame after it.
  const hasRows = count > 0;
  useLayoutEffect(() => {
    if (hasRows) {
      stamp("rows:commit");
      stampNextFrame("rows");
    }
  }, [hasRows]);

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-pane">
      <Header />
      <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto">
        <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((item) => {
            const id = order ? (order[item.index] ?? item.index) : item.index;
            return (
              <Row
                key={item.index}
                first={item.index === 0}
                top={item.start}
                name={columns.names[id] ?? ""}
                size={columns.sizes[id] ?? 0}
                modified={columns.modified[id] ?? 0}
                attributes={columns.attributes[id] ?? 0}
              />
            );
          })}
        </div>
      </div>
      <StatusBar />
    </div>
  );
}

interface RowProps {
  /** The first row carries an Element Timing id (spike #11 measures when it is painted). */
  first: boolean;
  top: number;
  name: string;
  size: number;
  modified: number;
  attributes: number;
}

const Row = memo(function Row({ first, top, name, size, modified, attributes }: RowProps) {
  const isDir = (attributes & ATTRIBUTE_DIRECTORY) !== 0;
  return (
    <div
      className="absolute inset-x-1 top-0 flex h-row items-center gap-2 rounded-ctrl px-2 hover:bg-hover"
      style={{ transform: `translateY(${top}px)` }}
    >
      <span className="min-w-0 flex-1 truncate" ref={first ? markFirstRow : undefined}>
        {name}
      </span>
      <span className="w-col-date shrink-0 text-fg-2 tabular-nums">{formatDate(modified)}</span>
      <span className="w-col-type shrink-0 truncate text-fg-2">{formatType(name, isDir)}</span>
      <span className="w-col-size shrink-0 text-right text-fg-2 tabular-nums">
        {isDir ? "" : formatSize(size)}
      </span>
    </div>
  );
});

/** Element Timing reports when this element is first painted (`element:first-row`). */
function markFirstRow(element: HTMLSpanElement | null): void {
  element?.setAttribute("elementtiming", "first-row");
}

const HEADER: { column: SortBy; label: string; className: string }[] = [
  { column: "name", label: strings.fileList.name, className: "min-w-0 flex-1 text-left" },
  { column: "modified", label: strings.fileList.modified, className: "w-col-date text-left" },
  { column: "type", label: strings.fileList.type, className: "w-col-type text-left" },
  { column: "size", label: strings.fileList.size, className: "w-col-size text-right" },
];

function Header() {
  const column = useListing((s) => s.column);
  const descending = useListing((s) => s.descending);
  const sortBy = useListing((s) => s.sortBy);
  return (
    <div className="mx-1 flex h-col-header shrink-0 items-center gap-2 border-stroke border-b px-2 text-fg-2 text-small">
      {HEADER.map((h) => (
        <button
          key={h.column}
          type="button"
          className={`shrink-0 truncate hover:text-fg ${h.className}`}
          onClick={() => void sortBy(h.column)}
        >
          {h.label}
          {column === h.column ? (descending ? " ▾" : " ▴") : ""}
        </button>
      ))}
    </div>
  );
}

function StatusBar() {
  const count = useListing((s) => s.count);
  const loading = useListing((s) => s.loading);
  const error = useListing((s) => s.error);
  return (
    <div className="flex h-statusbar shrink-0 items-center gap-4 border-stroke border-t px-3 text-fg-2 text-small tabular-nums">
      <span>{strings.fileList.items(count)}</span>
      {loading && <span>{strings.fileList.loading}</span>}
      {error && <span className="text-red">{strings.fileList.error(error)}</span>}
    </div>
  );
}
