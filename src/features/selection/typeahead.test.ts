import { describe, expect, it } from "vitest";
import { findByPrefix, TYPE_RESET_MS, TypeAhead } from "./typeahead";

// Entry ids 0…6, shown in a different order.
const names = ["Plot", "plan_rev2.dwg", "Pictures", "გეგმა.ნახ", "გეგმა 2.ნახ", "ნახაზი.dwg", "a"];
const rows = Uint32Array.from([6, 0, 1, 2, 3, 4, 5]);

/** Types `text` one key at a time, `gap` ms apart, and follows each match as the list does. */
function typeAll(text: string, start: number | undefined, gap = 100) {
  const typeAhead = new TypeAhead();
  let focus = start;
  const found: (string | undefined)[] = [];
  [...text].forEach((char, i) => {
    const { prefix, after } = typeAhead.type(char, i * gap);
    const from = focus === undefined ? 0 : after ? focus + 1 : focus;
    const at = findByPrefix(rows, names, prefix, from % rows.length);
    if (at !== undefined) {
      focus = at;
    }
    found.push(at === undefined ? undefined : names[rows[at] ?? 0]);
  });
  return found;
}

describe("type-to-select", () => {
  it("goes to the first name with the typed letters, ignoring case", () => {
    expect(typeAll("pla", undefined)).toEqual(["Plot", "Plot", "plan_rev2.dwg"]);
    expect(typeAll("PIC", undefined)).toEqual(["Plot", "Pictures", "Pictures"]);
  });

  it("works with Georgian names", () => {
    expect(typeAll("გეგმა ", undefined).at(-1)).toBe("გეგმა 2.ნახ");
    expect(typeAll("ნ", undefined)).toEqual(["ნახაზი.dwg"]);
  });

  it("the same letter again goes through the names that start with it, round to the top", () => {
    expect(typeAll("pppp", undefined)).toEqual(["Plot", "plan_rev2.dwg", "Pictures", "Plot"]);
  });

  it("a first letter starts after the focused row, so it moves on", () => {
    // Focus on Plot (row 1): "p" goes to the next name with p.
    expect(typeAll("p", 1)).toEqual(["plan_rev2.dwg"]);
  });

  it("a longer prefix keeps the focused row if it still matches", () => {
    expect(typeAll("pl", 1)).toEqual(["plan_rev2.dwg", "plan_rev2.dwg"]);
  });

  it("starts over after a pause", () => {
    const typeAhead = new TypeAhead();
    expect(typeAhead.type("p", 0)).toEqual({ prefix: "p", after: true });
    expect(typeAhead.type("l", 200)).toEqual({ prefix: "pl", after: false });
    expect(typeAhead.active(200 + TYPE_RESET_MS - 1)).toBe(true);
    expect(typeAhead.active(200 + TYPE_RESET_MS)).toBe(false);
    expect(typeAhead.type("a", 200 + TYPE_RESET_MS)).toEqual({ prefix: "a", after: true });
  });

  it("finds nothing when no name matches", () => {
    expect(typeAll("xyz", undefined)).toEqual([undefined, undefined, undefined]);
  });
});
