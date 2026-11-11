import { waitFor } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { EMPTY_SEARCH } from "../features/search/query";
import type { SavedSearch } from "../ipc";
import { batchMessage, doneMessage, fakeListings, orderMessage } from "../ipc/listing.testing";
import {
  searchBatch,
  searchDiff,
  searchDone,
  searchOrder,
  searchProgress,
} from "../ipc/search.testing";
import { DEFAULT_COLUMNS, withLocation } from "./columns";
import { Folder, type SearchSpec } from "./folder";
import { resetTabs, Tab } from "./tabs";

/**
 * A results tab (#66): a folder view that shows a search's results, from many folders, with
 * their Location, as they are found.
 */

const ROOT = String.raw`D:\Projects`;
const dwg: SavedSearch = { ...EMPTY_SEARCH, name: "*.dwg", modified: { kind: "thisWeek" } };
const spec = (search = dwg): SearchSpec => ({ search, folder: 5, name: "Projects", drive: "D:" });

const shown = (folder: Folder) => {
  const { entries } = folder.store.getState();
  return [...folder.shown(true).rows].map((id) => `${entries.names[id]} @ ${entries.location(id)}`);
};

afterEach(() => resetTabs());

test("results show as they are found, with their folders, then sorted once the search is done", async () => {
  const core = fakeListings({
    path: ROOT,
    searches: () => [
      searchBatch(0, [{ name: "plan_rev2.dwg" }, { name: "ნახაზი 1.dwg" }], [0, 1], 0, [
        String.raw`D:\Projects\BMW`,
        String.raw`D:\Projects\ბათუმი`,
      ]),
      searchProgress(2, 40),
    ],
  });
  const folder = new Folder(3);

  const opened = await folder.search(spec());

  expect(opened).toMatchObject({ id: 1, path: ROOT, folder: 5 });
  const start = core.calls.find((call) => call.cmd === "search_start")?.args;
  expect(start).toMatchObject({
    tab: 3,
    within: { kind: "folder", folder: 5 },
    query: { name: "*.dwg", items: "all", hidden: false, systemFolders: false },
  });
  // The tab's own columns, with Location after Name.
  expect(folder.store.getState().columns).toEqual(withLocation(DEFAULT_COLUMNS));
  await waitFor(() => expect(folder.store.getState().search?.found).toBe(2));
  expect(folder.store.getState().search).toMatchObject({ phase: "searching", checked: 40 });
  expect(shown(folder)).toEqual([
    String.raw`plan_rev2.dwg @ D:\Projects\BMW`,
    String.raw`ნახაზი 1.dwg @ D:\Projects\ბათუმი`,
  ]);

  // More come; the order covers what was found so far, the newcomers follow it.
  core.send(searchOrder([1, 0]));
  core.send(searchBatch(2, [{ name: "a.dwg" }], [0], 2, []));
  expect(shown(folder)).toEqual([
    String.raw`ნახაზი 1.dwg @ D:\Projects\ბათუმი`,
    String.raw`plan_rev2.dwg @ D:\Projects\BMW`,
    String.raw`a.dwg @ D:\Projects\BMW`,
  ]);

  core.send(searchOrder([2, 1, 0]));
  core.send(searchDone(3, 12.5));
  expect(folder.store.getState()).toMatchObject({
    status: "ready",
    search: { phase: "done", found: 3, ms: 12.5 },
  });
  expect(shown(folder)[0]).toBe(String.raw`a.dwg @ D:\Projects\BMW`);
});

test("Stop ends the search with what it found", async () => {
  const core = fakeListings({ path: ROOT });
  const folder = new Folder(3);
  await folder.search(spec());

  folder.stopSearch();
  core.send(searchDone(0, 4, true));

  expect(core.calls.map((call) => call.cmd)).toContain("search_cancel");
  expect(folder.store.getState().search?.phase).toBe("stopped");
  // Once it has ended, Stop has nothing to stop.
  folder.stopSearch();
  expect(core.calls.filter((call) => call.cmd === "search_cancel")).toHaveLength(1);
});

test("Jerry's renames and deletes reach the results", async () => {
  const core = fakeListings({
    path: ROOT,
    searches: () => [
      searchBatch(0, [{ name: "a.dwg" }, { name: "b.dwg" }], [0, 0], 0, [
        String.raw`D:\Projects\B`,
      ]),
      searchOrder([0, 1]),
      searchDone(2, 1),
    ],
  });
  const folder = new Folder(3);
  await folder.search(spec());
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  folder.setSelection({ selected: new Set([1]), anchor: 1, focused: 1 });

  // The folder B was renamed to ბ: a.dwg is in it, under a new folder id; b.dwg was deleted.
  core.send(searchDiff([0], [1], [{ name: "a.dwg" }], [1], 1, [String.raw`D:\Projects\ბ`]));
  core.send(searchOrder([0]));

  expect(shown(folder)).toEqual([String.raw`a.dwg @ D:\Projects\ბ`]);
  expect(folder.store.getState().selected.size).toBe(0);
});

test("a folder opened in a results tab has no Location column, nor a sort by it", async () => {
  fakeListings({
    path: ROOT,
    messages: () => [batchMessage(0, [{ name: "x" }]), orderMessage([0]), doneMessage(1, 1, 1)],
  });
  const folder = new Folder(3);
  await folder.search(spec());
  folder.sortBy("location");
  expect(folder.store.getState().sort.column).toBe("location");

  await folder.open({ kind: "folder", folder: 5 });

  expect(folder.store.getState()).toMatchObject({
    search: undefined,
    columns: DEFAULT_COLUMNS,
    sort: { column: "name" },
  });
});

test("a refresh runs the search again, keeping the quick filter", async () => {
  const core = fakeListings({ path: ROOT, searches: () => [searchDone(0, 1)] });
  const folder = new Folder(3);
  await folder.search(spec());
  folder.setFilter("rev");

  await folder.refresh();

  expect(core.calls.filter((call) => call.cmd === "search_start")).toHaveLength(2);
  expect(folder.store.getState()).toMatchObject({ filter: "rev", search: { phase: "searching" } });
});

test("a tab's search is a step in its history: Back searches again, Up opens the folder", async () => {
  const core = fakeListings({ path: ROOT, searches: () => [searchDone(0, 1)] });
  const tab = new Tab(9);
  await tab.open({ kind: "path", path: ROOT });

  await tab.search(spec());

  const { history, index } = tab.store.getState();
  expect(index).toBe(1);
  expect(history[1]).toMatchObject({ name: "*.dwg", path: ROOT, search: spec() });
  expect(history[1]?.parent).toBe(history[1]?.folder);

  // The same search again replaces the step; another one is a new step.
  await tab.search(spec());
  expect(tab.store.getState().history).toHaveLength(2);
  await tab.search(spec({ ...dwg, name: "rev" }));
  expect(tab.store.getState().history.map((entry) => entry.name)).toEqual([
    "Projects",
    "*.dwg",
    "rev",
  ]);

  tab.back();
  await waitFor(() =>
    expect(core.calls.filter((call) => call.cmd === "search_start")).toHaveLength(4),
  );
  expect(tab.folder.store.getState().search?.spec.search.name).toBe("*.dwg");

  tab.up();
  await waitFor(() => expect(tab.folder.store.getState().search).toBeUndefined());
  const opens = core.calls.filter((call) => call.cmd === "listing_open");
  expect(opens.at(-1)?.args.target).toEqual({ kind: "folder", folder: history[1]?.folder });
});
