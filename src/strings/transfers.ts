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

/** What a copy or a move did to an item: "copied", "moved". */
function past(kind: TransferKind): string {
  return kind === "copy" ? "copied" : "moved";
}

/** The transfer panel (#52): the job it shows, the queue above it, and summaries. */
const panel = {
  /** The current file: "ELV-010 CCTV Layout Rev 0.dwg · item 126 of 214". */
  item: (name: string, item: number, total: number) =>
    `${name} · item ${integer.format(item)} of ${integer.format(total)}`,
  counting: "Counting…",
  checkingSpace: "Checking free space…",
  /** "1.4 GB of 2.4 GB". */
  bytes: (done: number, total: number) => `${sizeText(done)} of ${sizeText(total)}`,
  percent: (value: number) => `${integer.format(value)}%`,
  /** "42.6 MB/s". */
  speed: (bytesPerSecond: number) => `${sizeText(bytesPerSecond)}/s`,
  /** "About 24 s left", "About 3 min left", "About 2 h 5 min left". */
  timeLeft: (seconds: number) => {
    if (seconds < 60) {
      return `About ${integer.format(Math.max(1, Math.ceil(seconds)))} s left`;
    }
    const minutes = Math.ceil(seconds / 60);
    if (minutes < 60) {
      return `About ${integer.format(minutes)} min left`;
    }
    const hours = Math.floor(minutes / 60);
    const rest = minutes % 60;
    return rest === 0
      ? `About ${integer.format(hours)} h left`
      : `About ${integer.format(hours)} h ${integer.format(rest)} min left`;
  },
  /** The bar, for screen readers. */
  progress: (kind: TransferKind) => (kind === "copy" ? "Copy progress" : "Move progress"),
  /** Where a job is, when it isn't going. */
  queued: "Queued",
  paused: "Paused",
  asking: "Needs your answer",
  /** "ნახაზი.dwg is already there": the conflict window asks what to do. */
  askingDetail: (name: string) => `${name} is already there`,
  waiting: "Waiting for the drive",
  ready: "Ready to resume",
  unfinished: "Not finished",
  unfinishedDetail: "Jerry closed before it finished",
  pause: "Pause",
  resume: "Resume",
  cancel: "Cancel",
  discard: "Discard",
  pauseAll: "Pause all",
  resumeAll: "Resume all",
  cancelAll: "Cancel all",
  /** The queue's caption: "4 transfers". */
  count: (count: number) => `${integer.format(count)} ${count === 1 ? "transfer" : "transfers"}`,
  /** The button that opens the queue: "Queue · 2". */
  queue: (count: number) => `Queue · ${integer.format(count)}`,
  showQueue: (count: number) => `Show transfer queue, ${integer.format(count)} waiting`,
  hideQueue: "Hide transfer queue",
  /** The queued jobs, for screen readers. */
  queueLabel: "Transfer queue",
  /** Closes the panel; the transfers go on. */
  hide: "Hide transfers",
  /** The status bar's way back to the panel. */
  show: "Show transfers",
  /** "3 files could not be copied". */
  failed: (count: number, kind: TransferKind) =>
    `${integer.format(count)} ${count === 1 ? "file" : "files"} could not be ${past(kind)}`,
  /** A job that couldn't start: "Couldn't copy 3 items to Archive". */
  couldntStart: (kind: TransferKind, sources: number, first: string, destination: string) => {
    const what = sources === 1 ? first : `${integer.format(sources)} items`;
    return `Couldn't ${kind} ${what} to ${destination}`;
  },
  /** Under a summary: "211 files copied to Archive". */
  finished: (files: number, kind: TransferKind, destination: string) =>
    `${integer.format(files)} ${files === 1 ? "file" : "files"} ${past(kind)} to ${destination}`,
  /** A move's copy that is complete, whose original stays: before the reason. */
  copiedOnly: "Copied, but the original stays.",
  showFailures: "Show list",
  hideFailures: "Hide list",
  /** The failed items, for screen readers. */
  failures: "Items that failed",
  retry: "Retry",
  dismiss: "Dismiss",
  /** Reordering the queue by drag (dnd-kit). */
  dragInstructions:
    "To move a transfer in the queue, press Space, then the arrow keys. Space puts it down, Escape cancels.",
  pickedUp: (name: string) => `Picked up ${name}.`,
  movedOver: (name: string, over: string) => `${name} is now where ${over} was.`,
  putDown: (name: string) => `${name} put down.`,
  dragCancelled: (name: string) => `Moving ${name} cancelled.`,
  /** The grip a row is dragged by. */
  reorder: (name: string) => `Move ${name} in the queue`,
};

export const transferStrings = {
  conflict,
  panel,
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
