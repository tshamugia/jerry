import { create } from "zustand";
import { commands, type DeleteFailure, type JerryError } from "../../ipc";
import type { Folder } from "../../stores/folder";
import { useSettings } from "../../stores/settings";
import { activeFolder, shownRows } from "../selection/commands";

/**
 * Deleting (PRD «ფაილური ოპერაციები» → «წაშლა»). Del sends the entries to the Recycle Bin; it
 * asks first only with "Confirm moving to the Recycle Bin" on, above 100 items, or for a system
 * folder. Shift+Del always asks, with what it removes, and deletes for good. Entries the Recycle
 * Bin can't take come back untouched, and Jerry asks whether to delete them for good. Both act on
 * the rows the list shows: the selected ones, or else the focused one.
 */

/** Above this many items Del always asks. */
export const ASK_ABOVE = 100;

/** Which confirmation: Del's, Shift+Del's, or for items the Recycle Bin can't take. */
export type DeleteMode = "recycle" | "permanent" | "noRecycleBin";

/** Entries of one listing to delete. */
export interface DeleteRequest {
  folder: Folder;
  listing: number;
  /** Entry ids, in display order. */
  ids: number[];
  /** One of them is a system folder or holds one: the dialog asks for an extra confirmation. */
  system: boolean;
}

/** An entry that stayed: its name, why, and which delete Retry runs for it again. */
export interface Failure {
  id: number;
  name: string;
  error: JerryError;
  /** It failed to be deleted for good, which the user confirmed; else to go to the Recycle Bin. */
  permanent: boolean;
}

export type DeleteDialogState =
  | {
      kind: "confirm";
      mode: DeleteMode;
      request: DeleteRequest;
      /** Failures of the delete before this question, shown after it. */
      earlier: Failure[];
    }
  | { kind: "failed"; request: DeleteRequest; failures: Failure[] };

/** The delete dialog this window shows, if any. */
export const useDeleteDialog = create<{ dialog: DeleteDialogState | undefined }>(() => ({
  dialog: undefined,
}));

const show = (dialog: DeleteDialogState | undefined) => useDeleteDialog.setState({ dialog });

/** Entries being deleted now, by listing: pressing Del again doesn't delete them twice. */
const busy = new Map<number, Set<number>>();

/** The entries Del acts on: the selected rows the list shows, in display order, else the focused one. */
export function deleteTargets(folder: Folder): number[] {
  const { selected, focused, listing } = folder.store.getState();
  const rows = shownRows(folder);
  const working = listing === undefined ? undefined : busy.get(listing);
  const free = (id: number) => !working?.has(id);
  if (selected.size > 0) {
    return Array.from(rows).filter((id) => selected.has(id) && free(id));
  }
  return focused !== undefined && rows.includes(focused) && free(focused) ? [focused] : [];
}

/** Whether Del or Shift+Del has anything to act on in the active pane. */
export function canDelete(): boolean {
  return deleteTargets(activeFolder()).length > 0;
}

/** Del (`permanent` false) or Shift+Del on the active pane's list. */
export async function startDelete(permanent: boolean): Promise<void> {
  const folder = activeFolder();
  const { listing } = folder.store.getState();
  const ids = deleteTargets(folder);
  if (listing === undefined || ids.length === 0) {
    return;
  }
  const check = await commands.deleteCheck(listing, ids);
  // Should the check itself fail, the deletes still refuse an unconfirmed system folder.
  const request: DeleteRequest = {
    folder,
    listing,
    ids,
    system: check.status === "ok" && check.data.system,
  };
  const confirmRecycle = useSettings.getState().settings?.confirmRecycle ?? false;
  if (permanent || request.system || ids.length > ASK_ABOVE || confirmRecycle) {
    show({ kind: "confirm", mode: permanent ? "permanent" : "recycle", request, earlier: [] });
  } else {
    await recycle(request);
  }
}

/** The dialog's confirm button. */
export async function confirmDelete(): Promise<void> {
  const dialog = useDeleteDialog.getState().dialog;
  if (dialog?.kind !== "confirm") {
    return;
  }
  show(undefined);
  if (dialog.mode === "recycle") {
    await recycle(dialog.request);
  } else {
    await permanently(dialog.request, dialog.earlier);
  }
}

/** The dialog's Cancel, Close or Esc. Failures from before a question still show. */
export function dismissDelete(): void {
  const dialog = useDeleteDialog.getState().dialog;
  if (dialog?.kind === "confirm" && dialog.earlier.length > 0) {
    show({ kind: "failed", request: dialog.request, failures: dialog.earlier });
  } else {
    show(undefined);
  }
}

/**
 * The failure dialog's Retry: each entry that stayed gets the delete it failed, never more. One
 * that didn't go to the Recycle Bin goes there again, and is deleted for good only if the user
 * says so then.
 */
export async function retryDelete(): Promise<void> {
  const dialog = useDeleteDialog.getState().dialog;
  if (dialog?.kind !== "failed") {
    return;
  }
  show(undefined);
  const again = (permanent: boolean) => ({
    ...dialog.request,
    ids: dialog.failures
      .filter((failure) => failure.permanent === permanent)
      .map((failure) => failure.id),
  });
  const forGood = again(true);
  if (forGood.ids.length > 0) {
    await permanently(forGood, []);
  }
  const toBin = again(false);
  if (toBin.ids.length > 0) {
    await recycle(toBin);
  }
}

async function recycle(request: DeleteRequest): Promise<void> {
  const result = await working(request, () =>
    commands.deleteRecycle(request.listing, request.ids, request.system),
  );
  if (result.status === "error") {
    showFailures(request, [], commandFailures(request, result.error, false));
    return;
  }
  const { noRecycleBin, failed } = result.data;
  const failures = named(request.folder, failed, false);
  if (noRecycleBin.length > 0) {
    const ids = new Set(noRecycleBin);
    show({
      kind: "confirm",
      mode: "noRecycleBin",
      // In display order, as asked.
      request: { ...request, ids: request.ids.filter((id) => ids.has(id)) },
      earlier: failures,
    });
  } else {
    showFailures(request, failures, []);
  }
}

async function permanently(request: DeleteRequest, earlier: Failure[]): Promise<void> {
  const result = await working(request, () =>
    commands.deletePermanently(request.listing, request.ids, request.system),
  );
  const failures =
    result.status === "error"
      ? commandFailures(request, result.error, true)
      : named(request.folder, result.data.failed, true);
  showFailures(request, earlier, failures);
}

/** Runs `run` with the request's entries marked busy. */
async function working<T>(request: DeleteRequest, run: () => Promise<T>): Promise<T> {
  let entries = busy.get(request.listing);
  if (!entries) {
    entries = new Set();
    busy.set(request.listing, entries);
  }
  for (const id of request.ids) entries.add(id);
  try {
    return await run();
  } finally {
    for (const id of request.ids) entries.delete(id);
    if (entries.size === 0) busy.delete(request.listing);
  }
}

/** Shows what stayed, if anything: `earlier` before a question, `later` after it. */
function showFailures(request: DeleteRequest, earlier: Failure[], later: Failure[]): void {
  const failures = [...earlier, ...later];
  if (failures.length > 0) {
    show({ kind: "failed", request, failures });
  }
}

function named(folder: Folder, failed: DeleteFailure[], permanent: boolean): Failure[] {
  const { names } = folder.store.getState().entries;
  return failed.map(({ entry, error }) => ({
    id: entry,
    name: names[entry] ?? "",
    error,
    permanent,
  }));
}

/** The whole command failed: every entry stayed, for the same reason. */
function commandFailures(request: DeleteRequest, error: JerryError, permanent: boolean): Failure[] {
  return named(
    request.folder,
    request.ids.map((entry) => ({ entry, error })),
    permanent,
  );
}
