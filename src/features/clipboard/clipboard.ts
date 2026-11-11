import { useEffect, useState } from "react";
import { create } from "zustand";
import { type CopyText, commands, events, type JerryError, subscribe } from "../../ipc";
import type { Folder } from "../../stores/folder";
import { chosenItems } from "../navigation/open";
import { activeFolder, shownRows } from "../selection/commands";

/**
 * The clipboard (PRD «კოპირება, გადატანა და ტრანსფერების რიგი» → «Clipboard»), compatible with
 * Explorer both ways. Ctrl+C and Ctrl+X put the chosen rows (the selected ones the list shows, or
 * else the focused one) on the clipboard as Explorer does; Ctrl+V pastes whatever files are there
 * into the active pane's folder; Ctrl+Shift+C and Ctrl+Alt+C copy their paths or names as text,
 * one per line. The core does the work (`clipboard_*`); a cut's rows show dimmed until the
 * clipboard holds anything else (`clipboard:changed`).
 */

/** Which clipboard command, for the failure dialog's title. */
export type ClipboardAction = "copy" | "cut" | "paste" | "copyPath" | "copyName";

/** A clipboard command that failed: why, and what Retry runs. */
export interface ClipboardFailure {
  action: ClipboardAction;
  error: JerryError;
  retry: () => void;
}

interface ClipboardState {
  /** Jerry's cut is on the clipboard: its rows show dimmed. */
  cut: boolean;
  /** Counts `clipboard:changed`: the lists ask for their cut rows again. */
  version: number;
  failed: ClipboardFailure | undefined;
}

export const useClipboard = create<ClipboardState>(() => ({
  cut: false,
  version: 0,
  failed: undefined,
}));

/**
 * Follows `clipboard:changed` until the returned function is called. Call once per window, at
 * start. A new window starts without a cut: only a cut made since shows dimmed.
 */
export function syncClipboard(): () => void {
  return subscribe(events.clipboardChanged, ({ cut }) =>
    useClipboard.setState((state) => ({ cut, version: state.version + 1 })),
  );
}

/** The rows a clipboard command acts on in `folder`, in display order. */
function chosen(folder: Folder): number[] {
  return chosenItems(folder, shownRows(folder));
}

/** Whether Copy, Cut, Copy path and Copy name have rows to act on in the active pane. */
export function canCopy(): boolean {
  const folder = activeFolder();
  return (
    folder !== undefined &&
    folder.store.getState().listing !== undefined &&
    chosen(folder).length > 0
  );
}

/** Whether Paste has a folder to paste into: the active pane's (search results have none). */
export function canPaste(): boolean {
  const state = activeFolder()?.store.getState();
  return state?.listing !== undefined && state.search === undefined;
}

/** Ctrl+C, or Ctrl+X with `cut`. */
export async function copyEntries(cut: boolean): Promise<void> {
  const folder = activeFolder();
  const listing = folder?.store.getState().listing;
  const ids = folder ? chosen(folder) : [];
  if (listing === undefined || ids.length === 0) {
    return;
  }
  const result = await commands.clipboardCopy(listing, ids, cut);
  if (result.status === "error") {
    fail(cut ? "cut" : "copy", result.error, () => void copyEntries(cut));
  }
}

/**
 * Ctrl+V: the files on the clipboard go into the active pane's folder, or into its subfolder
 * `into` (a folder's context menu), copied, or moved after a cut. Windows shows the progress; the
 * list shows them as they arrive. Text or an image on the clipboard does nothing (Paste as comes
 * later).
 */
export async function pasteEntries(into: number | null = null): Promise<void> {
  const listing = activeFolder()?.store.getState().listing;
  if (listing === undefined) {
    return;
  }
  const result = await commands.clipboardPaste(listing, into);
  if (result.status === "error") {
    fail("paste", result.error, () => void pasteEntries(into));
  }
}

/**
 * Whether the clipboard holds files to paste, asked when the component mounts (a menu opening):
 * undefined until the core answers. Paste stays grayed out without them, as in Explorer.
 */
export function useClipboardHasFiles(): boolean | undefined {
  const [files, setFiles] = useState<boolean>();
  useEffect(() => {
    let current = true;
    void commands.clipboardHasFiles().then((answer) => {
      if (current) setFiles(answer);
    });
    return () => {
      current = false;
    };
  }, []);
  return files;
}

/** Ctrl+Shift+C (`path`) or Ctrl+Alt+C (`name`). */
export async function copyText(text: CopyText): Promise<void> {
  const folder = activeFolder();
  const listing = folder?.store.getState().listing;
  const ids = folder ? chosen(folder) : [];
  if (listing === undefined || ids.length === 0) {
    return;
  }
  const result = await commands.clipboardCopyText(listing, ids, text);
  if (result.status === "error") {
    fail(text === "path" ? "copyPath" : "copyName", result.error, () => void copyText(text));
  }
}

function fail(action: ClipboardAction, error: JerryError, retry: () => void): void {
  useClipboard.setState({ failed: { action, error, retry } });
}

/** The failure dialog's Retry. */
export function retryClipboard(): void {
  const failed = useClipboard.getState().failed;
  useClipboard.setState({ failed: undefined });
  failed?.retry();
}

/** The failure dialog's Close or Esc. */
export function dismissClipboardFailure(): void {
  useClipboard.setState({ failed: undefined });
}

const NONE: ReadonlySet<number> = new Set();

/**
 * The rows of listing `listing` that Jerry cut, while the clipboard holds that cut: they show
 * dimmed. Asked again on every clipboard change and once the listing is all in (`ready`), so a
 * folder opened again, or the same folder in the other pane, shows them too.
 */
export function useCutRows(listing: number | undefined, ready: boolean): ReadonlySet<number> {
  const cut = useClipboard((state) => state.cut);
  const version = useClipboard((state) => state.version);
  const [rows, setRows] = useState<{ listing: number; ids: ReadonlySet<number> }>();
  // biome-ignore lint/correctness/useExhaustiveDependencies: `version` asks again on every clipboard change.
  useEffect(() => {
    if (!cut || listing === undefined || !ready) {
      setRows(undefined);
      return;
    }
    let current = true;
    void commands.clipboardCutEntries(listing).then((result) => {
      if (current && result.status === "ok") {
        setRows({ listing, ids: new Set(result.data) });
      }
    });
    return () => {
      current = false;
    };
  }, [cut, version, listing, ready]);
  return cut && rows !== undefined && rows.listing === listing ? rows.ids : NONE;
}
