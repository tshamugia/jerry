import {
  commands,
  type JerryError,
  type ListingOpened,
  type ListingTarget,
  type Sort,
  type SortColumn,
} from "./bindings";
import { HEADER_BYTES, messageHeader, stream } from "./channel";

/**
 * Folder listings. `listing_open` streams binary messages (ADR-0001, layout in
 * `src-tauri/src/listing.rs`): batches of entries as columns, then their order and one done
 * message; then, while the listing is open, a diff and a new order for every change in the
 * folder; or an error message. `listing_sort` sends its order through the same stream. Every
 * message starts with the 16-byte header, so the typed-array views below are aligned and copy
 * nothing.
 */

/** Message kinds: the header's first `u32`. */
export const ListingKind = { batch: 1, order: 2, done: 3, error: 4, diff: 5 } as const;

/** Columns in the order message's header, by their code. */
const SORT_COLUMNS: readonly SortColumn[] = [
  "name",
  "modified",
  "type",
  "size",
  "created",
  "extension",
  // Search results only (#66).
  "location",
];

/** Order flags: bit 0 descending, bit 1 folders first. */
const DESCENDING = 1;
const FOLDERS_FIRST = 2;

/**
 * Entries as columns: entry `i` is `names[i]`, `sizes[i]` and so on. Batches and diffs carry
 * them the same way.
 */
export interface ListingEntries {
  /** For display. An unpaired surrogate (a name that isn't valid Unicode) shows as U+FFFD. */
  names: string[];
  /** Bytes, exact up to 2^53. Folders are 0. */
  sizes: Float64Array;
  /** Last write time, ms since the Unix epoch. */
  modified: Float64Array;
  /** Creation time, ms since the Unix epoch. */
  created: Float64Array;
  /** `FILE_ATTRIBUTE_*` flags. */
  attributes: Uint32Array;
  /**
   * `icon://` keys (`./icons`): `IconKey.own` for an entry with its own icon (asked for by
   * entry), its type's key otherwise.
   */
  iconKeys: Uint32Array;
}

/** Entries `firstId` to `firstId + names.length - 1` of a listing. */
export interface ListingBatch extends ListingEntries {
  kind: "batch";
  firstId: number;
}

/**
 * What changed in the folder, collected over 100 ms. Entry `i` has id `ids[i]`, ascending: an id
 * the listing already has is that entry changed (a renamed entry keeps its id), the others are
 * new and continue the listing's ids in order. `removed` ids are gone for good. An order follows.
 */
export interface ListingDiff extends ListingEntries {
  kind: "diff";
  ids: Uint32Array;
  removed: Uint32Array;
}

/**
 * The entries the folder has, in display order by `sort`: entry ids, each once. The stream sends
 * one once the whole folder is in, right before done, one after every diff, and one for every
 * `sort` once the folder is in. Apply only an order whose `sort` is the tab's current one: an
 * order for an earlier sort can still be on its way.
 */
export interface ListingOrder {
  kind: "order";
  sort: Sort;
  ids: Uint32Array;
}

/** The whole folder has arrived. Diffs follow while the listing is open. */
export interface ListingDone {
  kind: "done";
  count: number;
  batches: number;
  /** How long the listing took in Rust. */
  rustMs: number;
}

/** Reading the folder failed, or the folder went away (`NotFound`, `DeviceGone`); nothing follows. */
export interface ListingFailed {
  kind: "error";
  error: JerryError;
}

export type ListingMessage =
  | ListingBatch
  | ListingOrder
  | ListingDone
  | ListingFailed
  | ListingDiff;

/** An open listing. Call `close` when its tab closes or goes to another folder. */
export interface OpenListing extends ListingOpened {
  /**
   * Sorts the listing by `sort` from now on. The new order comes through the stream, after any
   * diff before it; while the folder is still being read, the stream's own order follows the new
   * sort. Fails with `NotFound` once the listing is closed.
   */
  sort: (
    sort: Sort,
  ) => Promise<{ status: "ok"; data: null } | { status: "error"; error: JerryError }>;
  /** Stops the listing and its watcher; messages still on their way are dropped. */
  close: () => Promise<void>;
}

/**
 * Opens `target` for `tab`, sorted by `sort`, and calls `onMessage` with each message, in
 * order: batches, then the order and one done message, then a diff and an order for every change
 * in the folder, or an error message. Resolves once the listing has started, often after its
 * first batch has arrived. Fails only when `target` can't be used (a relative path, an entry of a
 * closed listing); errors reading the folder arrive as an error message.
 */
export async function openListing(
  tab: number,
  target: ListingTarget,
  sort: Sort,
  onMessage: (message: ListingMessage) => void,
): Promise<{ status: "ok"; data: OpenListing } | { status: "error"; error: JerryError }> {
  let open = true;
  const channel = stream<ArrayBuffer>((message) => {
    if (open) {
      onMessage(decodeListingMessage(message));
    }
  });
  const result = await commands.listingOpen(tab, target, sort, channel);
  if (result.status === "error") {
    return result;
  }
  const { id } = result.data;
  const resort = (next: Sort) => commands.listingSort(id, next);
  const close = () => {
    open = false;
    return commands.listingClose(id);
  };
  return { status: "ok", data: { ...result.data, sort: resort, close } };
}

// `ignoreBOM` keeps a leading U+FEFF: it can start a file name, and dropping it would shift the
// name offsets.
const utf16 = new TextDecoder("utf-16le", { ignoreBOM: true });
const utf8 = new TextDecoder();

/** Decodes one listing message. Throws on a kind this UI doesn't know. */
export function decodeListingMessage(message: ArrayBuffer): ListingMessage {
  const [kind, a = 0, b = 0, c = 0] = messageHeader(message);
  switch (kind) {
    case ListingKind.batch:
      return { kind: "batch", firstId: b, ...decodeEntries(message, HEADER_BYTES, a, c) };
    case ListingKind.diff:
      return decodeDiff(message, a, b, c);
    case ListingKind.order:
      return decodeOrder(message, ListingKind.order);
    case ListingKind.done:
      return {
        kind: "done",
        count: a,
        batches: b,
        rustMs: new Float64Array(message, HEADER_BYTES, 1)[0] ?? 0,
      };
    case ListingKind.error: {
      const json = utf8.decode(new Uint8Array(message, HEADER_BYTES, a));
      return { kind: "error", error: JSON.parse(json) as JerryError };
    }
    default:
      throw new RangeError(`Unknown listing message kind ${kind}`);
  }
}

/** An order message of kind `expected`: a listing's, or search results' (`./search`). */
export function decodeOrder(message: ArrayBuffer, expected: number): ListingOrder {
  const [kind, count = 0, column = 0, flags = 0] = messageHeader(message);
  const sortColumn = SORT_COLUMNS[column];
  if (kind !== expected || sortColumn === undefined) {
    throw new RangeError(`Not an order message: kind ${kind}, column ${column}`);
  }
  return {
    kind: "order",
    sort: {
      column: sortColumn,
      descending: (flags & DESCENDING) !== 0,
      foldersFirst: (flags & FOLDERS_FIRST) !== 0,
    },
    ids: new Uint32Array(message, HEADER_BYTES, count),
  };
}

function decodeDiff(
  message: ArrayBuffer,
  count: number,
  removedCount: number,
  nameUnits: number,
): ListingDiff {
  const ids = new Uint32Array(message, HEADER_BYTES, count);
  const removed = new Uint32Array(
    message,
    HEADER_BYTES + count * Uint32Array.BYTES_PER_ELEMENT,
    removedCount,
  );
  // Padded so the entries' f64 columns start on 8 bytes.
  const words = count + removedCount;
  const entriesAt = HEADER_BYTES + (words + (words % 2)) * Uint32Array.BYTES_PER_ELEMENT;
  return { kind: "diff", ids, removed, ...decodeEntries(message, entriesAt, count, nameUnits) };
}

/**
 * `count` entries laid out from `offset` on, as batches and diffs carry them, and search batches
 * (`./search`).
 */
export function decodeEntries(
  message: ArrayBuffer,
  offset: number,
  count: number,
  nameUnits: number,
): ListingEntries {
  const sizes = new Float64Array(message, offset, count);
  offset += count * Float64Array.BYTES_PER_ELEMENT;
  const modified = new Float64Array(message, offset, count);
  offset += count * Float64Array.BYTES_PER_ELEMENT;
  const created = new Float64Array(message, offset, count);
  offset += count * Float64Array.BYTES_PER_ELEMENT;
  const attributes = new Uint32Array(message, offset, count);
  offset += count * Uint32Array.BYTES_PER_ELEMENT;
  const iconKeys = new Uint32Array(message, offset, count);
  offset += count * Uint32Array.BYTES_PER_ELEMENT;
  const ends = new Uint32Array(message, offset, count);
  offset += count * Uint32Array.BYTES_PER_ELEMENT;
  // One decode for all names, then slices. An unpaired surrogate decodes to one U+FFFD, a unit
  // for a unit, so the ends stay valid.
  const text = utf16.decode(new Uint16Array(message, offset, nameUnits));
  const names = new Array<string>(count);
  let start = 0;
  for (let i = 0; i < count; i++) {
    const end = ends[i] ?? start;
    names[i] = text.slice(start, end);
    start = end;
  }
  return { names, sizes, modified, created, attributes, iconKeys };
}
