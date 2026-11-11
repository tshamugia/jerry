import {
  commands,
  type JerryError,
  type SearchIn,
  type SearchOpened,
  type SearchQuery,
  type Sort,
} from "./bindings";
import { HEADER_BYTES, messageHeader, stream } from "./channel";
import { decodeEntries, decodeOrder, type ListingEntries, type ListingOrder } from "./listing";

/**
 * Deep search (#65, #66). `search_start` streams binary messages (ADR-0001, layout in
 * `src-tauri/src/search.rs`): batches of results and progress, then their order and the done
 * message, or an error. The results stay for the tab that searched, under an id of a listing's
 * kind: every command on entries takes them, and while they are open a diff and an order follow
 * every change Jerry makes to them (a delete, a rename), and an order every sort.
 */

/** Message kinds: the header's first `u32`. */
export const SearchKind = { batch: 1, progress: 2, done: 3, error: 4, order: 5, diff: 6 } as const;

/** Results with their folders: each result's folder id, and the folders named for the first time. */
export interface ResultFolders {
  /** The folder of each result, by folder id. */
  folderIds: Uint32Array;
  /** Folders named for the first time: folder `firstFolder + i` is `folders[i]`, display text. */
  firstFolder: number;
  folders: string[];
}

/** Results `firstId` to `firstId + names.length - 1`, found since the last batch. */
export interface SearchBatch extends ListingEntries, ResultFolders {
  kind: "batch";
  firstId: number;
}

/**
 * Results Jerry changed: renamed (`ids[i]` is result `i` as it is now; one in a renamed folder has
 * the folder's new id) and removed (deleted, moved away; gone for good). An order follows once
 * the search is done.
 */
export interface SearchDiff extends ListingEntries, ResultFolders {
  kind: "diff";
  ids: Uint32Array;
  removed: Uint32Array;
}

/** How far the search is. */
export interface SearchCounts {
  /** Results so far. */
  found: number;
  /** Files and folders read so far, matching or not. */
  checked: number;
  /** Folders that couldn't be read: no permission, or gone meanwhile. */
  skipped: number;
}

/** After each batch, and every 50 ms while the counts change. */
export interface SearchProgress extends SearchCounts {
  kind: "progress";
}

/** The search is over; diffs and orders still follow while the results are open. */
export interface SearchDone extends SearchCounts {
  kind: "done";
  /** How long the search took in Rust. */
  rustMs: number;
  /** Stop ended it: the results are what it found until then. */
  stopped: boolean;
}

/** The folder couldn't be searched (`NotFound`, `AccessDenied`, `DeviceGone`); nothing follows. */
export interface SearchFailed {
  kind: "error";
  error: JerryError;
}

export type SearchMessage =
  | SearchBatch
  | SearchProgress
  | SearchDone
  | SearchFailed
  | ListingOrder
  | SearchDiff;

/** A search started: its results' id and the folder searched. Call `close` when its tab moves on. */
export interface OpenSearch extends SearchOpened {
  /**
   * Sorts the results by `sort` from now on; the order comes through the stream. While the
   * search goes on, later results follow unsorted until the order at its end.
   */
  sort: (
    sort: Sort,
  ) => Promise<{ status: "ok"; data: null } | { status: "error"; error: JerryError }>;
  /** Stop: what was found stays, and its order and the done message follow. */
  stop: () => Promise<void>;
  /** Forgets the results; messages still on their way are dropped. */
  close: () => Promise<void>;
}

/**
 * Searches `within` for `query`, for tab `tab` in place of what it shows, and calls `onMessage`
 * with each message in order: batches and progress, then the order (by `sort`) and the done
 * message, or an error; then a diff and an order for every change Jerry makes to the results.
 * Resolves once the search has started. Fails only for a folder token Jerry never gave; errors
 * searching the folder arrive as an error message.
 */
export async function startSearch(
  tab: number,
  within: SearchIn,
  query: SearchQuery,
  sort: Sort,
  onMessage: (message: SearchMessage) => void,
): Promise<{ status: "ok"; data: OpenSearch } | { status: "error"; error: JerryError }> {
  let open = true;
  const channel = stream<ArrayBuffer>((message) => {
    if (open) {
      onMessage(decodeSearchMessage(message));
    }
  });
  const result = await commands.searchStart(tab, within, query, sort, channel);
  if (result.status === "error") {
    return result;
  }
  const { id } = result.data;
  const resort = (next: Sort) => commands.listingSort(id, next);
  const stop = () => commands.searchCancel(id);
  const close = () => {
    open = false;
    return commands.listingClose(id);
  };
  return { status: "ok", data: { ...result.data, sort: resort, stop, close } };
}

// `ignoreBOM` keeps a leading U+FEFF: it can start a folder's name.
const utf16 = new TextDecoder("utf-16le", { ignoreBOM: true });
const utf8 = new TextDecoder();

/** Decodes one search message. Throws on a kind this UI doesn't know. */
export function decodeSearchMessage(message: ArrayBuffer): SearchMessage {
  const [kind, a = 0, b = 0, c = 0] = messageHeader(message);
  switch (kind) {
    case SearchKind.batch:
      return { kind: "batch", firstId: b, ...decodeResults(message, a, 0, c, false).results };
    case SearchKind.diff: {
      const { ids, removed, results } = decodeResults(message, a, b, c, true);
      return { kind: "diff", ids, removed, ...results };
    }
    case SearchKind.progress:
      return { kind: "progress", found: a, checked: b, skipped: c };
    case SearchKind.done:
      return {
        kind: "done",
        found: a,
        checked: b,
        skipped: c,
        rustMs: new Float64Array(message, HEADER_BYTES, 1)[0] ?? 0,
        stopped: (new Uint32Array(message, HEADER_BYTES + 8, 1)[0] ?? 0) !== 0,
      };
    case SearchKind.error: {
      const json = utf8.decode(new Uint8Array(message, HEADER_BYTES, a));
      return { kind: "error", error: JSON.parse(json) as JerryError };
    }
    case SearchKind.order:
      return decodeOrder(message, SearchKind.order);
    default:
      throw new RangeError(`Unknown search message kind ${kind}`);
  }
}

/**
 * A batch's or a diff's body: the folder row, the ids and removed ids (a diff's), each result's
 * folder id, the new folders' ends, padding, the results and the new folders' texts.
 */
function decodeResults(
  message: ArrayBuffer,
  count: number,
  removedCount: number,
  nameUnits: number,
  diff: boolean,
): { ids: Uint32Array; removed: Uint32Array; results: ListingEntries & ResultFolders } {
  const [folderCount = 0, firstFolder = 0, folderUnits = 0] = new Uint32Array(
    message,
    HEADER_BYTES,
    3,
  );
  let offset = HEADER_BYTES * 2;
  const take = (length: number) => {
    const words = new Uint32Array(message, offset, length);
    offset += length * Uint32Array.BYTES_PER_ELEMENT;
    return words;
  };
  const ids = diff ? take(count) : new Uint32Array(0);
  const removed = diff ? take(removedCount) : new Uint32Array(0);
  const folderIds = take(count);
  const folderEnds = take(folderCount);
  // Padded so the entries' f64 columns start on 8 bytes.
  const words = (diff ? 2 * count + removedCount : count) + folderCount;
  offset += (words % 2) * Uint32Array.BYTES_PER_ELEMENT;
  const entries = decodeEntries(message, offset, count, nameUnits);
  // The folders' texts are last.
  const textsAt = message.byteLength - folderUnits * Uint16Array.BYTES_PER_ELEMENT;
  const text = utf16.decode(new Uint16Array(message, textsAt, folderUnits));
  const folders = new Array<string>(folderCount);
  let start = 0;
  for (let i = 0; i < folderCount; i++) {
    const end = folderEnds[i] ?? start;
    folders[i] = text.slice(start, end);
    start = end;
  }
  return { ids, removed, results: { folderIds, firstFolder, folders, ...entries } };
}
