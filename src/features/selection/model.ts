import type { Selection } from "../../stores/folder";
import { wildcardMatcher } from "../filter/wildcard";

/**
 * The selection model, as Explorer behaves: pure functions from a selection, the rows the list
 * shows (entry ids in display order) and the input to the next selection. Rows are found by
 * entry id, never kept as indexes, so a sort or a change in the folder keeps the selection.
 * Every change makes a new `selected` set; one that changes nothing returns the same selection.
 */

export interface Modifiers {
  ctrl: boolean;
  shift: boolean;
}

export const NO_MODIFIERS: Modifiers = { ctrl: false, shift: false };

/** The row index of entry `id`, if the list shows it. */
export function indexOf(rows: Uint32Array, id: number | undefined): number | undefined {
  if (id === undefined) {
    return undefined;
  }
  const at = rows.indexOf(id);
  return at < 0 ? undefined : at;
}

/** The ids of rows `from` to `to`, both included, in either order. */
function span(rows: Uint32Array, from: number, to: number): Uint32Array {
  return rows.subarray(Math.min(from, to), Math.max(from, to) + 1);
}

/**
 * A click on row `index`: alone it selects only that row; Ctrl toggles it; Shift selects the
 * range from the anchor in place of the selection, Ctrl+Shift adds the range to it. Without
 * Shift the anchor moves to the row; the focus always does.
 */
export function clickRow(
  selection: Selection,
  rows: Uint32Array,
  index: number,
  { ctrl, shift }: Modifiers,
): Selection {
  const id = rows[index];
  if (id === undefined) {
    return selection;
  }
  if (shift) {
    // Without an anchor, the range starts at the focus, or else at the row itself.
    const from = indexOf(rows, selection.anchor) ?? indexOf(rows, selection.focused) ?? index;
    const range = span(rows, from, index);
    const selected = ctrl ? new Set(selection.selected) : new Set<number>();
    for (const each of range) {
      selected.add(each);
    }
    return { selected, anchor: rows[from], focused: id };
  }
  if (ctrl) {
    const selected = new Set(selection.selected);
    if (!selected.delete(id)) {
      selected.add(id);
    }
    return { selected, anchor: id, focused: id };
  }
  return { selected: new Set([id]), anchor: id, focused: id };
}

/**
 * The keyboard moving to row `index` (arrows, Home, End, PageUp, PageDown): alone it selects the
 * row as a click does, Shift selects the range from the anchor (Ctrl+Shift adds it), Ctrl moves
 * only the focus.
 */
export function moveTo(
  selection: Selection,
  rows: Uint32Array,
  index: number,
  modifiers: Modifiers,
): Selection {
  if (modifiers.ctrl && !modifiers.shift) {
    const id = rows[index];
    return id === undefined ? selection : { ...selection, focused: id };
  }
  return clickRow(selection, rows, index, modifiers);
}

/** The rows wholly in view, as indexes. */
export interface Page {
  first: number;
  last: number;
}

/** The keys that move through the list. */
export type MoveKey = "ArrowUp" | "ArrowDown" | "Home" | "End" | "PageUp" | "PageDown";

export function isMoveKey(key: string): key is MoveKey {
  return ["ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"].includes(key);
}

/**
 * The row `key` moves to from row `at` (none focused: the first) among `count` rows. PageDown
 * goes to the last row in view, or from there a page further; PageUp the same upwards.
 */
export function rowForKey(
  key: MoveKey,
  at: number | undefined,
  count: number,
  page: Page,
): number | undefined {
  if (count === 0) {
    return undefined;
  }
  const last = count - 1;
  if (key === "Home") return 0;
  if (key === "End") return last;
  if (at === undefined) return 0;
  const step = Math.max(1, page.last - page.first);
  switch (key) {
    case "ArrowUp":
      return Math.max(0, at - 1);
    case "ArrowDown":
      return Math.min(last, at + 1);
    case "PageUp":
      return Math.max(0, at > page.first ? page.first : at - step);
    case "PageDown":
      return Math.min(last, at < page.last ? page.last : at + step);
  }
}

/** Ctrl+Space: selects or deselects the focused row; the anchor moves to it. */
export function toggleFocused(selection: Selection, rows: Uint32Array): Selection {
  const at = indexOf(rows, selection.focused);
  return at === undefined ? selection : clickRow(selection, rows, at, { ctrl: true, shift: false });
}

/** Ctrl+A: every row the list shows; the focus and anchor stay. */
export function selectAll(selection: Selection, rows: Uint32Array): Selection {
  if (selection.selected.size === rows.length && rows.every((id) => selection.selected.has(id))) {
    return selection;
  }
  return { ...selection, selected: new Set(rows) };
}

/** Ctrl+I: the rows that aren't selected, and only those. */
export function invert(selection: Selection, rows: Uint32Array): Selection {
  const selected = new Set<number>();
  for (const id of rows) {
    if (!selection.selected.has(id)) {
      selected.add(id);
    }
  }
  return { ...selection, selected };
}

/** Esc: nothing selected; the focus and anchor stay. */
export function clear(selection: Selection): Selection {
  return selection.selected.size === 0 ? selection : { ...selection, selected: new Set() };
}

/**
 * Select by pattern: exactly the rows whose names match `pattern` (the quick filter's
 * `wildcardMatcher`: `*.pdf`, `*_rev*`, or a part of the name), in place of the selection.
 * The focus and anchor stay.
 */
export function selectMatching(
  selection: Selection,
  rows: Uint32Array,
  names: readonly string[],
  pattern: string,
): Selection {
  const matches = wildcardMatcher(pattern);
  const selected = new Set<number>();
  for (const id of rows) {
    if (matches(names[id] ?? "")) {
      selected.add(id);
    }
  }
  return { ...selection, selected };
}

/**
 * How a marquee combines with the selection it started from: alone it replaces it, Ctrl toggles
 * the rows it crosses, Shift adds them.
 */
export type MarqueeMode = "replace" | "toggle" | "add";

export function marqueeMode({ ctrl, shift }: Modifiers): MarqueeMode {
  return ctrl ? "toggle" : shift ? "add" : "replace";
}

/**
 * The selection while a marquee crosses rows `first` to `last` (indexes, both included; none if
 * `first > last`). `initial` is the selection when it started. The anchor and focus stay.
 */
export function marquee(
  initial: Selection,
  rows: Uint32Array,
  first: number,
  last: number,
  mode: MarqueeMode,
): Selection {
  const crossed = first <= last ? rows.subarray(first, last + 1) : new Uint32Array(0);
  const selected = mode === "replace" ? new Set<number>() : new Set(initial.selected);
  for (const id of crossed) {
    if (mode === "toggle" && initial.selected.has(id)) {
      selected.delete(id);
    } else {
      selected.add(id);
    }
  }
  return { ...initial, selected };
}

/**
 * The rows a marquee from `top` to `bottom` crosses (CSS px from the first row's top, in either
 * order), as `[first, last]` indexes; `first > last` when it crosses none. Rows are `rowHeight`
 * tall, one after another, so rows scrolled out of the DOM count too.
 */
export function rowsCrossed(
  top: number,
  bottom: number,
  rowHeight: number,
  count: number,
): [number, number] {
  const from = Math.min(top, bottom);
  const to = Math.max(top, bottom);
  const first = Math.max(0, Math.floor(from / rowHeight));
  const last = Math.min(count - 1, Math.ceil(to / rowHeight) - 1);
  return to <= 0 || from >= count * rowHeight ? [0, -1] : [first, last];
}
