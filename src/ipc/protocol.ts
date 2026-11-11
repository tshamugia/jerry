import { convertFileSrc } from "@tauri-apps/api/core";

/** Jerry's custom protocols, which serve binary data to `<img>` (CSP allows only these). */
export type Protocol = "icon" | "thumb";

/**
 * Unique to this page load. Icon keys and listing ids are numbered per process, so a protocol
 * URL must never be served from a cache filled by an earlier run; the core ignores it.
 */
export const pageLoad = crypto.randomUUID().slice(0, 8);

/**
 * The URL for `key` on one of Jerry's protocols (`http://icon.localhost/<key>` in WebView2).
 * Keys are tokens the core issued, never file paths.
 */
export function protocolUrl(protocol: Protocol, key: string): string {
  return convertFileSrc(key, protocol);
}
