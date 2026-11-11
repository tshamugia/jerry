import { expect, test } from "vitest";
import { conflictThumbUrl, entryThumbUrl, thumbPixels } from ".";

test("thumbnail sizes are device pixels within what the core serves", () => {
  expect(thumbPixels(64, 1)).toBe(64);
  expect(thumbPixels(64, 1.5)).toBe(96);
  expect(thumbPixels(92, 2)).toBe(184);
  expect(thumbPixels(4, 1)).toBe(16);
  expect(thumbPixels(800, 2)).toBe(1024);
});

test("an entry's thumbnail changes with its modified time and attributes", () => {
  const photo = { listing: 3, id: 1200, modified: 1_767_225_600_000.5, attributes: 0x20 };
  const url = new URL(entryThumbUrl(photo, 64) ?? "");
  expect(url.host).toBe("thumb.localhost");
  expect(url.pathname).toBe(`/entry-3-1200-${thumbPixels(64)}`);
  expect(url.searchParams.get("v")).toMatch(/^[0-9a-f]{8}$/);
  expect(url.searchParams.get("m")).toBe("1767225600000");
  expect(url.searchParams.get("a")).toBe("32");
  expect(entryThumbUrl({ ...photo, modified: 1 }, 64)).not.toBe(url.href);
  // Made online-only: RECALL_ON_DATA_ACCESS.
  const cloud = entryThumbUrl({ ...photo, attributes: 0x40_0020 }, 64);
  expect(new URL(cloud ?? "").searchParams.get("a")).toBe(String(0x40_0020));
});

test("folders have no thumbnail", () => {
  expect(entryThumbUrl({ listing: 3, id: 1, modified: 0, attributes: 0x10 }, 64)).toBeUndefined();
});

test("a conflict's thumbnails are by job and side, and change with the question", () => {
  const url = new URL(conflictThumbUrl(7, "existing", "question 1", 64));
  expect(url.pathname).toBe(`/conflict-7-existing-${thumbPixels(64)}`);
  expect(url.searchParams.get("q")).toMatch(/^[0-9a-f]{8}$/);
  expect(conflictThumbUrl(7, "existing", "question 1", 64)).toBe(url.href);
  expect(conflictThumbUrl(7, "existing", "question 2", 64)).not.toBe(url.href);
  expect(conflictThumbUrl(7, "incoming", "question 1", 64)).toContain("/conflict-7-incoming-");
});
