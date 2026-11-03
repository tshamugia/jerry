import { commands, type JerryError, type ListingOpened, type ListingTarget } from "./bindings";
import { HEADER_BYTES, messageHeader, stream } from "./channel";

/**
 * Folder listings. `listing_open` streams binary messages (ADR-0001, layout in
 * `src-tauri/src/listing.rs`): batches of entries as columns, then one done or error message.
 * Every message starts with the 16-byte header, so the typed-array views below are aligned and
 * copy nothing.
 */

/** Message kinds: the header's first `u32`. `order` arrives with sorting (#26). */
export const ListingKind = { batch: 1, order: 2, done: 3, error: 4 } as const;

/** Entries `firstId` to `firstId + names.length - 1` of a listing, as columns. */
export interface ListingBatch {
  kind: "batch";
  firstId: number;
  /** For display. An unpaired surrogate (a name that isn't valid Unicode) shows as U+FFFD. */
  names: string[];
  /** Bytes, exact up to 2^53. Folders are 0. */
  sizes: Float64Array;
  /** Last write time, ms since the Unix epoch. */
  modified: Float64Array;
  /** `FILE_ATTRIBUTE_*` flags. */
  attributes: Uint32Array;
}

/** The whole folder has arrived. */
export interface ListingDone {
  kind: "done";
  count: number;
  batches: number;
  /** How long the listing took in Rust. */
  rustMs: number;
}

/** Reading the folder failed; nothing follows. */
export interface ListingFailed {
  kind: "error";
  error: JerryError;
}

export type ListingMessage = ListingBatch | ListingDone | ListingFailed;

/** An open listing. Call `close` when its tab closes or goes to another folder. */
export interface OpenListing extends ListingOpened {
  /** Stops the listing; messages still on their way are dropped. */
  close: () => Promise<void>;
}

/**
 * Opens `target` for `tab` and calls `onMessage` with each message, in order: batches, then one
 * done or error message. Resolves once the listing has started, often after its first batch has
 * arrived. Fails only when `target` can't be used (a relative path, an entry of a closed
 * listing); errors reading the folder arrive as an error message.
 */
export async function openListing(
  tab: number,
  target: ListingTarget,
  onMessage: (message: ListingMessage) => void,
): Promise<{ status: "ok"; data: OpenListing } | { status: "error"; error: JerryError }> {
  let open = true;
  const channel = stream<ArrayBuffer>((message) => {
    if (open) {
      onMessage(decodeListingMessage(message));
    }
  });
  const result = await commands.listingOpen(tab, target, channel);
  if (result.status === "error") {
    return result;
  }
  const { id } = result.data;
  const close = () => {
    open = false;
    return commands.listingClose(id);
  };
  return { status: "ok", data: { ...result.data, close } };
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
      return decodeBatch(message, a, b, c);
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

function decodeBatch(
  message: ArrayBuffer,
  count: number,
  firstId: number,
  nameUnits: number,
): ListingBatch {
  let offset = HEADER_BYTES;
  const sizes = new Float64Array(message, offset, count);
  offset += count * Float64Array.BYTES_PER_ELEMENT;
  const modified = new Float64Array(message, offset, count);
  offset += count * Float64Array.BYTES_PER_ELEMENT;
  const attributes = new Uint32Array(message, offset, count);
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
  return { kind: "batch", firstId, names, sizes, modified, attributes };
}
