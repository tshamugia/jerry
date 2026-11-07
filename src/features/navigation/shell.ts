import { create } from "zustand";
import { commands, type JerryError } from "../../ipc";
import { activeFolder, shownRows } from "../selection/commands";
import { chosenItems } from "./open";

/**
 * Leaving Jerry for Windows' own tools (PRD «გახსნა»): Show in Explorer opens the folder in
 * Explorer with the chosen rows selected; Open in Terminal opens Windows Terminal, or PowerShell
 * without it, in a folder. The core does the work (`show_in_explorer`, `open_terminal`).
 */

/** Which command failed, for the failure dialog's title. */
export type ShellAction = "showInExplorer" | "openInTerminal";

/** A command that failed: why, and what Retry runs. */
export interface ShellFailure {
  action: ShellAction;
  error: JerryError;
  retry: () => void;
}

export const useShellFailure = create<{ failed: ShellFailure | undefined }>(() => ({
  failed: undefined,
}));

/** Whether Show in Explorer has rows to show: the active pane's chosen ones. */
export function canShowInExplorer(): boolean {
  const folder = activeFolder();
  return (
    folder !== undefined &&
    folder.store.getState().listing !== undefined &&
    chosenItems(folder, shownRows(folder)).length > 0
  );
}

/** Whether Open in Terminal has a folder: the active pane's. */
export function canOpenTerminal(): boolean {
  return activeFolder()?.store.getState().listing !== undefined;
}

/** Show in Explorer: the active pane's chosen rows, selected in Explorer in their folder. */
export async function showInExplorer(): Promise<void> {
  const folder = activeFolder();
  const listing = folder?.store.getState().listing;
  const ids = folder ? chosenItems(folder, shownRows(folder)) : [];
  if (listing === undefined || ids.length === 0) {
    return;
  }
  const result = await commands.showInExplorer(listing, ids);
  if (result.status === "error") {
    fail("showInExplorer", result.error, () => void showInExplorer());
  }
}

/**
 * Open in Terminal: in the one folder chosen in the active pane, as Explorer does on a folder, or
 * else (`here`, or files chosen) in the folder the pane shows.
 */
export async function openInTerminal(here = false): Promise<void> {
  const folder = activeFolder();
  const state = folder?.store.getState();
  if (!folder || state?.listing === undefined) {
    return;
  }
  const ids = here ? [] : chosenItems(folder, shownRows(folder));
  const [only] = ids;
  const entry =
    ids.length === 1 && only !== undefined && state.entries.isFolder(only) ? only : null;
  const result = await commands.openTerminal(state.listing, entry);
  if (result.status === "error") {
    fail("openInTerminal", result.error, () => void openInTerminal(here));
  }
}

function fail(action: ShellAction, error: JerryError, retry: () => void): void {
  useShellFailure.setState({ failed: { action, error, retry } });
}

/** The failure dialog's Retry. */
export function retryShell(): void {
  const failed = useShellFailure.getState().failed;
  useShellFailure.setState({ failed: undefined });
  failed?.retry();
}

/** The failure dialog's Close or Esc. */
export function dismissShellFailure(): void {
  useShellFailure.setState({ failed: undefined });
}
