import type { DragReport, SpikeError } from "../ipc";

/** Every user-visible string lives here. English only, no i18n library (PRD). */
export const strings = {
  appName: "Jerry",
  version: (version: string) => `Version ${version}`,
  close: "Close",

  /** Spike #13 (OLE drag out). Throwaway, like the screen that uses it. */
  spike: {
    title: "Spike #13: drag files out of Jerry",
    hint: "Select files (Ctrl+click for more), then drag them to another app. Right-drag works too.",
    refresh: "Refresh",
    folder: "Folder",
    noDrag: "No drag yet.",
    dragging: "Dragging…",
    report: (r: DragReport) => {
      if (!r.started) {
        return "Mouse button was released before the drag started.";
      }
      const outcome = r.dropped ? `Dropped: ${r.effect}` : "Cancelled";
      const performed = r.performed ? `, performed ${r.performed}` : "";
      return `${outcome}${performed}. UI→Rust ${ms(r.uiToRustMs)}, queue ${ms(r.queuedMs)}, data object ${ms(r.prepareMs)}, drag ${ms(r.dragMs)}.`;
    },
    capture: (capture: string | null | undefined) =>
      `Mouse capture at drag start: ${capture ?? "none"}`,
    events: "Page events since the drag started",
    error: (error: SpikeError) => {
      switch (error.code) {
        case "UnknownId":
          return "The list is out of date. Refresh and try again.";
        case "Busy":
          return "A drag is already running.";
        case "NotFound":
          return "A file is gone. Refresh and try again.";
        case "Io":
          return "Couldn't read the sample folder.";
        case "Os":
          return "Windows refused the drag. See the log.";
      }
    },
  },
} as const;

function ms(value: number | null) {
  return value === null ? "?" : `${value.toFixed(1)} ms`;
}
