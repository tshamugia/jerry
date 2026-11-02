import { commands } from "../../ipc";

/**
 * Dev builds: sends every long task (the page's main thread busy for 50 ms or more, so the UI
 * froze) to Jerry's log. Returns a function that stops it.
 */
export function reportLongTasks(): () => void {
  if (!PerformanceObserver.supportedEntryTypes.includes("longtask")) {
    return () => {};
  }
  const observer = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) {
      void commands.devLongTask(entry.duration);
    }
  });
  observer.observe({ type: "longtask", buffered: true });
  return () => observer.disconnect();
}
