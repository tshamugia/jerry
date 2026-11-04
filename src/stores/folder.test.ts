import { waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
import type { Sort } from "../ipc";
import {
  batchMessage,
  diffMessage,
  doneMessage,
  errorMessage,
  fakeListings,
  orderMessage,
} from "../ipc/listing.testing";
import { Folder } from "./folder";

const BY_NAME: Sort = { column: "name", descending: false, foldersFirst: true };
const work = { kind: "path", path: String.raw`C:\Work` } as const;

const names = (folder: Folder) => {
  const { entries, order } = folder.store.getState();
  return [...(order ?? [])].map((id) => entries.names[id]);
};

test("a folder fills batch by batch, then takes the core's order and is ready", async () => {
  const core = fakeListings({
    messages: () => [
      batchMessage(0, [{ name: "file10.txt" }, { name: "file2.txt" }]),
      batchMessage(2, [{ name: "პროექტი", attributes: 0x10 }]),
      orderMessage([2, 1, 0]),
      doneMessage(3, 2, 1),
    ],
  });
  const folder = new Folder(7);

  await folder.open(work);
  expect(folder.store.getState()).toMatchObject({ listing: 1, path: String.raw`C:\Work` });
  expect(core.calls[0]?.args).toMatchObject({ tab: 7, target: work, sort: BY_NAME });

  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  expect(names(folder)).toEqual(["პროექტი", "file2.txt", "file10.txt"]);
});

test("a sort asked for before the core answered is passed on once it has", async () => {
  const core = fakeListings();
  const folder = new Folder(7);

  const opening = folder.open(work);
  folder.sortBy("size");
  await opening;

  const sorts = core.calls.filter((call) => call.cmd === "listing_sort");
  expect(sorts.map((call) => call.args)).toEqual([
    { id: 1, sort: { column: "size", descending: false, foldersFirst: true } },
  ]);
});

test("clicking the sorted column again flips it; an order for an earlier sort is dropped", async () => {
  const core = fakeListings({
    messages: () => [
      batchMessage(0, [
        { name: "b.txt", size: 1 },
        { name: "a.txt", size: 2 },
      ]),
      orderMessage([1, 0]),
      doneMessage(2, 1, 1),
    ],
  });
  const folder = new Folder(7);
  await folder.open(work);
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));

  folder.sortBy("size");
  folder.sortBy("size");
  expect(folder.store.getState().sort).toEqual({ ...BY_NAME, column: "size", descending: true });

  // The ascending order arrives late: it isn't shown.
  core.send(orderMessage([0, 1], { column: 3 }));
  expect(names(folder)).toEqual(["a.txt", "b.txt"]);
  core.send(orderMessage([1, 0], { column: 3, descending: true }));
  expect(names(folder)).toEqual(["a.txt", "b.txt"]);
  core.send(orderMessage([0, 1], { column: 3, descending: true }));
  expect(names(folder)).toEqual(["b.txt", "a.txt"]);
});

test("a diff updates entries; removed ones leave the order and the selection at once", async () => {
  const core = fakeListings({
    messages: () => [
      batchMessage(0, [{ name: "a.txt" }, { name: "b.txt", size: 1 }]),
      orderMessage([0, 1]),
      doneMessage(2, 1, 1),
    ],
  });
  const folder = new Folder(7);
  await folder.open(work);
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  folder.select(0);

  core.send(diffMessage([1, 2], [0], [{ name: "b.txt", size: 9 }, { name: "ნახაზი.dwg" }]));

  const state = folder.store.getState();
  expect(names(folder)).toEqual(["b.txt"]);
  expect(state.entries.sizes[1]).toBe(9);
  expect(state.selected.size).toBe(0);
  expect(state.focused).toBeUndefined();

  core.send(orderMessage([1, 2]));
  expect(names(folder)).toEqual(["b.txt", "ნახაზი.dwg"]);
});

test("an error ends the listing; Retry opens the same folder by the failed listing", async () => {
  const core = fakeListings({
    messages: () => [errorMessage(String.raw`{"code":"AccessDenied","path":"C:\\დახურული"}`)],
  });
  const folder = new Folder(7);
  await folder.open(work);
  await waitFor(() => expect(folder.store.getState().status).toBe("failed"));
  expect(folder.store.getState().error?.code).toBe("AccessDenied");

  await folder.retry();

  const opens = core.calls.filter((call) => call.cmd === "listing_open");
  expect(opens.map((call) => call.args.target)).toEqual([work, { kind: "listing", listing: 1 }]);
});

test("a subfolder opens by its entry id; files don't open here", async () => {
  const core = fakeListings({
    messages: () => [
      batchMessage(0, [{ name: "პროექტი", attributes: 0x10 }, { name: "plan.dwg" }]),
      doneMessage(2, 1, 1),
    ],
  });
  const folder = new Folder(7);
  await folder.open(work);
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));

  folder.openEntry(1);
  folder.openEntry(0);

  const opens = core.calls.filter((call) => call.cmd === "listing_open");
  expect(opens.map((call) => call.args.target)).toEqual([
    work,
    { kind: "entry", listing: 1, entry: 0 },
  ]);
});

test("messages for a folder that was left are dropped", async () => {
  const core = fakeListings();
  const folder = new Folder(7);
  await folder.open(work);
  await folder.open({ kind: "path", path: String.raw`D:\Other` });

  // Still on its way from the first folder.
  core.send(batchMessage(0, [{ name: "old.txt" }]), 1);
  core.send(batchMessage(0, [{ name: "new.txt" }]), 2);
  expect(folder.store.getState().entries.names).toEqual(["new.txt"]);

  folder.close();
  expect(folder.store.getState()).toMatchObject({ status: "none", listing: undefined });
  expect(core.calls.at(-1)).toMatchObject({ cmd: "listing_close", args: { id: 2 } });
});
