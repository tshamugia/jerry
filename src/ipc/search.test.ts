import type { Channel } from "@tauri-apps/api/core";
import { mockIPC } from "@tauri-apps/api/mocks";
import { expect, test } from "vitest";
import { decodeSearchMessage, type SearchMessage, type SearchQuery, startSearch } from ".";
import { HEADER_BYTES } from "./channel";
import { entriesBytes, type TestEntry, unitCount, writeEntries } from "./listing.testing";

/** A batch message laid out as `src-tauri/src/search.rs` writes it. */
function searchBatch(
  firstId: number,
  entries: TestEntry[],
  folderIds: number[],
  firstFolder: number,
  folders: string[],
): ArrayBuffer {
  const units = unitCount(entries);
  const folderUnits = folders.reduce((count, folder) => count + folder.length, 0);
  const words = entries.length + folders.length;
  const entriesAt = HEADER_BYTES * 2 + (words + (words % 2)) * 4;
  const buffer = new ArrayBuffer(entriesAt + entriesBytes(entries.length, units) + folderUnits * 2);
  new Uint32Array(buffer, 0, 8).set([
    1,
    entries.length,
    firstId,
    units,
    folders.length,
    firstFolder,
    folderUnits,
    0,
  ]);
  const ends: number[] = [];
  for (const folder of folders) ends.push((ends.at(-1) ?? 0) + folder.length);
  new Uint32Array(buffer, HEADER_BYTES * 2, words).set([...folderIds, ...ends]);
  writeEntries(buffer, entriesAt, entries);
  const text = folders.join("");
  const at = buffer.byteLength - folderUnits * 2;
  new Uint16Array(buffer, at, folderUnits).set(stringUnits(text));
  return buffer;
}

/** UTF-16 code units, as Rust writes them. */
function stringUnits(text: string): number[] {
  return Array.from({ length: text.length }, (_, i) => text.charCodeAt(i));
}

function counts(kind: number, found: number, checked: number, skipped: number, ms?: number) {
  const buffer = new ArrayBuffer(HEADER_BYTES + (ms === undefined ? 0 : 8));
  new Uint32Array(buffer, 0, 4).set([kind, found, checked, skipped]);
  if (ms !== undefined) new Float64Array(buffer, HEADER_BYTES, 1).set([ms]);
  return buffer;
}

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

test("progress, done and error decode", () => {
  expect(decodeSearchMessage(counts(2, 48, 5000, 1))).toEqual({
    kind: "progress",
    found: 48,
    checked: 5000,
    skipped: 1,
  });
  expect(decodeSearchMessage(counts(3, 50, 6000, 1, 12.5))).toEqual({
    kind: "done",
    found: 50,
    checked: 6000,
    skipped: 1,
    rustMs: 12.5,
  });
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
  expect(() => decodeSearchMessage(counts(9, 0, 0, 0))).toThrow(RangeError);
});

test("startSearch streams decoded messages until it is cancelled", async () => {
  const calls: [string, unknown][] = [];
  let channel: Channel<ArrayBuffer> | undefined;
  mockIPC((cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === "search_start") {
      channel = (args as { onMessage: Channel<ArrayBuffer> }).onMessage;
      return 5;
    }
    return null;
  });
  const seen: SearchMessage["kind"][] = [];

  const result = await startSearch({ kind: "folder", folder: 12 }, query, (message) =>
    seen.push(message.kind),
  );
  if (result.status !== "ok") throw new Error(result.error.code);
  channel?.onmessage(searchBatch(0, [{ name: "ნახაზი.dwg" }], [0], 0, [DRIVE]));
  channel?.onmessage(counts(2, 1, 10, 0));
  await result.data.cancel();
  channel?.onmessage(counts(3, 1, 20, 0, 1));

  expect(result.data.id).toBe(5);
  expect(seen).toEqual(["batch", "progress"]);
  expect(calls.map(([cmd]) => cmd)).toEqual(["search_start", "search_cancel"]);
  expect(calls[0]?.[1]).toMatchObject({ within: { kind: "folder", folder: 12 }, query });
  expect(calls[1]?.[1]).toEqual({ id: 5 });
});

test("startSearch passes a JerryError through", async () => {
  mockIPC(() => Promise.reject({ code: "NotFound", path: null }));

  const result = await startSearch({ kind: "drive", folder: 99 }, query, () => {});

  expect(result).toEqual({ status: "error", error: { code: "NotFound", path: null } });
});
