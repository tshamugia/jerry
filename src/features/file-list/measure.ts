/**
 * Spike #12 measurement helpers: time to the next paint after a React commit, and a frame-time
 * logger for scrolling. Driven by hand from the spike bar or over CDP (`window.jerrySpike`).
 */

/** Resolves just after the next frame is painted: rAF runs before paint, the message after it. */
export function afterNextPaint(): Promise<number> {
  return new Promise((resolve) => {
    requestAnimationFrame(() => {
      const channel = new MessageChannel();
      channel.port1.onmessage = () => resolve(performance.now());
      channel.port2.postMessage(null);
    });
  });
}

const waiters: { version: number; resolve: (paintedAt: number) => void }[] = [];
let committedVersion = 0;

/** Called by the list in a layout effect once `version` is committed to the DOM. */
export function notifyCommitted(version: number): void {
  committedVersion = version;
  for (let i = waiters.length - 1; i >= 0; i--) {
    const waiter = waiters[i];
    if (waiter && waiter.version <= version) {
      waiters.splice(i, 1);
      void afterNextPaint().then(waiter.resolve);
    }
  }
}

/** Resolves with the time the first frame showing store `version` was painted. */
export function paintOf(version: number): Promise<number> {
  if (committedVersion >= version) {
    return afterNextPaint();
  }
  return new Promise((resolve) => waiters.push({ version, resolve }));
}

export interface FrameStats {
  frames: number;
  durationMs: number;
  fps: number;
  /** The display's own frame interval, measured before the run. */
  refreshMs: number;
  p50Ms: number;
  p95Ms: number;
  p99Ms: number;
  maxMs: number;
  /** Frames that took longer than 1.5 refresh intervals (missed at least one vsync). */
  missed: number;
  longTasks: number;
}

/** Records rAF intervals until `stop()`. */
export function startFrameLog(refreshMs: number): { stop: () => FrameStats } {
  const intervals: number[] = [];
  let last = 0;
  let running = true;
  let longTasks = 0;
  const observer = new PerformanceObserver((list) => {
    longTasks += list.getEntries().length;
  });
  try {
    observer.observe({ type: "longtask", buffered: false });
  } catch {
    // Long-task timing is optional.
  }
  const started = performance.now();
  const tick = (now: number) => {
    if (!running) {
      return;
    }
    if (last) {
      intervals.push(now - last);
    }
    last = now;
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
  return {
    stop() {
      running = false;
      observer.disconnect();
      return frameStats(intervals, performance.now() - started, refreshMs, longTasks);
    },
  };
}

/** Median rAF interval over `frames` idle frames: the display refresh. */
export async function measureRefresh(frames = 30): Promise<number> {
  const intervals: number[] = [];
  let last = await nextFrame();
  for (let i = 0; i < frames; i++) {
    const now = await nextFrame();
    intervals.push(now - last);
    last = now;
  }
  return percentile(intervals, 0.5);
}

function nextFrame(): Promise<number> {
  return new Promise((resolve) => requestAnimationFrame(resolve));
}

function frameStats(
  intervals: number[],
  durationMs: number,
  refreshMs: number,
  longTasks: number,
): FrameStats {
  return {
    frames: intervals.length,
    durationMs,
    fps: (intervals.length / durationMs) * 1000,
    refreshMs,
    p50Ms: percentile(intervals, 0.5),
    p95Ms: percentile(intervals, 0.95),
    p99Ms: percentile(intervals, 0.99),
    maxMs: Math.max(0, ...intervals),
    missed: intervals.filter((ms) => ms > refreshMs * 1.5).length,
    longTasks,
  };
}

function percentile(values: number[], p: number): number {
  if (values.length === 0) {
    return 0;
  }
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))] ?? 0;
}

export type ScrollMode = "linear" | "jump";

/**
 * Scrolls `element` every frame for `durationMs`: "linear" moves `rowsPerFrame` rows per frame
 * (a fast fling), "jump" lands on a random spot each frame (every row new: the worst case).
 */
export async function scrollTest(
  element: HTMLElement,
  rowHeight: number,
  mode: ScrollMode,
  durationMs = 4000,
  rowsPerFrame = 10,
): Promise<FrameStats> {
  element.scrollTop = 0;
  const refreshMs = await measureRefresh();
  const log = startFrameLog(refreshMs);
  const end = performance.now() + durationMs;
  let seed = 12345;
  await new Promise<void>((done) => {
    const step = (now: number) => {
      if (now >= end) {
        done();
        return;
      }
      const max = element.scrollHeight - element.clientHeight;
      if (mode === "linear") {
        const next = element.scrollTop + rowsPerFrame * rowHeight;
        element.scrollTop = next >= max ? 0 : next;
      } else {
        seed = (seed * 1103515245 + 12345) % 2147483648;
        element.scrollTop = (seed / 2147483648) * max;
      }
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  });
  return log.stop();
}
