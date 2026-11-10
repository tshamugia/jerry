import { commands, type JerryError, type SearchIn, type SearchQuery } from "./bindings";
import { HEADER_BYTES, messageHeader, stream } from "./channel";
import { decodeEntries, type ListingEntries } from "./listing";

/**
 * Deep search (#65). `search_start` streams binary messages (ADR-0001, layout in
 * `src-tauri/src/search.rs`): batches of results, progress, and one done or error message last.
 * The results tab is #66.
 */

/** Message kinds: the header's first `u32`. */
export const SearchKind = { batch: 1, progress: 2, done: 3, error: 4 } as const;

/** Results `firstId` to `firstId + names.length - 1`, found since the last batch. */
export interface SearchBatch extends ListingEntries {
  kind: "batch";
  firstId: number;
  /** The folder of each result, by folder id. */
  folderIds: Uint32Array;
  /** Folders named for the first time: folder `firstFolder + i` is `folders[i]`, display text. */
  firstFolder: number;
  folders: string[];
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

/** The search is over: nothing follows. */
export interface SearchDone extends SearchCounts {
  kind: "done";
  /** How long the search took in Rust. */
  rustMs: number;
}

/** The folder couldn't be searched (`NotFound`, `AccessDenied`, `DeviceGone`); nothing follows. */
export interface SearchFailed {
  kind: "error";
  error: JerryError;
}

export type SearchMessage = SearchBatch | SearchProgress | SearchDone | SearchFailed;

/** A running search. */
export interface RunningSearch {
  id: number;
  /** Stops the search; messages still on their way are dropped. */
  cancel: () => Promise<void>;
}

/**
 * Searches `within` for `query` and calls `onMessage` with each message, in order: batches and
 * progress, then one done or error message. Resolves once the search has started. Fails only for
 * a folder token Jerry never gave; errors searching the folder arrive as an error message.
 */
export async function startSearch(
  within: SearchIn,
  query: SearchQuery,
  onMessage: (message: SearchMessage) => void,
): Promise<{ status: "ok"; data: RunningSearch } | { status: "error"; error: JerryError }> {
  let open = true;
  const channel = stream<ArrayBuffer>((message) => {
    if (open) {
      onMessage(decodeSearchMessage(message));
    }
  });
  const result = await commands.searchStart(within, query, channel);
  if (result.status === "error") {
    return result;
  }
  const id = result.data;
  const cancel = () => {
    open = false;
    return commands.searchCancel(id);
  };
  return { status: "ok", data: { id, cancel } };
}

// `ignoreBOM` keeps a leading U+FEFF: it can start a folder's name.
const utf16 = new TextDecoder("utf-16le", { ignoreBOM: true });
const utf8 = new TextDecoder();

/** Decodes one search message. Throws on a kind this UI doesn't know. */
export function decodeSearchMessage(message: ArrayBuffer): SearchMessage {
  const [kind, a = 0, b = 0, c = 0] = messageHeader(message);
  switch (kind) {
    case SearchKind.batch:
      return decodeBatch(message, a, b, c);
    case SearchKind.progress:
      return { kind: "progress", found: a, checked: b, skipped: c };
    case SearchKind.done:
      return {
        kind: "done",
        found: a,
        checked: b,
        skipped: c,
        rustMs: new Float64Array(message, HEADER_BYTES, 1)[0] ?? 0,
      };
    case SearchKind.error: {
      const json = utf8.decode(new Uint8Array(message, HEADER_BYTES, a));
      return { kind: "error", error: JSON.parse(json) as JerryError };
    }
    default:
      throw new RangeError(`Unknown search message kind ${kind}`);
  }
}

function decodeBatch(
  message: ArrayBuffer,
  count: number,
  firstId: number,
  nameUnits: number,
): SearchBatch {
  const [folderCount = 0, firstFolder = 0, folderUnits = 0] = new Uint32Array(
    message,
    HEADER_BYTES,
    3,
  );
  let offset = HEADER_BYTES * 2;
  const folderIds = new Uint32Array(message, offset, count);
  offset += count * Uint32Array.BYTES_PER_ELEMENT;
  const folderEnds = new Uint32Array(message, offset, folderCount);
  // Padded so the entries' f64 columns start on 8 bytes.
  const words = count + folderCount;
  offset += (folderCount + (words % 2)) * Uint32Array.BYTES_PER_ELEMENT;
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
  return { kind: "batch", firstId, folderIds, firstFolder, folders, ...entries };
}
