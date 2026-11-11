import type { Channel } from "@tauri-apps/api/core";
import { mockIPC } from "@tauri-apps/api/mocks";
import { expect, test } from "vitest";
import {
  decodeSearchMessage,
  type SearchMessage,
  type SearchOpened,
  type SearchQuery,
  type Sort,
  startSearch,
} from ".";
import { HEADER_BYTES } from "./channel";
import { searchBatch, searchDiff, searchDone, searchOrder, searchProgress } from "./search.testing";

/** A drive's root as display text. */
const DRIVE = "C:\\";

const query: SearchQuery = {
  name: "ნახაზი *.dwg",
  fileType: null,
  size: null,
  modified: null,
  items: "all",
  hidden: false,
  followLinks: false,
  systemFolders: false,
};

const sort: Sort = { column: "name", descending: false, foldersFirst: true };

const opened: SearchOpened = {
  id: 5,
  path: String.raw`C:\Work`,
  name: "Work",
  folder: 12,
  crumbs: [],
};

test("a batch carries results, their folder ids and the new folders", () => {
  const message = searchBatch(
    10,
    [
      { name: "ნახაზი 1.dwg", size: 2 ** 40, modified: 1_767_225_600_000 },
      { name: "📐 ნახაზი.dwg", size: 5 },
      { name: "Folder", attributes: 0x10 },
    ],
    [3, 4, 4],
    4,
    [String.raw`C:\Work\ახალი 📁`],
  );

  const decoded = decodeSearchMessage(message);

  expect(decoded.kind).toBe("batch");
  if (decoded.kind !== "batch") return;
  expect(decoded.firstId).toBe(10);
  expect(decoded.names).toEqual(["ნახაზი 1.dwg", "📐 ნახაზი.dwg", "Folder"]);
  expect([...decoded.sizes]).toEqual([2 ** 40, 5, 0]);
  expect([...decoded.modified]).toEqual([1_767_225_600_000, 0, 0]);
  expect([...decoded.attributes]).toEqual([0x20, 0x20, 0x10]);
  expect([...decoded.folderIds]).toEqual([3, 4, 4]);
  expect(decoded.firstFolder).toBe(4);
  expect(decoded.folders).toEqual([String.raw`C:\Work\ახალი 📁`]);
  expect(decoded.sizes.buffer).toBe(message);
});

test("an odd number of words is padded and several folders split by their ends", () => {
  const message = searchBatch(
    0,
    [
      { name: "a.txt", size: 1 },
      { name: "b.txt", size: 2 },
    ],
    [0, 1],
    0,
    [DRIVE, String.raw`\\server\share\ნახაზები`],
  );
  // Two results and two folders: even, no padding. One folder less is odd.
  const odd = searchBatch(0, [{ name: "a.txt", size: 7 }, { name: "b.txt" }], [0, 0], 0, [
    String.raw`D:\x`,
  ]);

  const decoded = decodeSearchMessage(message);
  const decodedOdd = decodeSearchMessage(odd);

  if (decoded.kind !== "batch" || decodedOdd.kind !== "batch") throw new Error("not batches");
  expect(decoded.folders).toEqual([DRIVE, String.raw`\\server\share\ნახაზები`]);
  expect([...decoded.sizes]).toEqual([1, 2]);
  expect(decodedOdd.folders).toEqual([String.raw`D:\x`]);
  expect([...decodedOdd.sizes]).toEqual([7, 0]);
  expect(decodedOdd.names).toEqual(["a.txt", "b.txt"]);
});

test("a diff carries renamed results with their folders, and removed ones", () => {
  const message = searchDiff(
    [1, 4],
    [2],
    [{ name: "ნახაზი v2.dwg" }, { name: "b.pdf" }],
    [0, 7],
    7,
    [String.raw`D:\Work\ბ`],
  );

  const decoded = decodeSearchMessage(message);

  if (decoded.kind !== "diff") throw new Error("not a diff");
  expect([...decoded.ids]).toEqual([1, 4]);
  expect([...decoded.removed]).toEqual([2]);
  expect(decoded.names).toEqual(["ნახაზი v2.dwg", "b.pdf"]);
  expect([...decoded.folderIds]).toEqual([0, 7]);
  expect(decoded.firstFolder).toBe(7);
  expect(decoded.folders).toEqual([String.raw`D:\Work\ბ`]);

  const removedOnly = decodeSearchMessage(searchDiff([], [3, 5], [], []));
  if (removedOnly.kind !== "diff") throw new Error("not a diff");
  expect([...removedOnly.removed]).toEqual([3, 5]);
  expect(removedOnly.names).toEqual([]);
});

test("progress, order, done and error decode", () => {
  expect(decodeSearchMessage(searchProgress(48, 5000, 1))).toEqual({
    kind: "progress",
    found: 48,
    checked: 5000,
    skipped: 1,
  });
  expect(decodeSearchMessage(searchDone(50, 12.5))).toEqual({
    kind: "done",
    found: 50,
    checked: 500,
    skipped: 0,
    rustMs: 12.5,
    stopped: false,
  });
  expect(decodeSearchMessage(searchDone(3, 1, true))).toMatchObject({ stopped: true });
  const order = decodeSearchMessage(searchOrder([2, 0, 1], { column: 6, descending: true }));
  if (order.kind !== "order") throw new Error("not an order");
  expect(order.sort).toEqual({ column: "location", descending: true, foldersFirst: true });
  expect([...order.ids]).toEqual([2, 0, 1]);

  const json = new TextEncoder().encode(
    JSON.stringify({ code: "AccessDenied", path: String.raw`C:\ნახაზები` }),
  );
  const error = new ArrayBuffer(HEADER_BYTES + json.length);
  new Uint32Array(error, 0, 2).set([4, json.length]);
  new Uint8Array(error, HEADER_BYTES).set(json);
  expect(decodeSearchMessage(error)).toEqual({
    kind: "error",
    error: { code: "AccessDenied", path: String.raw`C:\ნახაზები` },
  });
  const unknown = new ArrayBuffer(HEADER_BYTES);
  new Uint32Array(unknown, 0, 1).set([9]);
  expect(() => decodeSearchMessage(unknown)).toThrow(RangeError);
});

test("startSearch streams decoded messages, stops, sorts and closes by the results' id", async () => {
  const calls: [string, unknown][] = [];
  let channel: Channel<ArrayBuffer> | undefined;
  mockIPC((cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === "search_start") {
      channel = (args as { onMessage: Channel<ArrayBuffer> }).onMessage;
      return opened;
    }
    return null;
  });
  const seen: SearchMessage["kind"][] = [];

  const result = await startSearch(7, { kind: "folder", folder: 12 }, query, sort, (message) =>
    seen.push(message.kind),
  );
  if (result.status !== "ok") throw new Error(result.error.code);
  channel?.onmessage(searchBatch(0, [{ name: "ნახაზი.dwg" }], [0], 0, [DRIVE]));
  channel?.onmessage(searchProgress(1, 10));
  await result.data.stop();
  channel?.onmessage(searchOrder([0]));
  channel?.onmessage(searchDone(1, 2, true));
  await result.data.sort({ ...sort, column: "location" });
  await result.data.close();
  channel?.onmessage(searchOrder([0]));

  expect(result.data).toMatchObject(opened);
  expect(seen).toEqual(["batch", "progress", "order", "done"]);
  expect(calls.map(([cmd]) => cmd)).toEqual([
    "search_start",
    "search_cancel",
    "listing_sort",
    "listing_close",
  ]);
  expect(calls[0]?.[1]).toMatchObject({ tab: 7, within: { kind: "folder", folder: 12 }, query });
  expect(calls[1]?.[1]).toEqual({ id: 5 });
  expect(calls[3]?.[1]).toEqual({ id: 5 });
});

test("startSearch passes a JerryError through", async () => {
  mockIPC(() => Promise.reject({ code: "NotFound", path: null }));

  const result = await startSearch(1, { kind: "drive", folder: 99 }, query, sort, () => {});

  expect(result).toEqual({ status: "error", error: { code: "NotFound", path: null } });
});
