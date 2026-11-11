import { create } from "zustand";
import { commands, type JerryError } from "../../ipc";
import type { Folder, FolderMessage } from "../../stores/folder";
import { useSettings } from "../../stores/settings";
import { strings } from "../../strings";
import { extensionOf } from "../file-list/format";
import { wildcardMatcher } from "../filter/wildcard";
import { activeFolder, selectionOf, shownRows } from "../selection/commands";
import { clickRow, indexOf, NO_MODIFIERS } from "../selection/model";
import { checkName, foldName, type NameProblem } from "./name";

/**
 * Inline rename and new folder (PRD «ფაილური ოპერაციები» → «შექმნა», «ცვლილება»). F2 or a slow
 * double click opens a field in the row, with the name selected without its extension. Enter
 * saves, Esc keeps the old name, Tab saves and goes on to the next row; leaving the field saves
 * too. The field says why a name can't be saved as the user types (`name.ts`, the core's rules,
 * and names the folder already has); changing the extension asks first. Ctrl+Shift+N makes
 * "New folder" (numbered when taken) and opens its name the same way.
 */

/** What Enter or Tab does once the name is saved: stop, or go on to the next row. */
export type After = "close" | "next";

/** The entry whose name is being edited. Replaced as a whole on every change. */
export interface Editing {
  /** Tells one editing from the next, even of the same entry. */
  session: number;
  folder: Folder;
  listing: number;
  id: number;
  /** The entry's name when editing started. */
  original: string;
  /** The extension the field leaves out (".dwg"), with "Show file name extensions" off; or "". */
  hidden: string;
  /** What the field holds. */
  text: string;
  /** Why it can't be the name, as the user types. */
  problem: NameProblem | undefined;
  /** Why the core refused the last save. */
  error: JerryError | undefined;
  saving: boolean;
  /** The extension changes: the question is open, and what follows a yes. */
  confirming: After | undefined;
}

/** Ctrl+Shift+N failed. */
export interface CreateFailure {
  error: JerryError;
}

interface RenameState {
  editing: Editing | undefined;
  createFailed: CreateFailure | undefined;
}

export const useRename = create<RenameState>(() => ({
  editing: undefined,
  createFailed: undefined,
}));

/** How long a new folder or a renamed entry is waited for in the list. */
const WAIT_MS = 10_000;

let sessions = 0;
let creating = false;
/** Stops watching the folder of the entry being edited. */
let unwatch: (() => void) | undefined;

const editing = () => useRename.getState().editing;

function update(change: Partial<Editing>): void {
  const current = editing();
  if (current) {
    useRename.setState({ editing: { ...current, ...change } });
  }
}

/** The entry F2 renames: the focused row, else the only selected one. */
function renameTarget(folder: Folder): number | undefined {
  const { focused, selected, listing } = folder.store.getState();
  if (listing === undefined) {
    return undefined;
  }
  const rows = shownRows(folder);
  if (focused !== undefined && indexOf(rows, focused) !== undefined) {
    return focused;
  }
  const [only] = selected;
  return selected.size === 1 && only !== undefined && indexOf(rows, only) !== undefined
    ? only
    : undefined;
}

/** Whether F2 has an entry to rename in the active pane. */
export function canRename(): boolean {
  const folder = activeFolder();
  return folder !== undefined && renameTarget(folder) !== undefined;
}

/** F2: renames the focused entry of the active pane (several selected: the focused one). */
export function startRename(): void {
  const folder = activeFolder();
  const id = folder && renameTarget(folder);
  if (folder && id !== undefined) {
    beginRename(folder, id);
  }
}

/**
 * Opens entry `id`'s name for editing, in place of any other: it becomes the only selected row
 * and scrolls into view.
 */
export function beginRename(folder: Folder, id: number): void {
  const state = folder.store.getState();
  const { listing, entries } = state;
  const rows = shownRows(folder);
  const at = indexOf(rows, id);
  if (listing === undefined || at === undefined) {
    return;
  }
  stopWatching();
  folder.setSelection(clickRow(selectionOf(state), rows, at, NO_MODIFIERS));
  folder.reveal(id);
  const original = entries.names[id] ?? "";
  const showExtensions = useSettings.getState().settings?.showExtensions ?? true;
  const dot = original.lastIndexOf(".");
  const hidden = showExtensions || entries.isFolder(id) || dot <= 0 ? "" : original.slice(dot);
  const text = original.slice(0, original.length - hidden.length);
  sessions += 1;
  useRename.setState({
    editing: {
      session: sessions,
      folder,
      listing,
      id,
      original,
      hidden,
      text,
      problem: undefined,
      error: undefined,
      saving: false,
      confirming: undefined,
    },
  });
  // The entry going away, or the tab showing another folder, ends the editing.
  unwatch = folder.store.subscribe((now) => {
    if (now.listing !== listing || now.entries.removed.has(id)) {
      cancelRename();
    }
  });
}

/** Whether a name is being edited (the field's Enter, Esc and Tab). */
export function isRenaming(): boolean {
  return editing() !== undefined;
}

/** The field's text changed: checks it as the user types. */
export function setRenameText(text: string): void {
  const current = editing();
  if (current && !current.saving) {
    update({ text, problem: problemOf(current, text), error: undefined });
  }
}

/** Esc: the entry keeps its name (a new folder stays "New folder"). */
export function cancelRename(): void {
  stopWatching();
  useRename.setState({ editing: undefined });
}

/**
 * Enter (`close`) or Tab (`next`): saves the name if it changed and can be saved. A new
 * extension asks first. A name that can't be saved keeps the field open, with the reason.
 */
export async function saveRename(after: After): Promise<void> {
  const current = editing();
  if (!current || current.saving || current.confirming) {
    return;
  }
  const name = current.text + current.hidden;
  if (name === current.original) {
    finish(current, after);
    return;
  }
  const problem = problemOf(current, current.text);
  if (problem) {
    update({ problem });
    return;
  }
  if (extensionChanges(current, name)) {
    update({ confirming: after });
    return;
  }
  await commit(current, name, after);
}

/** The extension question's Change. */
export async function confirmExtension(): Promise<void> {
  const current = editing();
  if (current?.confirming) {
    const after = current.confirming;
    update({ confirming: undefined });
    await commit({ ...current, confirming: undefined }, current.text + current.hidden, after);
  }
}

/** The extension question's Keep editing, or Esc: back to the field. */
export function keepEditing(): void {
  update({ confirming: undefined });
}

/**
 * The field lost the focus (a click elsewhere): saves a name that changed and can be saved,
 * else keeps the old one. Not while a save or the extension question is under way.
 */
export function leaveRename(session: number): void {
  const current = editing();
  if (!current || current.session !== session || current.saving || current.confirming) {
    return;
  }
  const name = current.text + current.hidden;
  if (name === current.original || problemOf(current, current.text)) {
    cancelRename();
  } else {
    void saveRename("close");
  }
}

/** The extension Explorer would warn about changing: files only, compared without case. */
function extensionChanges(current: Editing, name: string): boolean {
  if (current.hidden !== "" || current.folder.store.getState().entries.isFolder(current.id)) {
    return false;
  }
  return (
    extensionOf(current.original, false).toLowerCase() !== extensionOf(name, false).toLowerCase()
  );
}

/** The extensions the question names, with their dots ("" for none). */
export function extensionChange(current: Editing): { from: string; to: string } {
  const dotted = (name: string) => {
    const extension = extensionOf(name, false);
    return extension === "" ? "" : `.${extension}`;
  };
  return { from: dotted(current.original), to: dotted(current.text + current.hidden) };
}

async function commit(current: Editing, name: string, after: After): Promise<void> {
  const { session, folder, listing, id } = current;
  update({ saving: true, error: undefined });
  const result = await commands.rename(listing, id, name);
  if (editing()?.session !== session) {
    return;
  }
  if (result.status === "error") {
    update({ saving: false, error: result.error });
    return;
  }
  revealWhenSorted(folder, listing, id, name);
  finish(current, after);
}

/** Ends the editing; `next` goes on to the row after the entry, as the list shows them now. */
function finish(current: Editing, after: After): void {
  const rows = shownRows(current.folder);
  const at = indexOf(rows, current.id);
  const next = after === "next" && at !== undefined ? rows[at + 1] : undefined;
  if (next !== undefined) {
    beginRename(current.folder, next);
  } else {
    cancelRename();
  }
}

/** Why `text` (with the hidden extension) can't be the entry's new name. */
function problemOf(current: Editing, text: string): NameProblem | undefined {
  const name = text + current.hidden;
  const problem = checkName(name);
  if (problem || name === current.original) {
    return problem;
  }
  // A change of case only is the entry's own name.
  const folded = foldName(name);
  if (folded !== foldName(current.original) && takenNames(current.folder, current.id).has(folded)) {
    return { kind: "taken", name };
  }
  return undefined;
}

/**
 * The names next to entry `id`, folded (`foldName`), hidden entries' too: the folder's, or in
 * search results those found in the same folder (#66). Kept until the folder changes.
 */
const taken = new WeakMap<
  Folder,
  { version: number; place: string | undefined; names: Set<string> }
>();

function takenNames(folder: Folder, id: number): Set<string> {
  const { entries, version } = folder.store.getState();
  const place = entries.location(id);
  const cached = taken.get(folder);
  if (cached?.version === version && cached.place === place) {
    return cached.names;
  }
  const names = new Set<string>();
  for (let other = 0; other < entries.count; other++) {
    const name = entries.names[other];
    if (name !== undefined && !entries.removed.has(other) && entries.location(other) === place) {
      names.add(foldName(name));
    }
  }
  taken.set(folder, { version, place, names });
  return names;
}

function stopWatching(): void {
  unwatch?.();
  unwatch = undefined;
}

/**
 * Scrolls a renamed entry into view once the list has sorted it in its new place: the core sends
 * the new name in a diff, then the new order.
 */
function revealWhenSorted(folder: Folder, listing: number, id: number, name: string): void {
  const renamed = () => folder.store.getState().entries.names[id] === name;
  const reveal = () => {
    if (indexOf(shownRows(folder), id) !== undefined) {
      folder.reveal(id);
    }
  };
  if (renamed()) {
    reveal();
  }
  waitFor(folder, listing, (message) => {
    if (message.kind === "order" && renamed()) {
      reveal();
      return true;
    }
    return false;
  });
}

/**
 * Calls `check` with every message from the folder's listing until it returns true, the folder
 * shows another listing, or `WAIT_MS` pass.
 */
function waitFor(
  folder: Folder,
  listing: number,
  check: (message: FolderMessage) => boolean,
): void {
  const unlisten = folder.listen((message) => {
    if (folder.store.getState().listing !== listing || check(message)) {
      stop();
    }
  });
  const timer = setTimeout(() => stop(), WAIT_MS);
  function stop() {
    clearTimeout(timer);
    unlisten();
  }
}

/** Whether Ctrl+Shift+N can make a folder in the active pane's folder. */
export function canCreateFolder(): boolean {
  const state = activeFolder()?.store.getState();
  return (
    !creating &&
    state?.listing !== undefined &&
    state.status !== "failed" &&
    state.search === undefined
  );
}

/**
 * Ctrl+Shift+N: makes "New folder" (numbered when taken) in the active pane's folder; once the
 * list shows it, it is the only selected row and its name is open for editing. The quick filter
 * goes if it would hide it. A failure shows in a dialog with Retry.
 */
export async function newFolder(): Promise<void> {
  const folder = activeFolder();
  const listing = folder?.store.getState().listing;
  if (!folder || listing === undefined || creating) {
    return;
  }
  creating = true;
  // Entries that come after this are new: the folder is one of them.
  const from = folder.store.getState().entries.count;
  const result = await commands.createFolder(listing, strings.rename.newFolder).finally(() => {
    creating = false;
  });
  if (result.status === "error") {
    useRename.setState({ createFailed: { error: result.error } });
    return;
  }
  const name = result.data;
  const { filter } = folder.store.getState();
  if (filter !== "" && !wildcardMatcher(filter)(name)) {
    folder.setFilter("");
  }
  const shown = () => {
    const state = folder.store.getState();
    if (state.listing !== listing) {
      return true;
    }
    const { entries } = state;
    for (let id = from; id < entries.count; id++) {
      if (entries.names[id] === name && !entries.removed.has(id)) {
        if (indexOf(shownRows(folder), id) === undefined) {
          return false;
        }
        beginRename(folder, id);
        return true;
      }
    }
    return false;
  };
  if (!shown()) {
    waitFor(folder, listing, () => shown());
  }
}

/** The failure dialog's Retry. */
export function retryNewFolder(): void {
  useRename.setState({ createFailed: undefined });
  void newFolder();
}

export function dismissCreateFailed(): void {
  useRename.setState({ createFailed: undefined });
}
