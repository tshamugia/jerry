import { HEADER_BYTES } from "./channel";
import { entriesBytes, type TestEntry, unitCount, writeEntries } from "./listing.testing";

/**
 * Tests only: search messages built exactly as `src-tauri/src/search.rs` and
 * `src-tauri/src/results.rs` lay them out, so UI tests can feed a fake `search_start` channel.
 */

/** UTF-16 code units, as Rust writes them. */
function stringUnits(text: string): number[] {
  return Array.from({ length: text.length }, (_, i) => text.charCodeAt(i));
}

/**
 * A batch or a diff: `header`, the folder row, `words` (ids, removed ids, folder ids), the new
 * folders' ends, padding, the entries and the folders' texts.
 */
function resultsMessage(
  header: [number, number, number],
  words: number[],
  entries: TestEntry[],
  firstFolder: number,
  folders: string[],
): ArrayBuffer {
  const units = unitCount(entries);
  const folderUnits = folders.reduce((count, folder) => count + folder.length, 0);
  const count = words.length + folders.length;
  const entriesAt = HEADER_BYTES * 2 + (count + (count % 2)) * 4;
  const buffer = new ArrayBuffer(entriesAt + entriesBytes(entries.length, units) + folderUnits * 2);
  new Uint32Array(buffer, 0, 8).set([
    ...header,
    units,
    folders.length,
    firstFolder,
    folderUnits,
    0,
  ]);
  const ends: number[] = [];
  for (const folder of folders) ends.push((ends.at(-1) ?? 0) + folder.length);
  new Uint32Array(buffer, HEADER_BYTES * 2, count).set([...words, ...ends]);
  writeEntries(buffer, entriesAt, entries);
  const at = buffer.byteLength - folderUnits * 2;
  new Uint16Array(buffer, at, folderUnits).set(stringUnits(folders.join("")));
  return buffer;
}

/** Results `firstId…` in folders `folderIds`, naming folders `firstFolder…` for the first time. */
export function searchBatch(
  firstId: number,
  entries: TestEntry[],
  folderIds: number[],
  firstFolder: number,
  folders: string[],
): ArrayBuffer {
  return resultsMessage([1, entries.length, firstId], folderIds, entries, firstFolder, folders);
}

/** Results `ids` as they are now (`entries[i]`, in folder `folderIds[i]`) and `removed`. */
export function searchDiff(
  ids: number[],
  removed: number[],
  entries: TestEntry[],
  folderIds: number[],
  firstFolder = 0,
  folders: string[] = [],
): ArrayBuffer {
  return resultsMessage(
    [6, ids.length, removed.length],
    [...ids, ...removed, ...folderIds],
    entries,
    firstFolder,
    folders,
  );
}

export function searchProgress(found: number, checked: number, skipped = 0): ArrayBuffer {
  const buffer = new ArrayBuffer(HEADER_BYTES);
  new Uint32Array(buffer, 0, 4).set([2, found, checked, skipped]);
  return buffer;
}

export function searchDone(found: number, rustMs: number, stopped = false, skipped = 0) {
  const buffer = new ArrayBuffer(HEADER_BYTES + 16);
  new Uint32Array(buffer, 0, 4).set([3, found, found * 10, skipped]);
  new Float64Array(buffer, HEADER_BYTES, 1).set([rustMs]);
  new Uint32Array(buffer, HEADER_BYTES + 8, 1).set([stopped ? 1 : 0]);
  return buffer;
}

/** Results' order; `column` is the column's code (0 name … 6 location). */
export function searchOrder(
  ids: number[],
  { column = 0, descending = false, foldersFirst = true } = {},
): ArrayBuffer {
  const buffer = new ArrayBuffer(HEADER_BYTES + ids.length * 4);
  const flags = (descending ? 1 : 0) | (foldersFirst ? 2 : 0);
  new Uint32Array(buffer, 0, 4).set([5, ids.length, column, flags]);
  new Uint32Array(buffer, HEADER_BYTES, ids.length).set(ids);
  return buffer;
}
