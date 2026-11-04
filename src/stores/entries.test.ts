import { expect, test } from "vitest";
import { decodeListingMessage, type ListingBatch, type ListingDiff } from "../ipc";
import { batchMessage, diffMessage, type TestEntry } from "../ipc/listing.testing";
import { FolderEntries, visibleRows } from "./entries";

const batch = (firstId: number, entries: TestEntry[]) =>
  decodeListingMessage(batchMessage(firstId, entries)) as ListingBatch;
const diff = (ids: number[], removed: number[], entries: TestEntry[]) =>
  decodeListingMessage(diffMessage(ids, removed, entries)) as ListingDiff;

test("batches fill the columns by id and grow them past their capacity", () => {
  const entries = new FolderEntries();
  entries.appendBatch(batch(0, [{ name: "ნახაზი.dwg", size: 2 ** 40, created: 5 }]));
  const many = Array.from({ length: 2000 }, (_, i) => ({ name: `file${i}.txt`, size: i }));
  entries.appendBatch(batch(1, many));

  expect(entries.count).toBe(2001);
  expect(entries.names[0]).toBe("ნახაზი.dwg");
  expect(entries.sizes[0]).toBe(2 ** 40);
  expect(entries.created[0]).toBe(5);
  expect(entries.names[2000]).toBe("file1999.txt");
  expect(entries.sizes[2000]).toBe(1999);
  expect(entries.sizes.length).toBeGreaterThanOrEqual(2001);
});

test("a diff changes entries in place, adds new ids and marks removed ones", () => {
  const entries = new FolderEntries();
  entries.appendBatch(batch(0, [{ name: "a.txt" }, { name: "b.txt", size: 1 }]));

  entries.applyDiff(
    diff(
      [1, 2],
      [0],
      [
        { name: "b.txt", size: 4096 },
        { name: "პროექტი", attributes: 0x10 },
      ],
    ),
  );

  expect(entries.count).toBe(3);
  expect(entries.sizes[1]).toBe(4096);
  expect(entries.isFolder(2)).toBe(true);
  expect([...entries.removed]).toEqual([0]);
});

test("rows follow the core's order, or arrival order before it, without removed entries", () => {
  const entries = new FolderEntries();
  entries.appendBatch(batch(0, [{ name: "a" }, { name: "b" }, { name: "c" }]));
  const order = Uint32Array.from([2, 0, 1]);

  expect([...visibleRows(entries, undefined, true)]).toEqual([0, 1, 2]);
  expect(visibleRows(entries, order, true)).toBe(order);

  entries.applyDiff(diff([], [1], []));
  expect([...visibleRows(entries, undefined, true)]).toEqual([0, 2]);
});

test("hidden and system entries show only when asked for", () => {
  const entries = new FolderEntries();
  entries.appendBatch(
    batch(0, [
      { name: "desktop.ini", attributes: 0x2 | 0x4 },
      { name: "plan.dwg" },
      { name: "$Recycle.Bin", attributes: 0x10 | 0x2 },
      { name: "pagefile.sys", attributes: 0x4 },
    ]),
  );
  const order = Uint32Array.from([2, 0, 1, 3]);

  expect([...visibleRows(entries, order, false)]).toEqual([1]);
  expect(visibleRows(entries, order, true)).toBe(order);
  expect(entries.isHidden(0)).toBe(true);
  expect(entries.isHidden(1)).toBe(false);
});
