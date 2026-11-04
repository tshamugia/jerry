/**
 * The body's zones across the window: sidebar, the panes (two with a 1 px divider, or one in
 * single-pane mode), preview. The sidebar and preview keep the width the user dragged them to;
 * in a narrow window they hide on their own (preview first) before a pane gets narrower than
 * `MIN_PANE` (PRD «ინტერფეისის სტრუქტურა»). All sizes in CSS pixels.
 */

/** The narrowest a pane gets: its toolbar still fits. */
export const MIN_PANE = 240;
export const SIDEBAR_LIMITS = { min: 160, max: 400 } as const;
export const PREVIEW_LIMITS = { min: 200, max: 480 } as const;
const DIVIDER = 1;

/** The sizes the user chose (settings, or a drag in progress). */
export interface ZoneSizes {
  sidebar: number;
  preview: number;
  /** The left pane's share of the two panes, 0 to 1. */
  split: number;
}

/** The sizes as laid out in a window of a given width. */
export interface Zones {
  /** 0 when hidden. */
  sidebar: number;
  /** 0 when hidden. */
  preview: number;
  /** The left pane's share, so that both panes keep `MIN_PANE`; unused with one pane. */
  split: number;
  /** Two panes (the default), or one ("Dual-pane mode" off). */
  dual: boolean;
}

/** The narrowest the panes get together. */
function minPanes(dual: boolean): number {
  return dual ? 2 * MIN_PANE + DIVIDER : MIN_PANE;
}

export function layoutZones(width: number, wanted: ZoneSizes, dual = true): Zones {
  const sidebar = clamp(wanted.sidebar, SIDEBAR_LIMITS.min, SIDEBAR_LIMITS.max);
  const preview = clamp(wanted.preview, PREVIEW_LIMITS.min, PREVIEW_LIMITS.max);
  const panes = minPanes(dual);
  const showSidebar = width - sidebar >= panes;
  const showPreview = width - sidebar - preview >= panes;
  const zones = { sidebar: showSidebar ? sidebar : 0, preview: showPreview ? preview : 0 };
  return { ...zones, split: clampSplit(wanted.split, panesWidth(width, zones)), dual };
}

/** The grid columns for `--frame-columns`, shared by the title bar and the body. */
export function frameColumns(zones: Zones): string {
  const left = round(zones.split);
  const right = round(1 - left);
  return [
    zones.sidebar > 0 ? `${zones.sidebar}px` : "",
    zones.dual ? `minmax(0, ${left}fr) ${DIVIDER}px minmax(0, ${right}fr)` : "minmax(0, 1fr)",
    zones.preview > 0 ? `${zones.preview}px` : "",
  ]
    .join(" ")
    .trim();
}

/** The sidebar's width for a drag of its edge to `x`; the panes keep `MIN_PANE`. */
export function dragSidebar(x: number, width: number, zones: Zones): number {
  const max = Math.min(SIDEBAR_LIMITS.max, width - zones.preview - minPanes(zones.dual));
  return Math.round(clamp(x, SIDEBAR_LIMITS.min, max));
}

/** The preview's width for a drag of its edge to `x`; the panes keep `MIN_PANE`. */
export function dragPreview(x: number, width: number, zones: Zones): number {
  const max = Math.min(PREVIEW_LIMITS.max, width - zones.sidebar - minPanes(zones.dual));
  return Math.round(clamp(width - x, PREVIEW_LIMITS.min, max));
}

/** The left pane's share for a drag of the divider to `x`. */
export function dragSplit(x: number, width: number, zones: Zones): number {
  const panes = panesWidth(width, zones);
  return round(clampSplit((x - zones.sidebar) / panes, panes));
}

function panesWidth(width: number, zones: { sidebar: number; preview: number }): number {
  return width - zones.sidebar - zones.preview - DIVIDER;
}

function clampSplit(split: number, panes: number): number {
  if (!Number.isFinite(split) || panes < 2 * MIN_PANE) {
    return 0.5;
  }
  const min = MIN_PANE / panes;
  return clamp(split, min, 1 - min);
}

/** `value` within `min..max`; `min` wins if they cross. */
function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(value, max));
}

/** Three decimals: enough for a pixel at any window width, and no float noise in the settings. */
function round(share: number): number {
  return Math.round(share * 1000) / 1000;
}
