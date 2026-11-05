import { describe, expect, test } from "vitest";
import { wildcardMatcher } from "./wildcard";

const NAMES = [
  "ELV-001 LAN Layout GF.dwg",
  "plan_rev2.dwg",
  "Rev3 notes.txt",
  "report.PDF",
  "report.pdf.bak",
  "ნახაზი 12.dwg",
  "ᲜᲐᲮᲐᲖᲘ.dwg",
  "სართული 3 გეგმა.pdf",
  "a.b",
  "ab",
  "[draft] (v1) $100 + tax^2.txt",
  "📁 photos.zip",
];

const matching = (pattern: string) => NAMES.filter(wildcardMatcher(pattern));

describe("without wildcards, the pattern is a part of the name", () => {
  test("anywhere in it, in any case", () => {
    expect(matching("rev")).toEqual(["plan_rev2.dwg", "Rev3 notes.txt"]);
    expect(matching("LAYOUT")).toEqual(["ELV-001 LAN Layout GF.dwg"]);
    expect(matching(".pdf")).toEqual(["report.PDF", "report.pdf.bak", "სართული 3 გეგმა.pdf"]);
  });

  test("Georgian: Mtavruli and Mkhedruli are one case pair (spike #18)", () => {
    expect(matching("ნახაზ")).toEqual(["ნახაზი 12.dwg", "ᲜᲐᲮᲐᲖᲘ.dwg"]);
    expect(matching("ᲜᲐᲮᲐᲖᲘ")).toEqual(["ნახაზი 12.dwg", "ᲜᲐᲮᲐᲖᲘ.dwg"]);
    expect(matching("გეგმა")).toEqual(["სართული 3 გეგმა.pdf"]);
  });

  test("regular expression syntax is plain text", () => {
    for (const part of ["[draft]", "(v1)", "$100", "+ tax^2", "."]) {
      expect(matching(part)).toContain("[draft] (v1) $100 + tax^2.txt");
    }
    expect(matching("a.b")).toEqual(["a.b"]);
    expect(matching("\\")).toEqual([]);
  });

  test("the empty pattern matches every name", () => {
    expect(matching("")).toEqual(NAMES);
  });
});

describe("with wildcards, the pattern is the whole name", () => {
  test("`*.pdf` ends with .pdf, in any case", () => {
    expect(matching("*.pdf")).toEqual(["report.PDF", "სართული 3 გეგმა.pdf"]);
  });

  test("`rev*` starts with rev", () => {
    expect(matching("rev*")).toEqual(["Rev3 notes.txt"]);
    expect(matching("ნახ*")).toEqual(["ნახაზი 12.dwg", "ᲜᲐᲮᲐᲖᲘ.dwg"]);
  });

  test("`*_rev*` has it anywhere; stars in the middle span any run", () => {
    expect(matching("*_rev*")).toEqual(["plan_rev2.dwg"]);
    expect(matching("elv*gf.dwg")).toEqual(["ELV-001 LAN Layout GF.dwg"]);
    expect(matching("r*t*.pdf")).toEqual(["report.PDF"]);
    expect(matching("a**b")).toEqual(["a.b", "ab"]);
  });

  test("`?` is exactly one character, an emoji too", () => {
    expect(matching("a?b")).toEqual(["a.b"]);
    expect(matching("??")).toEqual(["ab"]);
    expect(matching("? photos.zip")).toEqual(["📁 photos.zip"]);
    expect(matching("ᲜᲐᲮᲐᲖ?.dwg")).toEqual(["ᲜᲐᲮᲐᲖᲘ.dwg"]);
  });

  test("`*` alone matches every name", () => {
    expect(matching("*")).toEqual(NAMES);
    expect(matching("**")).toEqual(NAMES);
  });

  test("regular expression syntax around wildcards is plain text", () => {
    expect(matching("[draft]*")).toEqual(["[draft] (v1) $100 + tax^2.txt"]);
    expect(matching("*$100 + tax^?.txt")).toEqual(["[draft] (v1) $100 + tax^2.txt"]);
  });
});

test("a matcher keeps no state between names", () => {
  const matches = wildcardMatcher("*.dwg");
  expect([matches("a.dwg"), matches("a.dwg"), matches("b.txt"), matches("c.DWG")]).toEqual([
    true,
    true,
    false,
    true,
  ]);
});
