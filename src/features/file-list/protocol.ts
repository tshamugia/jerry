import type { IpcErrorCode, ListingStats } from "../../ipc";

/**
 * Decoder for the binary listing messages (format in `src-tauri/src/listing.rs`). Every message
 * starts with a 16-byte header, so the typed-array views below are aligned and copy nothing.
 */

export const MSG_BATCH = 1;
export const MSG_ORDER = 2;
export const MSG_DONE = 3;
export const MSG_ERROR = 4;

export interface BatchMessage {
  kind: "batch";
  firstId: number;
  count: number;
  names: string[];
  /** Bytes, exact up to 2^53. */
  sizes: Float64Array;
  /** Last write time, ms since the Unix epoch. */
  modified: Float64Array;
  attributes: Uint32Array;
}

export interface OrderMessage {
  kind: "order";
  /** Entry ids in display order. */
  ids: Uint32Array;
  rustMs: number;
}

export interface DoneMessage {
  kind: "done";
  stats: ListingStats;
}

export interface ErrorMessage {
  kind: "error";
  code: IpcErrorCode;
  os: number;
}

export type ListingMessage = BatchMessage | OrderMessage | DoneMessage | ErrorMessage;

const ERROR_CODES: IpcErrorCode[] = ["AccessDenied", "NotFound", "Cancelled", "Os"];

const utf16 = new TextDecoder("utf-16le");

export function decodeMessage(buffer: ArrayBuffer): ListingMessage {
  const header = new Uint32Array(buffer, 0, 4);
  const kind = header[0];
  const count = header[1] ?? 0;
  switch (kind) {
    case MSG_BATCH:
      return decodeBatch(buffer, count, header[2] ?? 0, header[3] ?? 0);
    case MSG_ORDER:
      return {
        kind: "order",
        ids: new Uint32Array(buffer, 16, count),
        rustMs: new Float64Array(buffer, 8, 1)[0] ?? 0,
      };
    case MSG_DONE: {
      const f = new Float64Array(buffer, 16, 8);
      return {
        kind: "done",
        stats: {
          count,
          batches: header[2] ?? 0,
          bytes: f[0] ?? 0,
          firstBatchMs: f[1] ?? 0,
          listMs: f[2] ?? 0,
          keysMs: f[3] ?? 0,
          nameSortMs: f[4] ?? 0,
          ranksMs: f[5] ?? 0,
          orderMs: f[6] ?? 0,
          totalMs: f[7] ?? 0,
        },
      };
    }
    case MSG_ERROR:
      return { kind: "error", code: ERROR_CODES[count] ?? "Os", os: header[2] ?? 0 };
    default:
      throw new Error(`unknown listing message ${kind}`);
  }
}

function decodeBatch(
  buffer: ArrayBuffer,
  count: number,
  firstId: number,
  nameUnits: number,
): BatchMessage {
  let offset = 16;
  const sizes = new Float64Array(buffer, offset, count);
  offset += count * 8;
  const modified = new Float64Array(buffer, offset, count);
  offset += count * 8;
  const attributes = new Uint32Array(buffer, offset, count);
  offset += count * 4;
  const ends = new Uint32Array(buffer, offset, count);
  offset += count * 4;
  // One decode for the whole block, then cheap slices. An unpaired surrogate decodes to U+FFFD,
  // one unit for one unit, so the offsets stay valid.
  const text = utf16.decode(new Uint16Array(buffer, offset, nameUnits));
  const names = new Array<string>(count);
  let start = 0;
  for (let i = 0; i < count; i++) {
    const end = ends[i] ?? start;
    names[i] = text.slice(start, end);
    start = end;
  }
  return { kind: "batch", firstId, count, names, sizes, modified, attributes };
}
