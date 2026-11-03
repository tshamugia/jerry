import type { Channel } from "@tauri-apps/api/core";
import { mockIPC } from "@tauri-apps/api/mocks";
import { expect, test } from "vitest";
import { decodeListingMessage, type ListingMessage, openListing } from ".";
import { HEADER_BYTES } from "./channel";
import { batchMessage, doneMessage, errorMessage, type TestEntry } from "./listing.testing";

const file = (name: string, size = 0): TestEntry => ({ name, size });

test("a batch decodes into columns without copying", () => {
  const message = batchMessage(256, [
    { name: "ნახაზი 2.dwg", size: 2 ** 40 + 1, modified: 1_767_225_600_000, attributes: 0x20 },
    { name: "Folder", size: 0, modified: 0, attributes: 0x10 },
  ]);

  const decoded = decodeListingMessage(message);

  expect(decoded.kind).toBe("batch");
  if (decoded.kind !== "batch") return;
  expect(decoded.firstId).toBe(256);
  expect(decoded.names).toEqual(["ნახაზი 2.dwg", "Folder"]);
  expect([...decoded.sizes]).toEqual([2 ** 40 + 1, 0]);
  expect([...decoded.modified]).toEqual([1_767_225_600_000, 0]);
  expect([...decoded.attributes]).toEqual([0x20, 0x10]);
  expect(decoded.sizes.buffer).toBe(message);
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

  const result = await openListing(2, { kind: "path", path: String.raw`C:\Work` }, (message) =>
    seen.push(message.kind),
  );
  if (result.status !== "ok") throw new Error(result.error.code);
  channel?.onmessage(batchMessage(0, [file("a.txt")]));
  channel?.onmessage(doneMessage(1, 1, 0.5));
  await result.data.close();
  channel?.onmessage(batchMessage(1, [file("late.txt")]));

  expect(result.data).toMatchObject({ id: 7, path: String.raw`C:\Work` });
  expect(seen).toEqual(["batch", "done"]);
  expect(calls.map(([cmd]) => cmd)).toEqual(["listing_open", "listing_close"]);
  expect(calls[0]?.[1]).toMatchObject({
    tab: 2,
    target: { kind: "path", path: String.raw`C:\Work` },
  });
  expect(calls[1]?.[1]).toEqual({ id: 7 });
});

test("openListing passes a JerryError through", async () => {
  mockIPC(() => Promise.reject({ code: "InvalidName", path: "Work" }));

  const result = await openListing(1, { kind: "path", path: "Work" }, () => {});

  expect(result).toEqual({ status: "error", error: { code: "InvalidName", path: "Work" } });
});
