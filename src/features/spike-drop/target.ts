import type { TargetKind } from "../../ipc";

/**
 * Spike #14: what a point on the page means for a drop.
 *
 * Every drop target in the DOM carries `data-drop` (its kind), `data-drop-key` (unique per
 * element: the same folder can be a tab, a breadcrumb segment and a sidebar item at once),
 * `data-folder` (the folder id a drop there goes to) and `data-name` (for "Move to %1").
 * `data-drop="none"` marks places that refuse drops outright.
 */
export type DropHit = {
  key: string;
  kind: TargetKind;
  folder: number;
  name: string;
};

const KINDS: readonly TargetKind[] = ["row", "tab", "crumb", "sidebar", "pane"];

/** The drop target an element belongs to, or `null` where nothing accepts a drop. */
export function dropHit(element: Element | null): DropHit | null {
  const target = element?.closest<HTMLElement>("[data-drop]");
  if (!target) return null;
  const { drop, dropKey, folder, name } = target.dataset;
  const kind = KINDS.find((k) => k === drop);
  if (!kind || dropKey === undefined || folder === undefined) return null;
  return { key: dropKey, kind, folder: Number(folder), name: name ?? "" };
}

/** The drop target under a point OLE reports in client pixels. */
export function dropHitAt(x: number, y: number, dpr = window.devicePixelRatio || 1) {
  return dropHit(document.elementFromPoint(x / dpr, y / dpr));
}
