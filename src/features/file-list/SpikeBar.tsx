import { useState } from "react";
import { strings } from "../../strings";
import { listElement, rowHeight } from "./FileList";
import { type FrameStats, scrollTest } from "./measure";
import { useListing, type WireFormat } from "./store";

const COUNTS = [1_000, 10_000, 100_000];

const ms = (value: number) => value.toFixed(1);

/** Spike #12 controls: open a synthetic folder, run a scroll test, show the last timings. */
export function SpikeBar() {
  const open = useListing((s) => s.open);
  const lastOpen = useListing((s) => s.lastOpen);
  const lastSort = useListing((s) => s.lastSort);
  const [format, setFormat] = useState<WireFormat>("binary");
  const [scroll, setScroll] = useState<FrameStats | null>(null);

  const runScroll = async () => {
    if (listElement.current) {
      setScroll(await scrollTest(listElement.current, rowHeight(), "linear"));
    }
  };

  return (
    <div className="flex shrink-0 flex-col gap-1 bg-bar px-3 py-2 text-small">
      <div className="flex items-center gap-2">
        {COUNTS.map((count) => (
          <button
            key={count}
            type="button"
            className="h-toolbar-btn rounded-ctrl border border-ctrl-border bg-ctrl px-3 hover:bg-hover"
            onClick={() => void open(count, format)}
          >
            {strings.spike.open(count)}
          </button>
        ))}
        <label className="flex items-center gap-1">
          <input
            type="checkbox"
            checked={format === "json"}
            onChange={(e) => setFormat(e.target.checked ? "json" : "binary")}
          />
          {strings.spike.json}
        </label>
        <button
          type="button"
          className="h-toolbar-btn rounded-ctrl border border-ctrl-border bg-ctrl px-3 hover:bg-hover"
          onClick={() => void runScroll()}
        >
          {strings.spike.scroll}
        </button>
      </div>
      <div className="text-fg-2 tabular-nums">
        {lastOpen &&
          `open ${lastOpen.count} (${lastOpen.format}): first rows ${ms(lastOpen.firstRowsMs)} ms · sorted ${ms(lastOpen.sortedPaintMs)} ms · rust list ${ms(lastOpen.rust?.listMs ?? 0)} ms`}
        {lastSort &&
          ` · sort ${lastSort.column}: ${ms(lastSort.totalMs)} ms (rust ${ms(lastSort.rustMs)})`}
        {scroll &&
          ` · scroll ${scroll.fps.toFixed(0)} fps, p95 ${ms(scroll.p95Ms)} ms, missed ${scroll.missed}`}
      </div>
    </div>
  );
}
