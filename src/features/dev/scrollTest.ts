import { commands } from "../../ipc";
import { afterNextPaint } from "./paint";

/** How a scroll through a list went, frame by frame. */
export interface ScrollResult {
  rows: number;
  ms: number;
  frames: number;
  /** Frame times: the median is the display's refresh interval when nothing is dropped. */
  medianMs: number;
  p95Ms: number;
  maxMs: number;
  /** Frames longer than 1.5 × the median: each one is a visible stutter. */
  janky: number;
  /** Tasks of 50 ms or more on the page's main thread. */
  longTasks: number;
  /** Icon requests served from memory, sent to the Shell pool, rendered and failed meanwhile. */
  fromMemory: number;
  fromShell: number;
  rendered: number;
  failed: number;
}

/**
 * Dev builds: scrolls `viewport` from top to bottom at an even speed over `durationMs`, one step
 * per frame, and times every frame. Ten thousand rows in five seconds is a fast fling: every
 * frame shows rows (and icons) that weren't on screen the frame before.
 */
export async function scrollTest(
  viewport: HTMLElement,
  rowHeight: number,
  durationMs = 5000,
): Promise<ScrollResult> {
  viewport.scrollTop = 0;
  await afterNextPaint();
  const before = await iconStats();
  let longTasks = 0;
  const observer = PerformanceObserver.supportedEntryTypes.includes("longtask")
    ? new PerformanceObserver((list) => {
        longTasks += list.getEntries().length;
      })
    : undefined;
  observer?.observe({ type: "longtask" });

  const distance = viewport.scrollHeight - viewport.clientHeight;
  const frames: number[] = [];
  const ms = await new Promise<number>((resolve) => {
    let start = 0;
    let last = 0;
    const step = (now: number) => {
      if (start === 0) {
        start = now;
      } else {
        frames.push(now - last);
      }
      last = now;
      const progress = Math.min(1, (now - start) / durationMs);
      viewport.scrollTop = progress * distance;
      if (progress < 1) {
        requestAnimationFrame(step);
      } else {
        resolve(now - start);
      }
    };
    requestAnimationFrame(step);
  });
  observer?.disconnect();
  // Icons for the last rows may still be on their way.
  await new Promise((resolve) => setTimeout(resolve, 500));
  const after = await iconStats();

  const sorted = [...frames].sort((a, b) => a - b);
  const at = (quantile: number) => sorted[Math.floor(quantile * (sorted.length - 1))] ?? 0;
  const median = at(0.5);
  return {
    rows: Math.round(distance / rowHeight),
    ms,
    frames: frames.length,
    medianMs: median,
    p95Ms: at(0.95),
    maxMs: sorted.at(-1) ?? 0,
    janky: frames.filter((frame) => frame > median * 1.5).length,
    longTasks,
    fromMemory: after.fromMemory - before.fromMemory,
    fromShell: after.fromShell - before.fromShell,
    rendered: after.rendered - before.rendered,
    failed: after.failed - before.failed,
  };
}

async function iconStats() {
  const stats = await commands.devIconStats();
  return stats.status === "ok"
    ? stats.data
    : { fromMemory: 0, fromShell: 0, rendered: 0, failed: 0 };
}
