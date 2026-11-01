import type { SortBy } from "../../ipc";
import { listElement, rowHeight } from "./FileList";
import {
  type FrameStats,
  measureRefresh,
  type ScrollMode,
  scrollTest,
  startFrameLog,
} from "./measure";
import { type OpenTimings, type SortTimings, useListing, type WireFormat } from "./store";

/** Spike #12 harness, driven over CDP (`Runtime.evaluate`) or from the spike bar. */
export interface SpikeApi {
  open: (count: number, format?: WireFormat) => Promise<OpenTimings>;
  sort: (column: SortBy, descending: boolean) => Promise<SortTimings | null>;
  scroll: (mode: ScrollMode, durationMs?: number, rowsPerFrame?: number) => Promise<FrameStats>;
  /** Frame log around an outside scroll (CDP `Input.synthesizeScrollGesture`). */
  frameLogStart: () => Promise<void>;
  frameLogStop: () => FrameStats | null;
  listRect: () => { x: number; y: number; width: number; height: number } | null;
}

declare global {
  interface Window {
    jerrySpike?: SpikeApi;
  }
}

let frameLog: { stop: () => FrameStats } | null = null;

export function installSpikeApi(): void {
  window.jerrySpike = {
    open: (count, format) => useListing.getState().open(count, format),
    sort: (column, descending) => useListing.getState().sortBy(column, descending),
    scroll: async (mode, durationMs, rowsPerFrame) => {
      const element = listElement.current;
      if (!element) {
        throw new Error("no list");
      }
      return scrollTest(element, rowHeight(), mode, durationMs, rowsPerFrame);
    },
    frameLogStart: async () => {
      listElement.current?.scrollTo({ top: 0 });
      frameLog = startFrameLog(await measureRefresh());
    },
    frameLogStop: () => {
      const stats = frameLog?.stop() ?? null;
      frameLog = null;
      return stats;
    },
    listRect: () => {
      const rect = listElement.current?.getBoundingClientRect();
      return rect ? { x: rect.x, y: rect.y, width: rect.width, height: rect.height } : null;
    },
  };
}
