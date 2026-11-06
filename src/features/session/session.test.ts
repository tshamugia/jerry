import { waitFor } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import type {
  HistoryFolder,
  ListingTarget,
  SessionRestore,
  SessionSave,
  TabRestore,
} from "../../ipc";
import {
  batchMessage,
  doneMessage,
  errorMessage,
  fakeListings,
  orderMessage,
} from "../../ipc/listing.testing";
import { DEFAULT_COLUMNS } from "../../stores/columns";
import { activateTab, addTab, resetTabs, tabById, useTabs } from "../../stores/tabs";
import { useToasts } from "../../stores/toasts";
import { useWindow } from "../../stores/window";
import { restoreSession, restoreTabs } from "./restore";
import { saveSession, snapshot } from "./save";

afterEach(() => {
  resetTabs();
  useToasts.setState({ toasts: [] });
  useWindow.setState({ activePane: "left" });
});

const BY_DATE = { column: "modified", descending: true, foldersFirst: true } as const;

/** A folder as the core restores it: token `folder`, under `C:\Work`. */
function folder(token: number, name: string, focus: string | null = null): HistoryFolder {
  return {
    path: `C:\\Work\\${name}`,
    name,
    folder: token,
    parent: 1,
    crumbs: [{ name, folder: token }],
    focus,
  };
}

function tab(id: number, history: HistoryFolder[], extra: Partial<TabRestore> = {}): TabRestore {
  return { id, pinned: false, history, index: history.length - 1, look: { kind: "tab" }, ...extra };
}

const session: SessionRestore = {
  activePane: "right",
  left: {
    tabs: [
      tab(1, [folder(10, "ნახაზები"), folder(11, "04_Issued", "plan.dwg")], {
        pinned: true,
        index: 1,
        look: {
          kind: "saved",
          view: {
            view: "details",
            sort: BY_DATE,
            columns: { ...DEFAULT_COLUMNS, order: [...DEFAULT_COLUMNS.order], hidden: [] },
          },
        },
      }),
      tab(2, [folder(12, "Other")]),
    ],
    active: 0,
  },
  right: { tabs: [tab(3, [folder(13, "Right")])], active: 0 },
};

const opens = (core: ReturnType<typeof fakeListings>) =>
  core.calls.filter((call) => call.cmd === "listing_open").map((call) => call.args);

test("restoring gives each pane its tabs, ids, histories and looks; only the shown ones open", () => {
  const core = fakeListings();
  restoreTabs(session);

  const { left, right } = useTabs.getState();
  expect(left.tabs).toEqual([
    { id: 1, pinned: true },
    { id: 2, pinned: false },
  ]);
  expect(left.active).toBe(1);
  expect(right.tabs).toEqual([{ id: 3, pinned: false }]);
  expect(useWindow.getState().activePane).toBe("right");

  const first = tabById(1);
  expect(first?.store.getState().index).toBe(1);
  expect(first?.current()).toMatchObject({ name: "04_Issued", focus: "plan.dwg" });
  expect(first?.store.getState().history[0]).not.toHaveProperty("focus");
  expect(first?.folder.store.getState().sort).toEqual(BY_DATE);
  expect(first?.folder.store.getState().columns.hidden).toEqual([]);

  // Tab 1 opens with its sort, so it gets the listing the core started for it.
  expect(opens(core)).toEqual([
    { tab: 1, target: { kind: "folder", folder: 11 }, sort: BY_DATE, onMessage: expect.anything() },
    {
      tab: 3,
      target: { kind: "folder", folder: 13 },
      sort: { column: "name", descending: false, foldersFirst: true },
      onMessage: expect.anything(),
    },
  ]);

  // A new tab continues after the restored ids.
  expect(addTab("left").id).toBe(4);
});

test("a hidden restored tab opens its folder when it first shows", () => {
  const core = fakeListings();
  restoreTabs(session);
  expect(opens(core).map((call) => call.tab)).toEqual([1, 3]);

  activateTab("left", 2);
  expect(opens(core).at(-1)).toMatchObject({ tab: 2, target: { kind: "folder", folder: 12 } });
  activateTab("left", 1);
  activateTab("left", 2);
  expect(opens(core)).toHaveLength(3);
});

test("a restored folder that no longer exists opens its nearest parent, with a toast", async () => {
  const core = fakeListings({
    messages: (target: ListingTarget) =>
      target.kind === "folder" && target.folder === 13
        ? [errorMessage(JSON.stringify({ code: "NotFound", path: null }))]
        : [batchMessage(0, [{ name: "a.dwg" }]), orderMessage([0]), doneMessage(1, 1, 1)],
    opened: (target) =>
      target.kind === "nearest"
        ? { path: String.raw`C:\Work`, name: "Work", folder: 1, parent: null }
        : {},
  });
  restoreTabs(session);

  const right = tabById(3);
  await waitFor(() => expect(right?.current()?.name).toBe("Work"));
  expect(opens(core).at(-1)).toMatchObject({ tab: 3, target: { kind: "nearest", folder: 13 } });
  expect(right?.store.getState()).toMatchObject({ index: 0 });
  expect(right?.store.getState().history).toHaveLength(1);
  expect(useToasts.getState().toasts).toEqual([
    {
      id: expect.any(Number),
      title: "“Right” no longer exists",
      detail: "Opened “Work”, the nearest folder left",
    },
  ]);
  // The folder that is there opened as it was: no toast for it.
  await waitFor(() => expect(tabById(1)?.folder.store.getState().status).toBe("ready"));
  expect(useToasts.getState().toasts).toHaveLength(1);
});

test("if the core can't say what to restore, each pane opens the user's folder", async () => {
  const core = fakeListings({
    other: (cmd) =>
      cmd === "session_restore" ? Promise.reject({ code: "Internal", path: null }) : null,
  });
  await restoreSession();
  expect(opens(core).map((call) => call.target)).toEqual([{ kind: "home" }, { kind: "home" }]);
  expect(useTabs.getState().left.tabs).toHaveLength(1);
});

test("the session goes to the core on every change, once per burst and only when it changed", async () => {
  const core = fakeListings({
    other: (cmd) => (cmd === "session_restore" ? session : null),
  });
  await restoreSession();
  const stop = saveSession();
  const saves = () =>
    core.calls
      .filter((call) => call.cmd === "session_save")
      .map((call) => call.args.session as SessionSave);

  await waitFor(() => expect(saves()).toHaveLength(1));
  expect(saves()[0]).toEqual(snapshot());
  expect(saves()[0]?.left).toEqual({
    tabs: [
      {
        pinned: true,
        history: [
          { folder: 10, focus: null },
          { folder: 11, focus: "plan.dwg" },
        ],
        index: 1,
        view: {
          view: "details",
          sort: BY_DATE,
          columns: { order: [...DEFAULT_COLUMNS.order], hidden: [], widths: {} },
        },
      },
      expect.objectContaining({ pinned: false, index: 0 }),
    ],
    active: 0,
  });
  expect(saves()[0]?.activePane).toBe("right");

  // Several changes at once: one message.
  activateTab("left", 2);
  useWindow.setState({ activePane: "left" });
  await waitFor(() => expect(saves()).toHaveLength(2));
  expect(saves()[1]).toMatchObject({ activePane: "left", left: { active: 1 } });

  // A change the session doesn't keep (a batch, a selection): nothing goes.
  tabById(1)?.folder.setSelection({ selected: new Set([0]), anchor: 0, focused: 0 });
  await new Promise((resolve) => setTimeout(resolve, 10));
  expect(saves()).toHaveLength(2);

  // A sort is kept.
  tabById(3)?.folder.sortBy("size");
  await waitFor(() => expect(saves()).toHaveLength(3));
  expect(saves()[2]?.right.tabs[0]?.view.sort.column).toBe("size");
  tabById(3)?.folder.close();
  stop();
});
