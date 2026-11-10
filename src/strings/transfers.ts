/** Copying and moving (v0.2). English only (PRD). */

import type { ConflictChoice, TransferKind } from "../ipc";
import { sizeText } from "./errors";

const integer = new Intl.NumberFormat("en-US");

/** The conflict window (#51): a name a copy or move writes is taken. */
const conflict = {
  /** The window's title. */
  title: (folder: boolean) =>
    folder ? "A folder with this name is already there" : "A file with this name is already there",
  /** The title when Replace found no Recycle Bin for the file there. */
  noRecycleBinTitle: "This file can't go to the Recycle Bin",
  /** "ნახაზი.dwg is already in Archive." */
  already: (name: string, folder: string) => `${name} is already in ${folder}.`,
  /** Why Replace now asks again. */
  noRecycleBin:
    "The drive has no Recycle Bin for the file there. Replace deletes it for good, and that can't be undone.",
  /** The side that comes: "Copying from Projects", "Moving from Projects". */
  incoming: (kind: TransferKind, folder: string) =>
    `${kind === "copy" ? "Copying" : "Moving"} from ${folder}`,
  /** The side that is there: "Already in Archive". */
  existing: (folder: string) => `Already in ${folder}`,
  /** A folder has no size to show. */
  folder: "Folder",
  /** Its modified date can't be read. */
  noDate: "Date unknown",
  /** Marks the one modified later. */
  newer: "Newer",
  /** The list of choices, for screen readers. */
  choices: "What to do",
  choice: {
    replace: "Replace",
    skip: "Skip",
    keepBoth: "Keep both",
    replaceIfNewer: "Replace if newer",
    merge: "Merge",
  } satisfies Record<ConflictChoice, string>,
  /** What each choice does, under it. */
  hint: {
    replace: "The one there goes to the Recycle Bin",
    skip: "Leave the one there as it is",
    keepBoth: "The new one gets a numbered name",
    replaceIfNewer: "Replace it only if the new one was modified later",
    merge: "Put what is in it into the folder there",
  } satisfies Record<ConflictChoice, string>,
  /** Replace where there is no Recycle Bin. */
  replaceForGood: "Replace and delete the one there for good",
  /** "Apply to all": the same answer for every later conflict in the transfer. */
  applyToAll: "Apply to all",
  /** Ends the transfer: what is done stays. */
  cancel: "Cancel transfer",
  close: "Esc",
};

export const transferStrings = {
  conflict,
  /** The notice about jobs the Jerry before didn't finish (#50). */
  unfinished: (count: number) =>
    count === 1
      ? "Jerry closed before a transfer finished"
      : `Jerry closed before ${integer.format(count)} transfers finished`,
  /** One of them: "Copying 3 items to Archive", "Moving ნახაზი.dwg to Archive". */
  job: (kind: TransferKind, sources: number, first: string, destination: string) => {
    const what = sources === 1 ? first : `${integer.format(sources)} items`;
    return `${kind === "copy" ? "Copying" : "Moving"} ${what} to ${destination}`;
  },
  /** How far it came: "1 of 3 files · 8 MB of 24 MB". */
  done: (files: number, filesTotal: number, bytes: number, bytesTotal: number) =>
    `${integer.format(files)} of ${integer.format(filesTotal)} ${filesTotal === 1 ? "file" : "files"} · ${sizeText(bytes)} of ${sizeText(bytesTotal)}`,
  /** It stopped before it had counted what to do. */
  nothingDone: "Nothing done yet",
  /** After the first few jobs. */
  more: (count: number) => `and ${integer.format(count)} more`,
  resume: "Resume",
  discard: "Discard",
};
