import type { Channel } from "@tauri-apps/api/core";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { ListingOpened, ListingTarget, Sort } from "./bindings";
import { HEADER_BYTES } from "./channel";

/**
 * Tests only: listing messages built exactly as `src-tauri/src/listing.rs` lays them out, so
 * UI tests can feed a fake `listing_open` channel.
 */

/** A command the UI sent to the fake core. */
export interface Call {
  cmd: string;
  args: Record<string, unknown>;
}

export interface FakeListings {
  /** Every command the UI sent, in order. */
  calls: Call[];
  /**
   * Puts `message` on a listing's stream now, as the watcher would: the last opened one, or
   * listing `id`'s.
   */
  send: (message: ArrayBuffer, id?: number) => void;
}

/**
 * A fake core for listings. `listing_open` answers with the next id (from 1), `path`, its last
 * part as the name, a new folder token (100 + id) and no parent, or with what
 * `opened(target, id)` says instead; then it streams `messages(target)` on a timer.
 * `listing_sort` sends `sorted(sort)` through the same stream, as Rust does. Other commands answer
 * what `other` returns for them (a rejection is a command's error), or `null`.
 */
export function fakeListings({
  messages = () => [],
  sorted = () => undefined,
  path = String.raw`C:\Work`,
  opened = () => ({}),
  other = () => null,
}: {
  messages?: (target: ListingTarget) => ArrayBuffer[];
  sorted?: (sort: Sort) => ArrayBuffer | undefined;
  path?: string;
  opened?: (target: ListingTarget, id: number) => Partial<ListingOpened>;
  other?: (cmd: string, args: Record<string, unknown>) => unknown;
} = {}): FakeListings {
  const calls: Call[] = [];
  // Each listing's stream, by id - 1.
  const streams: Channel<ArrayBuffer>[] = [];
  mockIPC((cmd, args) => {
    const payload = (args ?? {}) as Record<string, unknown>;
    calls.push({ cmd, args: payload });
    if (cmd === "listing_open") {
      const stream = payload.onMessage as Channel<ArrayBuffer>;
      streams.push(stream);
      const queued = messages(payload.target as ListingTarget);
      setTimeout(() => {
        for (const message of queued) stream.onmessage(message);
      }, 0);
      const id = streams.length;
      const name = path.split("\\").filter(Boolean).at(-1) ?? path;
      const answer: ListingOpened = {
        id,
        path,
        name,
        folder: 100 + id,
        parent: null,
        crumbs: [{ name, folder: 100 + id }],
      };
      return { ...answer, ...opened(payload.target as ListingTarget, id) };
    }
    if (cmd === "listing_sort") {
      const order = sorted(payload.sort as Sort);
      const stream = streams[(payload.id as number) - 1];
      if (order && stream) {
        setTimeout(() => stream.onmessage(order), 0);
      }
      return null;
    }
    return other(cmd, payload);
  });
  return {
    calls,
    send: (message, id = streams.length) => streams[id - 1]?.onmessage(message),
  };
}

export interface TestEntry {
  name: string;
  size?: number;
  modified?: number;
  created?: number;
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
  return count * 36 + units * 2;
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
  new Float64Array(buffer, offset, count).set(entries.map((entry) => entry.created ?? 0));
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
