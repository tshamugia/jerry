import { describe, expect, it } from "vitest";
import type { Selection } from "../../stores/folder";
import {
  clear,
  clickRow,
  invert,
  marquee,
  marqueeMode,
  moveTo,
  NO_MODIFIERS,
  rowForKey,
  rowsCrossed,
  selectAll,
  selectWhere,
  toggleFocused,
} from "./model";

/** Rows as the list shows them: entry ids in display order, not in id order. */
const rows = Uint32Array.from([40, 10, 30, 20, 50, 60]);

const none: Selection = { selected: new Set(), anchor: undefined, focused: undefined };
const sel = (ids: number[], anchor?: number, focused?: number): Selection => ({
  selected: new Set(ids),
  anchor,
  focused,
});
const ids = (selection: Selection) => [...selection.selected].sort((a, b) => a - b);

const CTRL = { ctrl: true, shift: false };
const SHIFT = { ctrl: false, shift: true };
const CTRL_SHIFT = { ctrl: true, shift: true };

describe("clicks", () => {
  it("a click selects only that row and moves the anchor and focus to it", () => {
    expect(clickRow(sel([40, 10], 40, 10), rows, 2, NO_MODIFIERS)).toEqual(sel([30], 30, 30));
  });

  it("Ctrl+click toggles a row and keeps the others", () => {
    const one = clickRow(sel([40], 40, 40), rows, 2, CTRL);
    expect(one).toEqual(sel([40, 30], 30, 30));
    expect(clickRow(one, rows, 0, CTRL)).toEqual(sel([30], 40, 40));
  });

  it("Shift+click selects the range from the anchor in display order; the anchor stays", () => {
    const start = clickRow(none, rows, 1, NO_MODIFIERS);
    const down = clickRow(start, rows, 4, SHIFT);
    expect(down).toEqual(sel([10, 30, 20, 50], 10, 50));
    // Shift again, upwards: the range replaces the previous one, from the same anchor.
    expect(clickRow(down, rows, 0, SHIFT)).toEqual(sel([40, 10], 10, 40));
  });

  it("Ctrl+Shift+click adds the range to the selection", () => {
    const picked = clickRow(clickRow(none, rows, 0, NO_MODIFIERS), rows, 3, CTRL);
    expect(ids(clickRow(picked, rows, 5, CTRL_SHIFT))).toEqual([20, 40, 50, 60]);
  });

  it("Shift without an anchor starts at the focus, or else at the row itself", () => {
    expect(ids(clickRow(sel([], undefined, 30), rows, 4, SHIFT))).toEqual([20, 30, 50]);
    expect(clickRow(none, rows, 3, SHIFT)).toEqual(sel([20], 20, 20));
  });

  it("an anchor the list no longer shows counts as none", () => {
    expect(ids(clickRow(sel([99], 99, 10), rows, 2, SHIFT))).toEqual([10, 30]);
  });
});

describe("keys", () => {
  const page = { first: 1, last: 3 };

  it("arrows, Home and End move one row, to the ends, and stop at them", () => {
    expect(rowForKey("ArrowDown", 2, 6, page)).toBe(3);
    expect(rowForKey("ArrowUp", 2, 6, page)).toBe(1);
    expect(rowForKey("ArrowUp", 0, 6, page)).toBe(0);
    expect(rowForKey("ArrowDown", 5, 6, page)).toBe(5);
    expect(rowForKey("Home", 4, 6, page)).toBe(0);
    expect(rowForKey("End", 1, 6, page)).toBe(5);
  });

  it("with no focus, every key but End goes to the first row; an empty list has none", () => {
    expect(rowForKey("ArrowDown", undefined, 6, page)).toBe(0);
    expect(rowForKey("PageDown", undefined, 6, page)).toBe(0);
    expect(rowForKey("End", undefined, 6, page)).toBe(5);
    expect(rowForKey("ArrowDown", undefined, 0, page)).toBeUndefined();
  });

  it("PageDown goes to the last row in view, then a page further; PageUp the same up", () => {
    const view = { first: 10, last: 19 };
    expect(rowForKey("PageDown", 12, 100, view)).toBe(19);
    expect(rowForKey("PageDown", 19, 100, view)).toBe(28);
    expect(rowForKey("PageDown", 95, 100, view)).toBe(99);
    expect(rowForKey("PageUp", 15, 100, view)).toBe(10);
    expect(rowForKey("PageUp", 10, 100, view)).toBe(1);
    expect(rowForKey("PageUp", 3, 100, { first: 3, last: 12 })).toBe(0);
  });

  it("moving selects the row; Shift selects the range from the anchor; Ctrl moves only the focus", () => {
    const one = moveTo(none, rows, 1, NO_MODIFIERS);
    expect(one).toEqual(sel([10], 10, 10));
    const range = moveTo(moveTo(one, rows, 2, SHIFT), rows, 3, SHIFT);
    expect(range).toEqual(sel([10, 30, 20], 10, 20));
    const focusOnly = moveTo(range, rows, 5, CTRL);
    expect(focusOnly).toEqual(sel([10, 30, 20], 10, 60));
    // Ctrl+Space then picks the focused row too, and the anchor moves there.
    const picked = toggleFocused(focusOnly, rows);
    expect(picked).toEqual(sel([10, 30, 20, 60], 60, 60));
    expect(toggleFocused(picked, rows)).toEqual(sel([10, 30, 20], 60, 60));
  });

  it("Ctrl+Space without a focus does nothing", () => {
    expect(toggleFocused(none, rows)).toBe(none);
  });
});

describe("select all, invert, clear, pattern", () => {
  it("Ctrl+A selects every shown row and keeps the focus; again, it changes nothing", () => {
    const all = selectAll(sel([10], 10, 10), rows);
    expect(ids(all)).toEqual([10, 20, 30, 40, 50, 60]);
    expect(all.focused).toBe(10);
    expect(selectAll(all, rows)).toBe(all);
  });

  it("Ctrl+A on a filtered list selects only what it shows", () => {
    const shown = Uint32Array.from([30, 50]);
    expect(ids(selectAll(sel([10]), shown))).toEqual([30, 50]);
  });

  it("invert selects the shown rows that weren't, and only those", () => {
    expect(ids(invert(sel([40, 20, 99], 40, 40), rows))).toEqual([10, 30, 50, 60]);
    expect(ids(invert(invert(sel([40, 20]), rows), rows))).toEqual([20, 40]);
    expect(invert(none, new Uint32Array(0)).selected.size).toBe(0);
  });

  it("Esc clears the selection and keeps the focus and anchor", () => {
    expect(clear(sel([10, 30], 10, 30))).toEqual(sel([], 10, 30));
    expect(clear(none)).toBe(none);
  });

  it("select by pattern selects exactly the matching rows", () => {
    const names: Record<number, string> = {
      40: "ELV-001.pdf",
      10: "plan_rev2.dwg",
      30: "შენიშვნები.PDF",
      20: "notes.txt",
      50: "plan_rev3.pdf",
      60: "readme",
    };
    const endsPdf = (id: number) => names[id]?.toLowerCase().endsWith(".pdf") ?? false;
    expect(ids(selectWhere(sel([20], 20, 20), rows, endsPdf))).toEqual([30, 40, 50]);
  });
});

describe("marquee", () => {
  it("crosses the rows between its top and bottom, in either direction", () => {
    expect(rowsCrossed(35, 95, 30, 6)).toEqual([1, 3]);
    expect(rowsCrossed(95, 35, 30, 6)).toEqual([1, 3]);
    expect(rowsCrossed(40, 40, 30, 6)).toEqual([1, 1]);
    // From below the last row, upwards; from above the first, downwards.
    expect(rowsCrossed(400, 130, 30, 6)).toEqual([4, 5]);
    expect(rowsCrossed(-20, 10, 30, 6)).toEqual([0, 0]);
  });

  it("crosses none in the empty space below the rows or above them", () => {
    const [first, last] = rowsCrossed(200, 260, 30, 6);
    expect(first > last).toBe(true);
    const [above, under] = rowsCrossed(-40, -5, 30, 6);
    expect(above > under).toBe(true);
  });

  it("replaces the selection alone, toggles with Ctrl, adds with Shift", () => {
    const initial = sel([40, 10], 40, 40);
    expect(marqueeMode(NO_MODIFIERS)).toBe("replace");
    expect(ids(marquee(initial, rows, 1, 3, "replace"))).toEqual([10, 20, 30]);
    expect(marqueeMode(CTRL)).toBe("toggle");
    expect(ids(marquee(initial, rows, 1, 3, "toggle"))).toEqual([20, 30, 40]);
    expect(marqueeMode(SHIFT)).toBe("add");
    expect(ids(marquee(initial, rows, 1, 3, "add"))).toEqual([10, 20, 30, 40]);
    // Back to crossing nothing: the selection it started from, or none.
    expect(ids(marquee(initial, rows, 0, -1, "toggle"))).toEqual([10, 40]);
    expect(marquee(initial, rows, 0, -1, "replace").focused).toBe(40);
  });
});

describe("100 000 rows", () => {
  const many = Uint32Array.from({ length: 100_000 }, (_, i) => 99_999 - i);

  it("selects all, inverts and makes ranges well within a frame", () => {
    const started = performance.now();
    const all = selectAll(none, many);
    const inverted = invert(clickRow(all, many, 50_000, CTRL), many);
    const range = clickRow(clickRow(none, many, 10, NO_MODIFIERS), many, 90_000, SHIFT);
    const elapsed = performance.now() - started;
    expect(all.selected.size).toBe(100_000);
    expect(ids(inverted)).toEqual([49_999]);
    expect(range.selected.size).toBe(89_991);
    // Generous for a loaded CI machine; the real number is measured in the app.
    expect(elapsed).toBeLessThan(200);
  });
});
