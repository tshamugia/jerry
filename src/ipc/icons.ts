import { pageLoad, protocolUrl } from "./protocol";

/**
 * Icons from Jerry's `icon://` protocol (`src-tauri/src/icons.rs`): listing entries by the icon
 * key their batch carries, drives and standard places. Sizes are CSS px; the URL asks for device
 * pixels, so icons stay sharp at every DPI. Build a new URL when `devicePixelRatio` changes.
 */

/** Fixed icon keys; from `firstType` on there is one per extension, numbered by the core. */
export const IconKey = { own: 0, folder: 1, file: 2, firstType: 3 } as const;

/** Standard places for the sidebar. */
export type Place =
  | "desktop"
  | "documents"
  | "downloads"
  | "pictures"
  | "onedrive"
  | "recyclebin"
  | "share";

/** Icon sizes the core serves, in device pixels. */
const MIN_PX = 8;
const MAX_PX = 256;

/** `cssPx` in device pixels at `ratio`, within what the core serves. */
export function iconPixels(cssPx: number, ratio = globalThis.devicePixelRatio || 1): number {
  return Math.min(MAX_PX, Math.max(MIN_PX, Math.round(cssPx * ratio)));
}

function iconUrl(icon: string, cssPx: number): string {
  return `${protocolUrl("icon", `${icon}-${iconPixels(cssPx)}`)}?v=${pageLoad}`;
}

/** The icon of every file of a type: `key` from a listing batch, not `IconKey.own`. */
export function typeIconUrl(key: number, cssPx: number): string {
  return iconUrl(`type-${key}`, cssPx);
}

/** A listing entry, as its batch describes it. */
export interface IconEntry {
  /** The listing's id (`OpenListing.id`). */
  listing: number;
  /** The entry's id: the batch's `firstId + i`. */
  id: number;
  /** The batch's `iconKeys[i]`. */
  key: number;
  /** The batch's `modified[i]`: a changed file gets a new URL. */
  modified: number;
}

/**
 * A listing entry's icon: its own (.exe, .lnk, .ico, .url) when its key is `IconKey.own`, else
 * its type's, which every file of the type shares.
 */
export function entryIconUrl(entry: IconEntry, cssPx: number): string {
  if (entry.key !== IconKey.own) {
    return typeIconUrl(entry.key, cssPx);
  }
  return `${iconUrl(`entry-${entry.listing}-${entry.id}`, cssPx)}&m=${Math.trunc(entry.modified)}`;
}

/** A drive's icon, by letter ("C"). */
export function driveIconUrl(letter: string, cssPx: number): string {
  return iconUrl(`drive-${letter}`, cssPx);
}

/** A standard place's icon. Recycle Bin's shows whether it is full; ask again to update it. */
export function placeIconUrl(place: Place, cssPx: number): string {
  return iconUrl(`place-${place}`, cssPx);
}
