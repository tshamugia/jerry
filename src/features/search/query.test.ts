import { describe, expect, it } from "vitest";
import type { SavedSearch } from "../../ipc";
import {
  dateField,
  dateLabel,
  dateRange,
  EMPTY_SEARCH,
  filtersText,
  parseDate,
  parseSize,
  queryOf,
  rangeOf,
  SIZE_PRESETS,
  searchKey,
  searchTitle,
  sizeLabel,
  sizeRangeOf,
  typeLabel,
} from "./query";

const MB = 1024 * 1024;
const local = (year: number, month: number, date: number) =>
  new Date(year, month - 1, date).getTime();

describe("relative dates", () => {
  // Wednesday 11.11.2026, 15:30 local time.
  const now = new Date(2026, 10, 11, 15, 30);

  it("today runs from midnight to the next", () => {
    expect(dateRange({ kind: "today" }, now)).toEqual({
      from: local(2026, 11, 11),
      to: local(2026, 11, 12),
    });
  });

  it("this week runs from Monday", () => {
    expect(dateRange({ kind: "thisWeek" }, now)).toEqual({
      from: local(2026, 11, 9),
      to: local(2026, 11, 16),
    });
    // On a Sunday, the week began six days before.
    expect(dateRange({ kind: "thisWeek" }, new Date(2026, 10, 15, 9))?.from).toBe(
      local(2026, 11, 9),
    );
    // A week across a month's end.
    expect(dateRange({ kind: "thisWeek" }, new Date(2026, 11, 2))).toEqual({
      from: local(2026, 11, 30),
      to: local(2026, 12, 7),
    });
  });

  it("this month runs from the 1st", () => {
    expect(dateRange({ kind: "thisMonth" }, new Date(2026, 11, 31))).toEqual({
      from: local(2026, 12, 1),
      to: local(2027, 1, 1),
    });
  });

  it("a range is as it was saved, and none is none", () => {
    expect(dateRange({ kind: "range", from: 5, to: null }, now)).toEqual({ from: 5, to: null });
    expect(dateRange(null, now)).toBeNull();
  });
});

describe("typed dates", () => {
  it("read as DD.MM.YYYY at local midnight", () => {
    expect(parseDate("09.11.2026")).toBe(local(2026, 11, 9));
    expect(parseDate(" 9/11/26 ")).toBe(local(2026, 11, 9));
    expect(parseDate("31.02.2026")).toBeUndefined();
    expect(parseDate("2026-11-09")).toBeUndefined();
    expect(dateField(local(2026, 1, 5))).toBe("05.01.2026");
  });

  it("make a range whose last day is included", () => {
    expect(rangeOf("01.11.2026", "09.11.2026")).toEqual({
      kind: "range",
      from: local(2026, 11, 1),
      to: local(2026, 11, 10),
    });
    expect(rangeOf("", "09.11.2026")).toEqual({
      kind: "range",
      from: null,
      to: local(2026, 11, 10),
    });
    expect(rangeOf("bad", "")).toBeUndefined();
  });
});

describe("sizes", () => {
  it("read with a unit, MB without one", () => {
    expect(parseSize("500 KB")).toBe(500 * 1024);
    expect(parseSize("1,5 gb")).toBe(1.5 * 1024 * MB);
    expect(parseSize("12")).toBe(12 * MB);
    expect(parseSize("12 parsecs")).toBeUndefined();
    expect(sizeRangeOf("1 MB", "")).toEqual({ min: MB, max: null });
    expect(sizeRangeOf("", "")).toEqual({ min: null, max: null });
    expect(sizeRangeOf("x", "")).toBeUndefined();
  });

  it("show a preset by its name, else the range", () => {
    expect(sizeLabel(SIZE_PRESETS[1]?.size ?? null)).toBe("Under 1 MB");
    expect(sizeLabel({ min: 10 * MB, max: null })).toBe("At least 10 MB");
    expect(sizeLabel({ min: null, max: 512 * 1024 })).toBe("At most 512 KB");
    expect(sizeLabel(null)).toBe("Any size");
  });
});

describe("a search", () => {
  const dwg: SavedSearch = {
    ...EMPTY_SEARCH,
    name: "  *.dwg ",
    modified: { kind: "thisWeek" },
    items: "files",
  };

  it("becomes the engine's query: dates worked out, hidden files with Ctrl+H", () => {
    const now = new Date(2026, 10, 11, 15, 30);
    expect(queryOf(dwg, true, now)).toEqual({
      name: "*.dwg",
      fileType: null,
      size: null,
      modified: { from: local(2026, 11, 9), to: local(2026, 11, 16) },
      items: "files",
      hidden: true,
      followLinks: false,
      systemFolders: false,
    });
  });

  it("says its filters in a few words", () => {
    expect(filtersText(dwg, dateField)).toBe("This week · Files only");
    expect(
      filtersText(
        {
          ...dwg,
          fileType: { kind: "drawings" },
          modified: { kind: "range", from: local(2026, 11, 1), to: local(2026, 11, 10) },
          wholeDrive: true,
        },
        dateField,
      ),
    ).toBe("Drawings (DWG, DXF) · 01.11.2026 – 09.11.2026 · Files only · Whole drive");
    expect(filtersText(EMPTY_SEARCH, dateField)).toBe("");
    expect(typeLabel({ kind: "extension", extension: "*.PDF" })).toBe(".PDF");
    expect(typeLabel({ kind: "extension", extension: "" })).toBe("No extension");
    expect(dateLabel({ kind: "range", from: null, to: local(2026, 11, 10) }, dateField)).toBe(
      "Until 09.11.2026",
    );
  });

  it("is named by its name, or as search results without one", () => {
    expect(searchTitle(dwg)).toBe("*.dwg");
    expect(searchTitle(EMPTY_SEARCH)).toBe("Search results");
  });

  it("has a key that tells it from another", () => {
    const reordered = JSON.parse(
      '{"wholeDrive":false,"items":"files","modified":{"kind":"thisWeek"},"size":null,"fileType":null,"name":"  *.dwg "}',
    );
    expect(searchKey(reordered)).toBe(searchKey(dwg));
    expect(searchKey({ ...dwg, wholeDrive: true })).not.toBe(searchKey(dwg));
  });
});
