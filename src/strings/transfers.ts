/** Copying and moving (v0.2). English only (PRD). */

import type { TransferKind } from "../ipc";
import { sizeText } from "./errors";

const integer = new Intl.NumberFormat("en-US");

export const transferStrings = {
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
