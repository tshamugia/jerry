import { Fragment } from "react";
import { strings } from "../../strings";

/** Segments shown before the leading ones collapse into "…". */
const MAX_SEGMENTS = 4;

/**
 * The address bar as path segments, the current folder in semibold (spike #18 version: display
 * only, no navigation). The path text comes from Rust and is never turned back into a path.
 */
export function Breadcrumb({ path }: { path: string }) {
  const segments = path.split("\\").filter(Boolean);
  const first = Math.max(0, segments.length - MAX_SEGMENTS);
  return (
    <nav
      aria-label={strings.pane.address}
      data-zone="breadcrumb"
      className="flex h-address min-w-0 flex-1 items-center overflow-hidden rounded-ctrl border border-ctrl-border bg-ctrl px-1.5"
    >
      {first > 0 && <span className="shrink-0 px-1 text-fg-2">…</span>}
      {segments.slice(first).map((segment, offset) => {
        const index = first + offset;
        const current = index === segments.length - 1;
        return (
          <Fragment key={segments.slice(0, index + 1).join("\\")}>
            {(index > 0 || first > 0) && <Chevron />}
            <span
              data-segment
              className={`truncate px-1 ${current ? "max-w-full shrink-0 font-semibold" : "min-w-0 text-fg-2"}`}
            >
              {segment}
            </span>
          </Fragment>
        );
      })}
    </nav>
  );
}

function Chevron() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 16 16"
      fill="none"
      className="shrink-0 text-fg-3"
      aria-hidden="true"
    >
      <path
        d="M6 3.5L10.5 8 6 12.5"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
