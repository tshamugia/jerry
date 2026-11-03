import { protocolUrl } from "./protocol";

/**
 * Icons from Jerry's `icon://` protocol (`src-tauri/src/icons.rs`): file types by the icon key a
 * listing batch carries, drives and standard places. Sizes are CSS px; the URL asks for device
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

/**
 * Unique to this page load. Keys are numbered per process, so an icon URL must never be served
 * from a cache filled by an earlier run; the core ignores it.
 */
const pageLoad = crypto.randomUUID().slice(0, 8);

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

/** A drive's icon, by letter ("C"). */
export function driveIconUrl(letter: string, cssPx: number): string {
  return iconUrl(`drive-${letter}`, cssPx);
}

/** A standard place's icon. Recycle Bin's shows whether it is full; ask again to update it. */
export function placeIconUrl(place: Place, cssPx: number): string {
  return iconUrl(`place-${place}`, cssPx);
}
