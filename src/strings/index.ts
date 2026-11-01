import type {
  DndEnter,
  DndKeys,
  DndStats,
  DragReport,
  DropLabels,
  InstallReport,
  SpikeError,
} from "../ipc";

/** Every user-visible string lives here. English only, no i18n library (PRD). */
export const strings = {
  appName: "Jerry",
  version: (version: string) => `Version ${version}`,
  close: "Close",

  /** Spikes #13 and #14 (OLE drag out and in). Throwaway, like the screen that uses them. */
  spike: {
    folder: "Folder",
    report: (r: DragReport) => {
      if (!r.started) {
        return "Mouse button was released before the drag started.";
      }
      const outcome = r.dropped ? `Dragged out: ${r.effect}` : "Drag cancelled";
      const performed = r.performed ? `, performed ${r.performed}` : "";
      return `${outcome}${performed}. UI→Rust ${ms(r.uiToRustMs)}, data object ${ms(r.prepareMs)}, drag ${ms(r.dragMs)}.`;
    },
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
          return "Windows refused. See the log.";
      }
    },
  },

  /** Spike #14 (OLE drop into Jerry). */
  drop: {
    favorites: "Favorites",
    network: "Network (refuses drops)",
    empty: "This folder is empty. Drop files here",
    copyHere: "Copy here",
    moveHere: "Move here",
    cancel: "Cancel",
    windows: "Window tree before / after registering",
    installing: "Registering the drop target…",
    /** Drop description texts; Windows puts the folder name where `%1` is. */
    labels: (count: number): DropLabels => {
      const items = count > 1 ? ` ${count} items` : "";
      return {
        copy: `Copy${items} to %1`,
        moves: `Move${items} to %1`,
        link: `Create link${count > 1 ? "s" : ""} in %1`,
      };
    },
    install: (r: InstallReport) =>
      `Drop target on ${r.registered} window${r.registered === 1 ? "" : "s"}. WebView2 external drop ${
        r.webviewDropOff ? "off" : "still on"
      }.`,
    domDrags: (count: number) =>
      `WebView2's own drop target got ${count} drag events: Jerry's isn't the one in use.`,
    drag: (d: DndEnter) => {
      const what = d.files > 0 ? `${d.files} files` : `${d.virtualFiles} virtual files`;
      const from = d.fromJerry ? " from Jerry" : "";
      const asyncMode = d.asyncCapable ? ", async" : "";
      return `Drag${from}: ${what} (${d.names.join(", ")}), allows ${d.allowed.join("/")}${asyncMode}. Formats: ${d.formats.join(", ")}`;
    },
    over: (target: string) => `Over: ${target}`,
    keys: (k: DndKeys) =>
      [k.left && "left", k.right && "right", k.ctrl && "Ctrl", k.shift && "Shift", k.alt && "Alt"]
        .filter(Boolean)
        .join("+"),
    stats: (s: DndStats) =>
      `Last drag: ${s.overs} DragOver calls, ${s.emitted} points, ${s.answered} answered, ${s.stale} answered late. Round trip median ${ms(s.rttMedianMs)}, p95 ${ms(s.rttP95Ms)}, max ${ms(s.rttMaxMs)}. DragOver every ${ms(s.overGapMedianMs)} (max ${ms(s.overGapMaxMs)}).`,
    logEntry: (e: {
      dropId: number;
      requested: string;
      items: number;
      readMs: number;
      takeMs: number;
      state: string;
      done?: {
        target: string;
        saved: string[];
        errors: string[];
        bytes: number | null;
        ms: number | null;
      };
    }) => {
      const head = `#${e.dropId} ${e.state}: ${e.items} items, ${e.requested} requested (spike copies only), Drop read ${ms(e.readMs)}, take ${ms(e.takeMs)}`;
      if (!e.done) return head;
      const errors = e.done.errors.length > 0 ? ` Errors: ${e.done.errors.join("; ")}` : "";
      return `${head}. Into ${e.done.target}: ${e.done.saved.join(", ") || "nothing"} (${e.done.bytes ?? 0} B, ${ms(e.done.ms)}).${errors}`;
    },
  },
} as const;

function ms(value: number | null) {
  return value === null ? "?" : `${value.toFixed(1)} ms`;
}
