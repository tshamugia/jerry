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
}

export function batchMessage(firstId: number, entries: TestEntry[]): ArrayBuffer {
  const count = entries.length;
  const units: number[] = [];
  const ends: number[] = [];
  for (const { name } of entries) {
    for (let i = 0; i < name.length; i++) units.push(name.charCodeAt(i));
    ends.push(units.length);
  }
  const buffer = new ArrayBuffer(HEADER_BYTES + count * 24 + units.length * 2);
  new Uint32Array(buffer, 0, 4).set([1, count, firstId, units.length]);
  let offset = HEADER_BYTES;
  new Float64Array(buffer, offset, count).set(entries.map((entry) => entry.size ?? 0));
  offset += count * 8;
  new Float64Array(buffer, offset, count).set(entries.map((entry) => entry.modified ?? 0));
  offset += count * 8;
  new Uint32Array(buffer, offset, count).set(entries.map((entry) => entry.attributes ?? 0x20));
  offset += count * 4;
  new Uint32Array(buffer, offset, count).set(ends);
  offset += count * 4;
  new Uint16Array(buffer, offset, units.length).set(units);
  return buffer;
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
