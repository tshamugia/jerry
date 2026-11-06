import type { Channel } from "@tauri-apps/api/core";
import { mockIPC } from "@tauri-apps/api/mocks";
import { waitFor } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { ListingOpened, Look, Sort } from "../ipc";
import {
  batchMessage,
  diffMessage,
  doneMessage,
  errorMessage,
  fakeListings,
  orderMessage,
} from "../ipc/listing.testing";
import { DEFAULT_COLUMNS } from "./columns";
import { Folder, REMEMBER_VIEW_MS } from "./folder";

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

  // The folder remembers the user's sort once the clicks settle, or as the tab leaves it.
  expect(core.calls.some((call) => call.cmd === "folder_view_save")).toBe(false);
  folder.close();
  expect(
    core.calls.filter((call) => call.cmd === "folder_view_save").map((call) => call.args),
  ).toEqual([
    {
      folder: 101,
      view: {
        view: "details",
        sort: { ...BY_NAME, column: "size", descending: true },
        columns: {
          order: [...DEFAULT_COLUMNS.order],
          hidden: [...DEFAULT_COLUMNS.hidden],
          widths: {},
        },
      },
    },
  ]);
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
  folder.setSelection({ selected: new Set([0]), anchor: 0, focused: 0 });

  core.send(diffMessage([1, 2], [0], [{ name: "b.txt", size: 9 }, { name: "ნახაზი.dwg" }]));

  const state = folder.store.getState();
  expect(names(folder)).toEqual(["b.txt"]);
  expect(state.entries.sizes[1]).toBe(9);
  expect(state.selected.size).toBe(0);
  expect(state.focused).toBeUndefined();
  expect(state.anchor).toBeUndefined();

  core.send(orderMessage([1, 2]));
  expect(names(folder)).toEqual(["b.txt", "ნახაზი.dwg"]);
});

test("the selection survives changes in the folder; a removed focus passes to the next row", async () => {
  const core = fakeListings({
    messages: () => [
      batchMessage(0, [{ name: "a.txt" }, { name: "b.txt" }, { name: "c.txt" }, { name: "d.txt" }]),
      orderMessage([0, 1, 2, 3]),
      doneMessage(4, 1, 1),
    ],
  });
  const folder = new Folder(7);
  await folder.open(work);
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  expect([...folder.shown(false).rows]).toEqual([0, 1, 2, 3]);
  folder.setSelection({ selected: new Set([0, 1, 3]), anchor: 0, focused: 1 });
  const before = folder.store.getState().selected;

  // A change and an addition keep the very same selection: no row repaints for it.
  core.send(diffMessage([2, 4], [], [{ name: "c.txt", size: 5 }, { name: "e.txt" }]));
  core.send(orderMessage([0, 1, 2, 3, 4]));
  expect(folder.store.getState().selected).toBe(before);

  // The focused row goes: b.txt's place is c.txt's now.
  core.send(diffMessage([], [1], []));
  expect(folder.store.getState()).toMatchObject({
    selected: new Set([0, 3]),
    anchor: 0,
    focused: 2,
  });

  // The last row goes while focused: the focus stays on the last row.
  folder.setSelection({ selected: new Set([4]), anchor: 4, focused: 4 });
  core.send(diffMessage([], [4], []));
  expect(folder.store.getState()).toMatchObject({ selected: new Set(), anchor: 3, focused: 3 });
});

test("the shown rows leave out hidden entries and are the same array until something changes", async () => {
  fakeListings({
    messages: () => [
      batchMessage(0, [{ name: "a.txt" }, { name: "desktop.ini", attributes: 0x2 | 0x4 }]),
      orderMessage([0, 1]),
      doneMessage(2, 1, 1),
    ],
  });
  const folder = new Folder(7);
  await folder.open(work);
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));

  const rows = folder.shown(false).rows;
  expect([...rows]).toEqual([0]);
  expect(folder.shown(false).rows).toBe(rows);
  expect([...folder.shown(true).rows]).toEqual([0, 1]);

  // Hiding hidden files again drops them from the selection.
  folder.setSelection({ selected: new Set([0, 1]), anchor: 1, focused: 1 });
  folder.keepShown(folder.shown(false).rows);
  expect(folder.store.getState()).toMatchObject({
    selected: new Set([0]),
    anchor: undefined,
    focused: undefined,
  });
});

test("an error ends the listing; Retry opens the same folder by the failed listing", async () => {
  const core = fakeListings({
    messages: () => [errorMessage(String.raw`{"code":"AccessDenied","path":"C:\\დახურული"}`)],
  });
  const folder = new Folder(7);
  await folder.open(work);
  await waitFor(() => expect(folder.store.getState().status).toBe("failed"));
  expect(folder.store.getState().error?.code).toBe("AccessDenied");

  await folder.refresh();

  const opens = core.calls.filter((call) => call.cmd === "listing_open");
  expect(opens.map((call) => call.args.target)).toEqual([work, { kind: "listing", listing: 1 }]);
});

test("a folder opened again focuses and selects an entry by name once all of it is in", async () => {
  const core = fakeListings();
  const folder = new Folder(7);
  await folder.open(work, { focus: "ნახაზი.dwg" });
  core.send(batchMessage(0, [{ name: "a.txt" }, { name: "ნახაზი.dwg" }, { name: "c.txt" }]));
  core.send(orderMessage([0, 1, 2]));
  // Only once the whole folder is in: the name may come in a later batch.
  expect(folder.store.getState().focused).toBeUndefined();

  core.send(doneMessage(3, 1, 1));
  expect(folder.store.getState()).toMatchObject({
    focused: 1,
    anchor: 1,
    selected: new Set([1]),
    scrollTo: { kind: "reveal", id: 1 },
  });
  folder.scrolled();
  expect(folder.store.getState().scrollTo).toBeUndefined();
});

test("a selection made while the folder opens wins over the one it would get back", async () => {
  const core = fakeListings();
  const folder = new Folder(7);
  await folder.open(work, { focus: "c.txt" });
  core.send(batchMessage(0, [{ name: "a.txt" }, { name: "c.txt" }]));
  folder.setSelection({ selected: new Set([0]), anchor: 0, focused: 0 });
  core.send(doneMessage(2, 1, 1));

  expect(folder.store.getState()).toMatchObject({ focused: 0, selected: new Set([0]) });
  expect(folder.store.getState().scrollTo).toBeUndefined();
});

test("Refresh reads the folder again, keeping the selection, focus, filter and scroll by name", async () => {
  const entries = [{ name: "a.txt" }, { name: "ნახაზი.dwg" }, { name: "c.txt" }];
  const core = fakeListings({
    messages: () => [batchMessage(0, entries), orderMessage([0, 1, 2]), doneMessage(3, 1, 1)],
  });
  const folder = new Folder(7);
  await folder.open(work);
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  folder.setFilter(".");
  folder.setSelection({ selected: new Set([0, 2]), anchor: 0, focused: 1 });
  folder.scrollTop = 120;

  await folder.refresh();
  expect(core.calls.filter((call) => call.cmd === "listing_open").at(-1)?.args.target).toEqual({
    kind: "listing",
    listing: 1,
  });
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  expect(folder.store.getState()).toMatchObject({
    selected: new Set([0, 2]),
    focused: 1,
    filter: ".",
    scrollTo: { kind: "top", top: 120 },
  });
});

test("opening another folder starts it at the top, unfiltered", async () => {
  fakeListings();
  const folder = new Folder(7);
  await folder.open(work);
  folder.setFilter("plan");
  folder.scrollTop = 300;

  const opened = await folder.open({ kind: "path", path: String.raw`D:\Other` });
  expect(opened).toMatchObject({ id: 2, path: String.raw`C:\Work`, folder: 102 });
  expect(folder.store.getState().filter).toBe("");
  expect(folder.scrollTop).toBe(0);
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

const BY_SIZE: Sort = { column: "size", descending: true, foldersFirst: true };
const NARROW = { order: [...DEFAULT_COLUMNS.order], hidden: [], widths: { size: 90 } };

test("a folder opens with the look the core gives it: its own view, or the defaults", async () => {
  let look: Look = { kind: "saved", view: { view: "details", sort: BY_SIZE, columns: NARROW } };
  const core = fakeListings({ opened: () => ({ look }) });
  const folder = new Folder(7);
  folder.applyLook({ sort: BY_NAME, columns: DEFAULT_COLUMNS });

  await folder.open(work);
  expect(folder.store.getState()).toMatchObject({ sort: BY_SIZE, columns: NARROW });
  // The core started the listing with the folder's sort: nothing to sort again.
  expect(core.calls.some((call) => call.cmd === "listing_sort")).toBe(false);

  look = { kind: "default", view: "details", sort: BY_NAME };
  await folder.open({ kind: "path", path: String.raw`D:\Other` });
  expect(folder.store.getState()).toMatchObject({ sort: BY_NAME, columns: DEFAULT_COLUMNS });

  // A tab that keeps its own look (views not remembered per folder, or Refresh).
  folder.applyLook({ sort: BY_SIZE, columns: NARROW });
  look = { kind: "tab" };
  await folder.open(work);
  expect(folder.store.getState()).toMatchObject({ sort: BY_SIZE, columns: NARROW });
  folder.close();
});

test("an order for the folder's own sort that comes before the core's answer is shown", async () => {
  const calls: string[] = [];
  mockIPC((cmd, args) => {
    calls.push(cmd);
    if (cmd !== "listing_open") return null;
    const stream = (args as { onMessage: Channel<ArrayBuffer> }).onMessage;
    stream.onmessage(
      batchMessage(0, [
        { name: "small.txt", size: 1 },
        { name: "big.txt", size: 9 },
      ]),
    );
    stream.onmessage(orderMessage([1, 0], { column: 3, descending: true }));
    stream.onmessage(doneMessage(2, 1, 1));
    const answer: ListingOpened = {
      id: 1,
      path: String.raw`C:\Work`,
      name: "Work",
      folder: 5,
      parent: null,
      crumbs: [],
      look: { kind: "saved", view: { view: "details", sort: BY_SIZE, columns: NARROW } },
    };
    return answer;
  });
  const folder = new Folder(7);
  await folder.open(work);
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  expect(folder.store.getState().sort).toEqual(BY_SIZE);
  expect(names(folder)).toEqual(["big.txt", "small.txt"]);
  expect(calls).not.toContain("listing_sort");
});

test("the user's sort and columns are remembered for the folder once the changes settle", async () => {
  vi.useFakeTimers();
  try {
    const core = fakeListings();
    const folder = new Folder(7);
    await folder.open(work);
    const saves = () => core.calls.filter((call) => call.cmd === "folder_view_save");

    // A column's edge dragged: many changes, one save.
    for (const px of [100, 120, 140]) {
      folder.setColumns({ ...DEFAULT_COLUMNS, widths: { modified: px } });
    }
    vi.advanceTimersByTime(REMEMBER_VIEW_MS - 1);
    expect(saves()).toHaveLength(0);
    vi.advanceTimersByTime(1);
    expect(saves().map((call) => call.args)).toEqual([
      {
        folder: 101,
        view: {
          view: "details",
          sort: BY_NAME,
          columns: { ...DEFAULT_COLUMNS, widths: { modified: 140 } },
        },
      },
    ]);

    // Leaving the folder saves at once, for the folder left.
    folder.sortBy("type");
    await folder.open({ kind: "path", path: String.raw`D:\Other` });
    expect(saves().at(-1)?.args).toMatchObject({ folder: 101, view: { sort: { column: "type" } } });

    // A look the tab takes isn't the user's: nothing is saved.
    folder.applyLook({ sort: BY_SIZE, columns: NARROW });
    vi.advanceTimersByTime(REMEMBER_VIEW_MS);
    expect(saves()).toHaveLength(2);
    folder.close();
  } finally {
    vi.useRealTimers();
  }
});
