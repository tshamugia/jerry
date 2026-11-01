import type { ClipboardState, CopyReport, PasteReport, SpikeError } from "../ipc";

const ms = (value: number | null) => (value === null ? "?" : value.toFixed(1));

/** Every user-visible string lives here. English only, no i18n library (PRD). */
export const strings = {
  appName: "Jerry",
  version: (version: string) => `Version ${version}`,

  /** Spike #15 (clipboard Jerry ↔ Explorer) only; goes away with the spike branch. */
  spike: {
    windowTitle: "Jerry · spike #15 clipboard",
    close: "Close",
    title: "Clipboard ↔ Explorer",
    copy: "Copy (Ctrl+C)",
    cut: "Cut (Ctrl+X)",
    paste: "Paste (Ctrl+V)",
    selectAll: "Select all (Ctrl+A)",
    refresh: "Refresh",
    hint: "Copy or cut here and paste in Explorer, Outlook or Telegram; copy or cut in Explorer and paste here. Paste goes into this folder (%TEMP%\\jerry-spike-15). Closing Jerry keeps its files on the clipboard.",
    folder: "Folder",
    ready: "Nothing copied yet.",
    nothingSelected: "Select files first.",
    pasting: "Pasting…",
    copied: (r: CopyReport) =>
      `${r.op === "cut" ? "Cut" : "Copied"} ${r.files} item(s) to the clipboard · queued ${ms(r.queuedMs)} ms, Shell object ${ms(r.prepareMs)} ms, OleSetClipboard ${ms(r.setMs)} ms`,
    pasted: (r: PasteReport) => {
      const what = r.op === "cut" ? "Moved" : "Copied";
      const from = r.fromJerry ? "from Jerry" : "from another app";
      const skipped = r.skipped.length > 0 ? ` · skipped: ${r.skipped.join(", ")}` : "";
      return `${what} ${r.pasted.length} item(s) ${from} (Preferred DropEffect ${r.preferredEffect ?? "none"}) · read ${ms(r.readMs)} ms, ${r.op === "cut" ? "move" : "copy"} ${ms(r.pasteMs)} ms${skipped}`;
    },
    clipboard: (c: ClipboardState) => {
      const owner = {
        none: "nobody",
        jerry: "Jerry",
        other: `another app (pid ${c.ownerPid ?? "?"})`,
      }[c.owner];
      if (c.files === 0) return `Clipboard: no files · owner: ${owner}`;
      return `Clipboard: ${c.files} file(s), ${c.op === "cut" ? "Cut" : "Copy"} · owner: ${owner}`;
    },
    formats: (formats: string[]) => `Formats: ${formats.join(", ")}`,
    error: (error: SpikeError) =>
      ({
        UnknownId: "The list is out of date. Refresh and try again.",
        Busy: "A paste is still running.",
        NothingToPaste: "The clipboard holds no files.",
        NotFound: "A file is gone. Refresh and try again.",
        Io: "The file operation failed. See %TEMP%\\jerry-spike-15.log.",
        Os: "Windows refused. See %TEMP%\\jerry-spike-15.log.",
      })[error.code],
  },
} as const;
