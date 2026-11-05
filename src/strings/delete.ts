/** Deleting (Del, Shift+Del): the confirmations and what failed. English only (PRD). */

import { sizeText } from "./errors";

const integer = new Intl.NumberFormat("en-US");

const items = (count: number) => `${integer.format(count)} ${count === 1 ? "item" : "items"}`;
const plural = (count: number, one: string, many: string) =>
  `${integer.format(count)} ${count === 1 ? one : many}`;

/** What is being deleted: one item by name, or a count. */
export interface Subject {
  count: number;
  /** The item's name when there is one item. */
  name: string | undefined;
}

const subject = ({ count, name }: Subject) =>
  count === 1 && name !== undefined ? `“${name}”` : items(count);

export const deleteStrings = {
  cancel: "Cancel",
  close: "Esc",
  /** Del with "Confirm moving to the Recycle Bin" on, above 100 items, or with a system folder. */
  recycle: {
    title: "Move to Recycle Bin",
    question: (what: Subject) => `Move ${subject(what)} to the Recycle Bin?`,
    confirm: "Move to Recycle Bin",
  },
  /** Shift+Del. */
  permanent: {
    title: "Delete permanently",
    question: (what: Subject) => `Permanently delete ${subject(what)}?`,
    confirm: "Delete permanently",
  },
  /** Items the Recycle Bin can't take, after Del: they are still there. */
  noRecycleBin: {
    title: "No Recycle Bin here",
    question: (what: Subject) =>
      what.count === 1
        ? `${subject(what)} can't go to the Recycle Bin. Delete it permanently?`
        : `${subject(what)} can't go to the Recycle Bin. Delete them permanently?`,
    why: "Network and removable drives have no Recycle Bin, and Windows can't recycle paths over 260 characters or files bigger than the Recycle Bin.",
    confirm: "Delete permanently",
    keep: (count: number) => (count === 1 ? "Keep it" : "Keep them"),
  },
  /** Under a permanent delete's question. */
  noUndo: "This can't be undone.",
  /** What a permanent delete removes, while it is counted and once it is. */
  counting: "Counting…",
  tally: (tally: { files: number; folders: number; bytes: number }, done: boolean): string => {
    const parts = [
      ...(tally.folders > 0 ? [plural(tally.folders, "folder", "folders")] : []),
      plural(tally.files, "file", "files"),
      sizeText(tally.bytes),
    ];
    return done ? parts.join(", ") : `Counting… ${parts.join(", ")} so far`;
  },
  /** The extra confirmation for system folders (data safety rule 6). */
  system: {
    warning: (count: number) =>
      count === 1
        ? "This is a Windows system folder, or holds one. Deleting it can stop Windows or your programs from working."
        : "The selection includes a Windows system folder, or a folder that holds one. Deleting it can stop Windows or your programs from working.",
    agree: "I understand. Delete the system folder anyway.",
  },
  /** After a delete that left something behind. */
  failed: {
    title: "Couldn't delete everything",
    summary: (count: number) =>
      count === 1
        ? "1 item couldn't be deleted."
        : `${integer.format(count)} items couldn't be deleted.`,
    /** Past the ones listed by name. */
    more: (count: number) => `…and ${items(count)} more.`,
    retry: "Retry",
    dismiss: "Close",
  },
} as const;
