import { pageLoad, protocolUrl } from "./protocol";

/**
 * Thumbnails from Jerry's `thumb://` protocol (`src-tauri/src/thumbnails.rs`): pictures, PDFs,
 * drawings and whatever else Windows makes thumbnails of, as Explorer shows them. A file with
 * none (or a cloud-only file whose thumbnail Windows doesn't have) answers 404: keep showing its
 * type icon, as `ui/Thumbnail` does. Sizes are CSS px, the box the thumbnail fits in; the URL
 * asks for device pixels. Build a new URL when `devicePixelRatio` changes.
 */

/** Thumbnail sizes the core serves, in device pixels. */
const MIN_PX = 16;
const MAX_PX = 1024;

const FILE_ATTRIBUTE_DIRECTORY = 0x10;

/** `cssPx` in device pixels at `ratio`, within what the core serves. */
export function thumbPixels(cssPx: number, ratio = globalThis.devicePixelRatio || 1): number {
  return Math.min(MAX_PX, Math.max(MIN_PX, Math.round(cssPx * ratio)));
}

function thumbUrl(thumb: string, cssPx: number, query: string): string {
  return `${protocolUrl("thumb", `${thumb}-${thumbPixels(cssPx)}`)}?v=${pageLoad}&${query}`;
}

/** A listing entry, as its batch describes it. */
export interface ThumbEntry {
  /** The listing's id (`OpenListing.id`). */
  listing: number;
  /** The entry's id: the batch's `firstId + i`. */
  id: number;
  /** The batch's `modified[i]`: a changed file gets a new URL. */
  modified: number;
  /** The batch's `attributes[i]`: a file made online-only, or brought back, too. */
  attributes: number;
}

/** A listing entry's thumbnail; none for a folder. */
export function entryThumbUrl(entry: ThumbEntry, cssPx: number): string | undefined {
  if (entry.attributes & FILE_ATTRIBUTE_DIRECTORY) {
    return undefined;
  }
  const version = `m=${Math.trunc(entry.modified)}&a=${entry.attributes >>> 0}`;
  return thumbUrl(`entry-${entry.listing}-${entry.id}`, cssPx, version);
}

/** A side of a transfer conflict: what comes, or what is already there. */
export type ConflictSide = "incoming" | "existing";

/**
 * A side of the conflict transfer job `job` asks about. `question` tells one of its questions
 * from the next (the conflict as the queue lists it): the URL changes with it.
 */
export function conflictThumbUrl(
  job: number,
  side: ConflictSide,
  question: string,
  cssPx: number,
): string {
  return thumbUrl(`conflict-${job}-${side}`, cssPx, `q=${fnv1a(question)}`);
}

/** A 32-bit FNV-1a hash of `text`, in hex: short, and the same for the same text. */
function fnv1a(text: string): string {
  let hash = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}
