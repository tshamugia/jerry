import type { Channel } from "@tauri-apps/api/core";
import { mockIPC } from "@tauri-apps/api/mocks";
import { expect, test } from "vitest";
import {
  decodeListingMessage,
  IconKey,
  type ListingMessage,
  type ListingOrder,
  openListing,
  type Sort,
} from ".";
import { HEADER_BYTES } from "./channel";
import {
  batchMessage,
  diffMessage,
  doneMessage,
  errorMessage,
  orderMessage,
  type TestEntry,
} from "./listing.testing";

const file = (name: string, size = 0): TestEntry => ({ name, size });
const byName: Sort = { column: "name", descending: false, foldersFirst: true };

test("a batch decodes into columns without copying", () => {
  const message = batchMessage(256, [
    {
      name: "ნახაზი 2.dwg",
      size: 2 ** 40 + 1,
      modified: 1_767_225_600_000,
      created: 1_700_000_000_000,
      attributes: 0x20,
      iconKey: 7,
    },
    { name: "Folder", size: 0, modified: 0, attributes: 0x10, iconKey: 1 },
    { name: "setup.exe", size: 1, modified: 0, attributes: 0x20, iconKey: 0 },
  ]);

  const decoded = decodeListingMessage(message);

  expect(decoded.kind).toBe("batch");
  if (decoded.kind !== "batch") return;
  expect(decoded.firstId).toBe(256);
  expect(decoded.names).toEqual(["ნახაზი 2.dwg", "Folder", "setup.exe"]);
  expect([...decoded.sizes]).toEqual([2 ** 40 + 1, 0, 1]);
  expect([...decoded.modified]).toEqual([1_767_225_600_000, 0, 0]);
  expect([...decoded.created]).toEqual([1_700_000_000_000, 0, 0]);
  expect([...decoded.attributes]).toEqual([0x20, 0x10, 0x20]);
  expect([...decoded.iconKeys]).toEqual([7, IconKey.folder, IconKey.own]);
  expect(decoded.sizes.buffer).toBe(message);
  expect(decoded.iconKeys.buffer).toBe(message);
});

test("odd names keep every later name in place", () => {
  const names = [
    "\uFEFFstarts with a BOM.txt",
    "lone \uD800 surrogate.txt",
    "emoji 📐.dwg",
    "after.txt",
  ];

  const decoded = decodeListingMessage(
    batchMessage(
      0,
      names.map((name) => file(name)),
    ),
  );

  if (decoded.kind !== "batch") throw new Error(decoded.kind);
  expect(decoded.names).toEqual([
    "\uFEFFstarts with a BOM.txt",
    "lone \uFFFD surrogate.txt",
    "emoji 📐.dwg",
    "after.txt",
  ]);
});

test("a diff decodes its ids, removals and entries like a batch's", () => {
  for (const removed of [[0], [0, 4]]) {
    const message = diffMessage([1, 5], removed, [
      { name: "b.txt", size: 2 ** 40 + 1, modified: 1_767_225_600_000 },
      { name: "გეგმა 📐.pdf", size: 3, attributes: 0x10 },
    ]);

    const decoded = decodeListingMessage(message);

    if (decoded.kind !== "diff") throw new Error(decoded.kind);
    expect([...decoded.ids]).toEqual([1, 5]);
    expect([...decoded.removed]).toEqual(removed);
    expect(decoded.names).toEqual(["b.txt", "გეგმა 📐.pdf"]);
    expect([...decoded.sizes]).toEqual([2 ** 40 + 1, 3]);
    expect([...decoded.modified]).toEqual([1_767_225_600_000, 0]);
    expect([...decoded.attributes]).toEqual([0x20, 0x10]);
    expect([...decoded.iconKeys]).toEqual([IconKey.firstType, IconKey.folder]);
    expect(decoded.sizes.buffer).toBe(message);
  }
  const removalOnly = decodeListingMessage(diffMessage([], [3], []));
  if (removalOnly.kind !== "diff") throw new Error(removalOnly.kind);
  expect(removalOnly.names).toEqual([]);
  expect([...removalOnly.removed]).toEqual([3]);
});

test("done and error messages decode", () => {
  expect(decodeListingMessage(doneMessage(100_000, 4, 31.5))).toEqual({
    kind: "done",
    count: 100_000,
    batches: 4,
    rustMs: 31.5,
  });
  expect(
    decodeListingMessage(errorMessage(String.raw`{"code":"AccessDenied","path":"C:\\დახურული"}`)),
  ).toEqual({
    kind: "error",
    error: { code: "AccessDenied", path: String.raw`C:\დახურული` },
  });
});

test("an order decodes into ids and the sort it follows", () => {
  const message = orderMessage([2, 0, 70_000], { column: 4, descending: true });

  const decoded = decodeListingMessage(message);

  if (decoded.kind !== "order") throw new Error(decoded.kind);
  expect(decoded.sort).toEqual({ column: "created", descending: true, foldersFirst: true });
  expect([...decoded.ids]).toEqual([2, 0, 70_000]);
  expect(decoded.ids.buffer).toBe(message);
  const columns = [0, 1, 2, 3, 4, 5].map((column) => {
    const order = decodeListingMessage(orderMessage([], { column, foldersFirst: false }));
    return order.kind === "order" ? [order.sort.column, order.sort.foldersFirst] : [];
  });
  expect(columns).toEqual([
    ["name", false],
    ["modified", false],
    ["type", false],
    ["size", false],
    ["created", false],
    ["extension", false],
  ]);
  expect(() => decodeListingMessage(orderMessage([], { column: 6 }))).toThrow(RangeError);
});

test("an unknown message kind is a bug", () => {
  const buffer = new ArrayBuffer(HEADER_BYTES);
  new Uint32Array(buffer, 0, 1).set([9]);
  expect(() => decodeListingMessage(buffer)).toThrow(RangeError);
});

test("openListing streams decoded messages until it is closed", async () => {
  const calls: [string, unknown][] = [];
  let channel: Channel<ArrayBuffer> | undefined;
  mockIPC((cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === "listing_open") {
      channel = (args as { onMessage: Channel<ArrayBuffer> }).onMessage;
      return { id: 7, path: String.raw`C:\Work` };
    }
    return null;
  });
  const seen: ListingMessage["kind"][] = [];

  const result = await openListing(
    2,
    { kind: "path", path: String.raw`C:\Work` },
    byName,
    (message) => seen.push(message.kind),
  );
  if (result.status !== "ok") throw new Error(result.error.code);
  channel?.onmessage(batchMessage(0, [file("a.txt")]));
  channel?.onmessage(orderMessage([0]));
  channel?.onmessage(doneMessage(1, 1, 0.5));
  await result.data.close();
  channel?.onmessage(batchMessage(1, [file("late.txt")]));

  expect(result.data).toMatchObject({ id: 7, path: String.raw`C:\Work` });
  expect(seen).toEqual(["batch", "order", "done"]);
  expect(calls.map(([cmd]) => cmd)).toEqual(["listing_open", "listing_close"]);
  expect(calls[0]?.[1]).toMatchObject({
    tab: 2,
    target: { kind: "path", path: String.raw`C:\Work` },
    sort: byName,
  });
  expect(calls[1]?.[1]).toEqual({ id: 7 });
});

test("an open listing re-sorts through listing_sort and gets the order on its stream", async () => {
  const calls: [string, unknown][] = [];
  let channel: Channel<ArrayBuffer> | undefined;
  mockIPC((cmd, args) => {
    calls.push([cmd, args]);
    switch (cmd) {
      case "listing_open":
        channel = (args as { onMessage: Channel<ArrayBuffer> }).onMessage;
        return { id: 7, path: String.raw`C:\Work` };
      case "listing_sort":
        channel?.onmessage(orderMessage([1, 0], { column: 3, descending: true }));
        return null;
      default:
        return null;
    }
  });
  const bySize: Sort = { column: "size", descending: true, foldersFirst: true };
  const orders: ListingOrder[] = [];

  const opened = await openListing(
    2,
    { kind: "path", path: String.raw`C:\Work` },
    byName,
    (message) => {
      if (message.kind === "order") orders.push(message);
    },
  );
  if (opened.status !== "ok") throw new Error(opened.error.code);
  const sorted = await opened.data.sort(bySize);

  expect(sorted).toEqual({ status: "ok", data: null });
  expect(orders.map((order) => order.sort)).toEqual([bySize]);
  expect([...(orders[0]?.ids ?? [])]).toEqual([1, 0]);
  expect(calls[1]).toEqual(["listing_sort", { id: 7, sort: bySize }]);
});

test("sorting a closed listing fails with NotFound", async () => {
  mockIPC((cmd) =>
    cmd === "listing_sort"
      ? Promise.reject({ code: "NotFound", path: null })
      : { id: 7, path: String.raw`C:\Work` },
  );

  const opened = await openListing(
    2,
    { kind: "path", path: String.raw`C:\Work` },
    byName,
    () => {},
  );
  if (opened.status !== "ok") throw new Error(opened.error.code);

  expect(await opened.data.sort(byName)).toEqual({
    status: "error",
    error: { code: "NotFound", path: null },
  });
});

test("openListing passes a JerryError through", async () => {
  mockIPC(() => Promise.reject({ code: "InvalidName", path: "Work" }));

  const result = await openListing(1, { kind: "path", path: "Work" }, byName, () => {});

  expect(result).toEqual({ status: "error", error: { code: "InvalidName", path: "Work" } });
});
