import { commands, type JsMark } from "../../ipc";
import { jsStart } from "./first";

/**
 * Spike #11: startup stamps on the UI side. "js:start" comes from `./first`, the bundle's first
 * module. Stamps are `performance.now()` values; the report turns them into ms since the Unix
 * epoch (`timeOrigin + t`) to line up with Rust's.
 */

const marks: [string, number][] = [["js:start", jsStart]];
const values: [string, number][] = [];

/** Stamps a startup stage once (later calls with the same name are ignored). */
export function stamp(name: string, at = performance.now()): void {
  if (!marks.some(([n]) => n === name)) {
    marks.push([name, at]);
  }
}

/** Records a loose value for the report. */
export function value(name: string, v: number): void {
  values.push([name, v]);
}

let firstRowPainted: () => void = () => {};
/** Resolves once the first row's text has been painted (Element Timing), or after 5 s. */
export const firstRowPaint = new Promise<void>((resolve) => {
  firstRowPainted = resolve;
  setTimeout(resolve, 5000);
});

// Paint timings come from the compositor's presentation time, buffered from page start.
new PerformanceObserver((list) => {
  for (const entry of list.getEntries()) {
    if (entry.entryType === "paint") {
      stamp(`paint:${entry.name}`, entry.startTime);
    } else if (entry.entryType === "element") {
      const element = entry as PerformanceEntry & { renderTime: number; identifier: string };
      stamp(`element:${element.identifier}`, element.renderTime || element.startTime);
      if (element.identifier === "first-row") {
        firstRowPainted();
      }
    }
  }
}).observe({ entryTypes: ["paint", "element"] });

document.addEventListener("visibilitychange", () => {
  stamp(`visibility:${document.visibilityState}`);
});
value("visibilityAtStart", document.visibilityState === "visible" ? 1 : 0);

/** Stamps when the frame after `name` reaches the compositor: rAF, then a macrotask. */
export function stampNextFrame(name: string): void {
  requestAnimationFrame(() => {
    stamp(`${name}:raf`);
    const channel = new MessageChannel();
    channel.port1.onmessage = () => stamp(`${name}:frame`);
    channel.port2.postMessage(null);
  });
}

const toEpoch = ([name, at]: [string, number]): JsMark => ({
  name,
  at: performance.timeOrigin + at,
});

/** Sends the stamps to Rust (which writes the report). */
export async function sendReport(phase: string): Promise<void> {
  const navigation = performance.getEntriesByType("navigation")[0] as
    | PerformanceNavigationTiming
    | undefined;
  if (navigation) {
    for (const key of [
      "fetchStart",
      "responseStart",
      "responseEnd",
      "domInteractive",
      "domContentLoadedEventStart",
      "loadEventStart",
    ] as const) {
      stamp(`nav:${key}`, navigation[key]);
    }
  }
  for (const entry of performance.getEntriesByType("resource") as PerformanceResourceTiming[]) {
    const file = entry.name.split("/").pop() ?? entry.name;
    const kind = file.endsWith(".css") ? "css" : file.endsWith(".js") ? "js" : file;
    stamp(`res:${kind}:start`, entry.startTime);
    stamp(`res:${kind}:end`, entry.responseEnd);
    value(`res:${kind}:bytes`, entry.decodedBodySize);
  }
  const memory = (performance as Performance & { memory?: { usedJSHeapSize: number } }).memory;
  if (memory) {
    value(`jsHeap:${phase}`, memory.usedJSHeapSize);
  }
  // Both clocks should agree; a large gap would skew every stamp.
  value("clockSkewMs", Date.now() - (performance.timeOrigin + performance.now()));
  await commands.startupReport({
    phase,
    marks: marks.map(toEpoch),
    values: values.map(([name, v]) => ({ name, at: v })),
  });
}
