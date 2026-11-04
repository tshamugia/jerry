import { HEADER_BYTES } from "./channel";

/**
 * Tests only: listing messages built exactly as `src-tauri/src/listing.rs` lays them out, so
 * UI tests can feed a fake `listing_open` channel.
 */

export interface TestEntry {
  name: string;
  size?: number;
  modified?: number;
  attributes?: number;
  /** Defaults to the folder key for folders, a type key otherwise. */
  iconKey?: number;
}

export function batchMessage(firstId: number, entries: TestEntry[]): ArrayBuffer {
  const units = unitCount(entries);
  const buffer = new ArrayBuffer(HEADER_BYTES + entriesBytes(entries.length, units));
  new Uint32Array(buffer, 0, 4).set([1, entries.length, firstId, units]);
  writeEntries(buffer, HEADER_BYTES, entries);
  return buffer;
}

/** A diff message: `entries[i]` has id `ids[i]`. */
export function diffMessage(ids: number[], removed: number[], entries: TestEntry[]): ArrayBuffer {
  const units = unitCount(entries);
  const words = ids.length + removed.length;
  const entriesAt = HEADER_BYTES + (words + (words % 2)) * 4;
  const buffer = new ArrayBuffer(entriesAt + entriesBytes(entries.length, units));
  new Uint32Array(buffer, 0, 4).set([5, ids.length, removed.length, units]);
  new Uint32Array(buffer, HEADER_BYTES, words).set([...ids, ...removed]);
  writeEntries(buffer, entriesAt, entries);
  return buffer;
}

/** All names' UTF-16 units: a string's `length` counts them. */
function unitCount(entries: TestEntry[]): number {
  return entries.reduce((count, { name }) => count + name.length, 0);
}

function entriesBytes(count: number, units: number): number {
  return count * 28 + units * 2;
}

function writeEntries(buffer: ArrayBuffer, at: number, entries: TestEntry[]) {
  const count = entries.length;
  const units: number[] = [];
  const ends: number[] = [];
  for (const { name } of entries) {
    for (let i = 0; i < name.length; i++) units.push(name.charCodeAt(i));
    ends.push(units.length);
  }
  let offset = at;
  new Float64Array(buffer, offset, count).set(entries.map((entry) => entry.size ?? 0));
  offset += count * 8;
  new Float64Array(buffer, offset, count).set(entries.map((entry) => entry.modified ?? 0));
  offset += count * 8;
  const attributes = entries.map((entry) => entry.attributes ?? 0x20);
  new Uint32Array(buffer, offset, count).set(attributes);
  offset += count * 4;
  new Uint32Array(buffer, offset, count).set(
    entries.map((entry, i) => entry.iconKey ?? ((attributes[i] ?? 0) & 0x10 ? 1 : 3)),
  );
  offset += count * 4;
  new Uint32Array(buffer, offset, count).set(ends);
  offset += count * 4;
  new Uint16Array(buffer, offset, units.length).set(units);
}

/** An order message; `column` is the column's code (0 name … 5 extension). */
export function orderMessage(
  ids: number[],
  { column = 0, descending = false, foldersFirst = true } = {},
): ArrayBuffer {
  const buffer = new ArrayBuffer(HEADER_BYTES + ids.length * 4);
  const flags = (descending ? 1 : 0) | (foldersFirst ? 2 : 0);
  new Uint32Array(buffer, 0, 4).set([2, ids.length, column, flags]);
  new Uint32Array(buffer, HEADER_BYTES, ids.length).set(ids);
  return buffer;
}

export function doneMessage(count: number, batches: number, rustMs: number): ArrayBuffer {
  const buffer = new ArrayBuffer(HEADER_BYTES + 8);
  new Uint32Array(buffer, 0, 3).set([3, count, batches]);
  new Float64Array(buffer, HEADER_BYTES, 1).set([rustMs]);
  return buffer;
}

export function errorMessage(json: string): ArrayBuffer {
  const bytes = new TextEncoder().encode(json);
  const buffer = new ArrayBuffer(HEADER_BYTES + bytes.length);
  new Uint32Array(buffer, 0, 2).set([4, bytes.length]);
  new Uint8Array(buffer, HEADER_BYTES).set(bytes);
  return buffer;
}
