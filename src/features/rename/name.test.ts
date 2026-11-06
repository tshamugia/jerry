import { expect, test } from "vitest";
import table from "../../../crates/jerry-core/tests/name_cases.json";
import { checkName, foldName } from "./name";

interface Case {
  name: string;
  repeat?: number;
  problem: string | null;
}

// The same table the core's rules are tested with (`crates/jerry-core/tests/name.rs`).
const cases: Case[] = table.cases;

test("the table has its cases", () => {
  expect(cases.length).toBeGreaterThan(40);
});

test.each(cases.map((c) => [JSON.stringify(c.name), c.repeat ?? 1, c] as const))(
  "%s × %i breaks the rule the core says",
  (_, repeat, { name, problem }) => {
    expect(checkName(name.repeat(repeat))?.kind ?? null).toBe(problem);
  },
);

test("a character problem names the character, a reserved one the device", () => {
  expect(checkName("a:b")).toEqual({ kind: "character", character: ":" });
  expect(checkName("nul.txt")).toEqual({ kind: "reserved", device: "NUL" });
  expect(checkName("lpt³.dwg")).toEqual({ kind: "reserved", device: "LPT³" });
});

test("names fold without regard to case, but Georgian letters stay as they are", () => {
  expect(foldName("Plan.DWG")).toBe(foldName("plan.dwg"));
  expect(foldName("Ünïcode")).toBe(foldName("üNÏCODE"));
  expect(foldName("ნახაზი Plan")).toBe("ნახაზი PLAN");
  expect(foldName("ნახაზი")).not.toBe(foldName("ᲜᲐᲮᲐᲖᲘ"));
  expect(foldName("Straße")).not.toBe(foldName("STRASSE"));
});
