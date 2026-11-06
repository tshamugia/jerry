import { waitFor } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import type { ListingOpened, ListingTarget } from "../ipc";
import {
  batchMessage,
  doneMessage,
  type FakeListings,
  fakeListings,
  orderMessage,
  type TestEntry,
} from "../ipc/listing.testing";
import {
  activateTab,
  activeTab,
  addTab,
  closeOtherTabs,
  closeTab,
  closeTabsToRight,
  duplicateTab,
  MAX_HISTORY,
  moveTab,
  openInNewTab,
  pinTab,
  reopenClosedTab,
  resetTabs,
  showTabNumber,
  stepTab,
  type Tab,
  tabById,
  useTabs,
} from "./tabs";

afterEach(() => {
  resetTabs();
});

/** A drive's root. (`String.raw` can't end in a backslash.) */
const ROOT = "C:\\";

/** A small disk: each folder's token, path, name, parent token and entries. */
const FOLDERS: Record<number, { path: string; parent: number | null; entries: TestEntry[] }> = {
  1: { path: ROOT, parent: null, entries: [{ name: "Work", attributes: 0x10 }] },
  2: {
    path: String.raw`C:\Work`,
    parent: 1,
    entries: [{ name: "ნახაზები", attributes: 0x10 }, { name: "plan.dwg" }],
  },
  3: { path: String.raw`C:\Work\ნახაზები`, parent: 2, entries: [{ name: "a.dwg" }] },
  4: { path: String.raw`D:\Other`, parent: null, entries: [] },
};

/** The folder token a target names on the small disk; entries open ნახაზები. */
function tokenOf(target: ListingTarget): number {
  switch (target.kind) {
    case "folder":
      return target.folder;
    case "path":
      return Number(
        Object.entries(FOLDERS).find(([, folder]) => folder.path === target.path)?.[0] ?? 4,
      );
    case "entry":
      return 3;
    default:
      return 2;
  }
}

function disk(): FakeListings {
  return fakeListings({
    messages: (target) => {
      const { entries } = FOLDERS[tokenOf(target)] ?? { entries: [] };
      return [
        batchMessage(0, entries),
        orderMessage(entries.map((_, i) => i)),
        doneMessage(entries.length, 1, 1),
      ];
    },
    opened: (target): Partial<ListingOpened> => {
      const folder = tokenOf(target);
      const { path, parent } = FOLDERS[folder] ?? { path: "?", parent: null };
      // As the core names them: a root by itself, another folder by its last part.
      const name = path.endsWith("\\") ? path : (path.split("\\").at(-1) ?? path);
      return { path, name, folder, parent };
    },
  });
}

const work = { kind: "path", path: String.raw`C:\Work` } as const;

const ready = (tab: Tab) => waitFor(() => expect(tab.folder.store.getState().status).toBe("ready"));
const names = (tab: Tab) => tab.store.getState().history.map((entry) => entry.name);
const focusedName = (tab: Tab) => {
  const { focused, entries } = tab.folder.store.getState();
  return focused === undefined ? undefined : entries.names[focused];
};
const ids = (pane: "left" | "right") => useTabs.getState()[pane].tabs.map((tab) => tab.id);

test("opening folders makes a history; Back and Forward go along it", async () => {
  const core = disk();
  const tab = addTab("left");
  await tab.open(work);
  await ready(tab);
  // The entry focused when the tab leaves a folder gets the focus back on return.
  tab.folder.setSelection({ selected: new Set([0]), anchor: 0, focused: 0 });
  await tab.open({ kind: "entry", listing: 1, entry: 0 });
  await ready(tab);
  expect(names(tab)).toEqual(["Work", "ნახაზები"]);
  expect(tab.store.getState().index).toBe(1);

  tab.back();
  expect(tab.store.getState().index).toBe(0);
  await ready(tab);
  expect(focusedName(tab)).toBe("ნახაზები");
  expect(core.calls.filter((call) => call.cmd === "listing_open").at(-1)?.args.target).toEqual({
    kind: "folder",
    folder: 2,
  });

  tab.forward();
  expect(tab.current()?.name).toBe("ნახაზები");
  tab.forward();
  expect(tab.store.getState().index).toBe(1);
});

test("opening a folder from back in the history drops the steps after it", async () => {
  disk();
  const tab = addTab("left");
  await tab.open(work);
  await tab.open({ kind: "entry", listing: 1, entry: 0 });
  tab.back();
  await tab.open({ kind: "path", path: String.raw`D:\Other` });
  expect(names(tab)).toEqual(["Work", "Other"]);
  // The folder shown, opened again, isn't a new step.
  await tab.open({ kind: "path", path: String.raw`D:\Other` });
  expect(names(tab)).toEqual(["Work", "Other"]);
});

test("Back held down steps back once per repeat, before the folders answer", async () => {
  disk();
  const tab = addTab("left");
  await tab.open({ kind: "path", path: ROOT });
  await tab.open(work);
  await tab.open({ kind: "entry", listing: 2, entry: 0 });

  tab.back();
  tab.back();
  tab.back();
  expect(tab.store.getState().index).toBe(0);
  await ready(tab);
  expect(tab.current()?.path).toBe(ROOT);
});

test("Up opens the folder this one is in, focusing the folder it came from; a root has none", async () => {
  disk();
  const tab = addTab("left");
  await tab.open({ kind: "entry", listing: 1, entry: 0 });
  tab.up();
  await waitFor(() => expect(tab.current()?.name).toBe("Work"));
  await ready(tab);
  expect(focusedName(tab)).toBe("ნახაზები");
  expect(tab.folder.store.getState().selected.size).toBe(1);
  expect(names(tab)).toEqual(["ნახაზები", "Work"]);

  tab.up();
  await waitFor(() => expect(tab.current()?.parent).toBeNull());
  tab.up();
  expect(names(tab)).toEqual(["ნახაზები", "Work", ROOT]);
});

test("a tab's history keeps its latest folders", async () => {
  let next = 10;
  fakeListings({ opened: () => ({ folder: next++ }) });
  const tab = addTab("left");
  for (let i = 0; i < MAX_HISTORY + 5; i++) {
    await tab.open(work);
  }
  const { history, index } = tab.store.getState();
  expect(history).toHaveLength(MAX_HISTORY);
  expect(index).toBe(MAX_HISTORY - 1);
  expect(history[0]?.folder).toBe(15);
});

test("a new tab opens after the active one and shows; background ones keep their order", async () => {
  disk();
  const first = addTab("left");
  await first.open(work);
  const last = addTab("left");
  activateTab("left", first.id);

  const shown = openInNewTab("left", { kind: "folder", folder: 2 });
  expect(ids("left")).toEqual([first.id, shown.id, last.id]);
  expect(useTabs.getState().left.active).toBe(shown.id);

  activateTab("left", first.id);
  const a = openInNewTab("left", { kind: "entry", listing: 1, entry: 0 }, { activate: false });
  const b = openInNewTab("left", { kind: "entry", listing: 1, entry: 0 }, { activate: false });
  expect(ids("left")).toEqual([first.id, a.id, b.id, shown.id, last.id]);
  expect(useTabs.getState().left.active).toBe(first.id);
  // The new tab takes its opener's sort.
  first.folder.setSort({ column: "size", descending: true, foldersFirst: true });
  const c = openInNewTab("left", { kind: "folder", folder: 2 });
  expect(c.folder.store.getState().sort.column).toBe("size");
});

test("closing shows the right neighbour; pinned and last tabs stay; Ctrl+Shift+T brings it back", async () => {
  disk();
  const [a, b, c] = [addTab("right"), addTab("right"), addTab("right")];
  await b.open(work);
  await b.open({ kind: "entry", listing: 1, entry: 0 });
  await ready(b);
  b.folder.setSelection({ selected: new Set([0]), anchor: 0, focused: 0 });
  activateTab("right", b.id);

  closeTab("right", b.id);
  expect(ids("right")).toEqual([a.id, c.id]);
  expect(useTabs.getState().right.active).toBe(c.id);
  expect(tabById(b.id)).toBeUndefined();

  pinTab("right", a.id, true);
  closeTab("right", a.id);
  expect(ids("right")).toEqual([a.id, c.id]);

  const back = reopenClosedTab();
  expect(back).toBeDefined();
  expect(ids("right")).toEqual([a.id, back?.id, c.id]);
  expect(useTabs.getState().right.active).toBe(back?.id);
  expect(back && names(back)).toEqual(["Work", "ნახაზები"]);
  expect(back?.store.getState().index).toBe(1);
  // With the entry it had focused.
  if (back) await ready(back);
  expect(back && focusedName(back)).toBe("a.dwg");
  expect(reopenClosedTab()).toBeUndefined();

  const only = addTab("left");
  closeTab("left", only.id);
  expect(ids("left")).toEqual([only.id]);
});

test("pinning moves a tab to the pinned group; dragging keeps a tab in its group", () => {
  const [a, b, c, d] = [addTab("left"), addTab("left"), addTab("left"), addTab("left")];
  pinTab("left", c.id, true);
  expect(useTabs.getState().left.tabs).toEqual([
    { id: c.id, pinned: true },
    { id: a.id, pinned: false },
    { id: b.id, pinned: false },
    { id: d.id, pinned: false },
  ]);
  // An unpinned tab dragged to the front stops after the pinned ones.
  moveTab("left", d.id, 0);
  expect(ids("left")).toEqual([c.id, d.id, a.id, b.id]);
  moveTab("left", c.id, 3);
  expect(ids("left")).toEqual([c.id, d.id, a.id, b.id]);
  moveTab("left", d.id, 3);
  expect(ids("left")).toEqual([c.id, a.id, b.id, d.id]);
  pinTab("left", c.id, false);
  expect(useTabs.getState().left.tabs[0]).toEqual({ id: c.id, pinned: false });
});

test("Close others and Close to the right keep pinned tabs", () => {
  const [a, b, c, d] = [addTab("left"), addTab("left"), addTab("left"), addTab("left")];
  pinTab("left", a.id, true);
  activateTab("left", d.id);
  closeTabsToRight("left", b.id);
  expect(ids("left")).toEqual([a.id, b.id]);
  expect(useTabs.getState().left.active).toBe(b.id);

  const e = addTab("left");
  closeOtherTabs("left", e.id);
  expect(ids("left")).toEqual([a.id, e.id]);
  expect(useTabs.getState().left.active).toBe(e.id);
  expect(c.folder.store.getState().status).toBe("none");
});

test("Duplicate copies the history, the look and the filter, right after the tab", async () => {
  disk();
  const tab = addTab("left");
  addTab("left");
  await tab.open(work);
  await tab.open({ kind: "entry", listing: 1, entry: 0 });
  await ready(tab);
  tab.folder.setFilter("a");
  tab.folder.setSort({ column: "modified", descending: true, foldersFirst: false });

  const copy = duplicateTab("left", tab.id);
  expect(ids("left")[1]).toBe(copy?.id);
  expect(activeTab("left")).toBe(copy);
  expect(copy && names(copy)).toEqual(["Work", "ნახაზები"]);
  expect(copy?.folder.store.getState()).toMatchObject({
    filter: "a",
    sort: { column: "modified", descending: true, foldersFirst: false },
  });
});

test("Ctrl+Tab goes round the strip; Ctrl+1…8 go to that tab and Ctrl+9 to the last", () => {
  const [a, b, c] = [addTab("left"), addTab("left"), addTab("left")];
  stepTab("left", -1);
  expect(useTabs.getState().left.active).toBe(c.id);
  stepTab("left", 1);
  expect(useTabs.getState().left.active).toBe(a.id);
  showTabNumber("left", 2);
  expect(useTabs.getState().left.active).toBe(b.id);
  showTabNumber("left", 8);
  expect(useTabs.getState().left.active).toBe(b.id);
  showTabNumber("left", 9);
  expect(useTabs.getState().left.active).toBe(c.id);
});
