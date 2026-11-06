import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { commandById, runCommand } from "../../commands/registry";
import type { Favorite, FavoriteGroup, FavoriteList, JerryError, RecentFolder } from "../../ipc";
import { fakeListings } from "../../ipc/listing.testing";
import { useSettings } from "../../stores/settings";
import { defaultSettings } from "../../stores/settings.testing";
import { addTab, resetTabs } from "../../stores/tabs";
import { useWindow } from "../../stores/window";
import { folderName } from "./FavoritesSection";
import {
  dropOf,
  type FavoriteRow,
  favoriteRows,
  resetFavorites,
  syncFavorites,
  useFavorites,
} from "./favorites";
import { Sidebar } from "./Sidebar";
import { resetSidebar } from "./store";

function favorite(id: number, name: string, group: number | null = null): Favorite {
  return { id, path: `D:\\Projects\\${name}`, name, renamed: false, group };
}

const insta: FavoriteGroup = { id: 1, name: "Insta" };
const ts: FavoriteGroup = { id: 2, name: "TSGroup" };

/** Top level: Inbox. Insta: BMW, Audi. TSGroup: Site. */
const list: FavoriteList = {
  groups: [insta, ts],
  favorites: [
    favorite(10, "Inbox"),
    favorite(11, "BMW", 1),
    favorite(12, "Audi", 1),
    favorite(13, "Site", 2),
  ],
};

const recent: RecentFolder[] = [
  { path: String.raw`D:\Projects\BMW\Drawings`, name: "Drawings" },
  { path: String.raw`D:\Projects\Audi\Drawings`, name: "Drawings" },
  { path: String.raw`C:\Users\TS\Downloads`, name: "Downloads" },
];

function show(more: Partial<ReturnType<typeof useFavorites.getState>> = {}) {
  useFavorites.setState({ ...list, recent, ...more });
}

const nav = () => screen.getByRole("navigation", { name: "Navigation" });
const keys = (rows: FavoriteRow[]) => rows.map((row) => row.key);

/** The sidebar, once dragging (loaded lazily) has taken over its favorites. */
async function renderSidebar() {
  render(<Sidebar />);
  if (useFavorites.getState().favorites.length > 0) {
    await waitFor(() =>
      expect(within(nav()).getAllByRole("button", { name: "Inbox" })[0]).toHaveAttribute(
        "aria-roledescription",
        "sortable",
      ),
    );
  }
}

let stop: (() => void) | undefined;

beforeEach(() => {
  useSettings.setState({ settings: defaultSettings });
});

afterEach(() => {
  stop?.();
  stop = undefined;
  resetFavorites();
  resetSidebar();
  resetTabs();
  useWindow.setState({ activePane: "left" });
  useSettings.setState({ settings: undefined });
});

describe("the Favorites section's lines", () => {
  test("are the top level, then each group's caption over its favorites", () => {
    expect(keys(favoriteRows(list.groups, list.favorites, new Set()))).toEqual([
      "f10",
      "g1",
      "f11",
      "f12",
      "g2",
      "f13",
    ]);
    // A folded group, and a group being dragged, show their caption only.
    expect(keys(favoriteRows(list.groups, list.favorites, new Set([1])))).toEqual([
      "f10",
      "g1",
      "g2",
      "f13",
    ]);
    expect(keys(favoriteRows(list.groups, list.favorites, new Set(), 2))).toEqual([
      "f10",
      "g1",
      "f11",
      "f12",
      "g2",
    ]);
  });

  test("a dropped favorite joins the group whose caption is nearest above it", () => {
    const rows = favoriteRows(list.groups, list.favorites, new Set());
    // Inbox onto Insta's caption, from above: first in Insta.
    expect(dropOf(rows, 0, 1)).toEqual({ kind: "favorite", id: 10, group: 1, index: 0 });
    // Audi onto Insta's caption, from below: last of the top level.
    expect(dropOf(rows, 3, 1)).toEqual({ kind: "favorite", id: 12, group: null, index: 1 });
    // Site onto BMW: before it, in Insta.
    expect(dropOf(rows, 5, 2)).toEqual({ kind: "favorite", id: 13, group: 1, index: 0 });
    // BMW onto TSGroup's caption: first in TSGroup.
    expect(dropOf(rows, 2, 4)).toEqual({ kind: "favorite", id: 11, group: 2, index: 0 });
    // BMW onto Audi: within Insta.
    expect(dropOf(rows, 2, 3)).toEqual({ kind: "favorite", id: 11, group: 1, index: 1 });
  });

  test("a dropped group goes among the groups, after the top level", () => {
    const lifted = favoriteRows(list.groups, list.favorites, new Set(), 1);
    expect(keys(lifted)).toEqual(["f10", "g1", "g2", "f13"]);
    expect(dropOf(lifted, 1, 3)).toEqual({ kind: "group", id: 1, index: 1 });
    const second = favoriteRows(list.groups, list.favorites, new Set(), 2);
    expect(dropOf(second, 4, 0)).toEqual({ kind: "group", id: 2, index: 0 });
  });

  test("a folder's own name is its last part, or its root", () => {
    expect(folderName(String.raw`D:\Projects\2026\BMW\Drawings`)).toBe("Drawings");
    expect(folderName("D:\\")).toBe("D:");
    expect(folderName(String.raw`\\INSTA-NAS01\Projects`)).toBe(String.raw`\\INSTA-NAS01\Projects`);
    expect(folderName(String.raw`\\INSTA-NAS01\Projects\ნახაზები`)).toBe("ნახაზები");
  });
});

test("favorites show in their groups, above Recent and the places, and groups fold", async () => {
  mockIPC(() => null);
  show();
  await renderSidebar();

  const names = within(nav())
    .getAllByRole("button")
    .map((button) => button.getAttribute("aria-label") ?? button.textContent);
  expect(names.slice(0, 9)).toEqual([
    "Show or hide sidebar (Ctrl+B)",
    "Settings (Ctrl+,)",
    "Hide Favorites",
    "Add to favorites (Ctrl+Shift+D)",
    "Inbox",
    "Hide Insta",
    "BMW",
    "Audi",
    "Hide TSGroup",
  ]);

  fireEvent.click(within(nav()).getByRole("button", { name: "Hide Insta" }));
  expect(within(nav()).queryByRole("button", { name: "BMW" })).toBeNull();
  expect(within(nav()).getByRole("button", { name: "Site" })).toBeInTheDocument();
  fireEvent.click(within(nav()).getByRole("button", { name: "Show Insta" }));
  expect(within(nav()).getByRole("button", { name: "BMW" })).toBeInTheDocument();

  fireEvent.click(within(nav()).getByRole("button", { name: "Hide Favorites" }));
  expect(within(nav()).queryByRole("button", { name: "Inbox" })).toBeNull();
});

test("Recent unfolds the latest folders; two of a name show where they are", async () => {
  mockIPC(() => null);
  show();
  await renderSidebar();
  expect(within(nav()).queryByRole("button", { name: /Downloads/ })).toBeNull();

  fireEvent.click(within(nav()).getByRole("button", { name: "Show Recent" }));
  expect(within(nav()).getByRole("button", { name: /^Drawings\s*BMW$/ })).toHaveAttribute(
    "title",
    String.raw`D:\Projects\BMW\Drawings`,
  );
  expect(within(nav()).getByRole("button", { name: /^Drawings\s*Audi$/ })).toBeInTheDocument();
  expect(within(nav()).getByRole("button", { name: /^Downloads$/ })).toBeInTheDocument();
});

test("a click opens a favorite, the one the tab shows is selected", async () => {
  const core = fakeListings({ path: String.raw`D:\Projects\BMW` });
  const tab = addTab("left");
  show();
  await renderSidebar();

  fireEvent.click(within(nav()).getByRole("button", { name: "BMW" }));
  await waitFor(() => expect(tab.current()?.path).toBe(String.raw`D:\Projects\BMW`));
  expect(core.calls.find((call) => call.cmd === "listing_open")?.args.target).toEqual({
    kind: "path",
    path: String.raw`D:\Projects\BMW`,
  });
  expect(within(nav()).getByRole("button", { name: "BMW" })).toHaveAttribute(
    "aria-current",
    "location",
  );
  // Opening checks it again.
  expect(core.calls).toContainEqual({ cmd: "favorites_check", args: { id: 11 } });
});

test("a favorite whose folder is gone is grey, and a click says why", async () => {
  const gone: JerryError = { code: "DeviceGone", path: String.raw`E:\Site` };
  let there = false;
  const core = fakeListings({
    other: (cmd) => (cmd === "favorites_check" ? (there ? null : gone) : null),
  });
  addTab("left");
  show({ problems: new Map([[13, gone]]) });
  await renderSidebar();

  const site = within(nav()).getByRole("button", { name: "Site" });
  expect(site).toHaveClass("text-disabled");
  expect(site).toHaveAttribute("title", String.raw`D:\Projects\Site (not available)`);
  fireEvent.click(site);
  const dialog = await screen.findByRole("dialog", { name: "Can't open Site" });
  expect(dialog).toHaveTextContent(/of D:\\Projects\\Site isn't connected/);
  expect(core.calls.some((call) => call.cmd === "listing_open")).toBe(false);

  // Plugged in: Try again opens it.
  there = true;
  fireEvent.click(within(dialog).getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  await waitFor(() =>
    expect(core.calls.find((call) => call.cmd === "listing_open")?.args.target).toEqual({
      kind: "path",
      path: String.raw`D:\Projects\Site`,
    }),
  );
  expect(within(nav()).getByRole("button", { name: "Site" })).not.toHaveClass("text-disabled");
});

test("a deleted favorite says so, and can be removed from the explanation", async () => {
  const missing: JerryError = { code: "NotFound", path: String.raw`D:\Projects\Audi` };
  const core = fakeListings({ other: (cmd) => (cmd === "favorites_check" ? missing : null) });
  show({ problems: new Map([[12, missing]]) });
  await renderSidebar();

  fireEvent.click(within(nav()).getByRole("button", { name: "Audi" }));
  const dialog = await screen.findByRole("dialog", { name: "Can't open Audi" });
  expect(dialog).toHaveTextContent(/D:\\Projects\\Audi isn't there any more/);
  fireEvent.click(within(dialog).getByRole("button", { name: "Remove from favorites" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(core.calls).toContainEqual({ cmd: "favorites_remove", args: { id: 12 } });
});

test("F2 renames a favorite in place; empty brings back its folder's name", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return null;
  });
  show();
  await renderSidebar();

  fireEvent.keyDown(within(nav()).getByRole("button", { name: "BMW" }), { key: "F2" });
  const field = within(nav()).getByRole("textbox", { name: "Favorite name" });
  expect(field).toHaveFocus();
  expect(field).toHaveValue("BMW");
  fireEvent.change(field, { target: { value: "BMW ნახაზები" } });
  fireEvent.keyDown(field, { key: "Enter" });
  await waitFor(() => expect(within(nav()).queryByRole("textbox")).toBeNull());
  expect(calls).toContainEqual({
    cmd: "favorites_rename",
    args: { id: 11, name: "BMW ნახაზები" },
  });

  fireEvent.contextMenu(within(nav()).getByRole("button", { name: "Audi" }));
  fireEvent.click(await screen.findByRole("menuitem", { name: "Rename" }));
  // It opens once the menu has closed, and keeps the focus the menu would give back.
  const again = await within(nav()).findByRole("textbox", { name: "Favorite name" });
  await waitFor(() => expect(again).toHaveFocus());
  expect(again).toHaveAttribute("placeholder", "Audi");
  fireEvent.change(again, { target: { value: "  " } });
  fireEvent.keyDown(again, { key: "Enter" });
  await waitFor(() =>
    expect(calls).toContainEqual({ cmd: "favorites_rename", args: { id: 12, name: null } }),
  );

  // Esc leaves the name as it was.
  fireEvent.keyDown(within(nav()).getByRole("button", { name: "Site" }), { key: "F2" });
  fireEvent.keyDown(within(nav()).getByRole("textbox"), { key: "Escape" });
  expect(within(nav()).queryByRole("textbox")).toBeNull();
  expect(calls.filter((call) => call.cmd === "favorites_rename")).toHaveLength(2);
});

test("groups are made, renamed and removed from their menus", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "favorites_group_add" && (args as { name: string }).name.trim() === "") {
      throw { code: "InvalidName", path: null };
    }
    return cmd === "favorites_group_add" ? 3 : null;
  });
  show();
  await renderSidebar();

  fireEvent.contextMenu(within(nav()).getByRole("button", { name: "Hide Favorites" }));
  fireEvent.click(await screen.findByRole("menuitem", { name: "New group" }));
  const field = await within(nav()).findByRole("textbox", { name: "Group name" });
  await waitFor(() => expect(field).toHaveFocus());
  fireEvent.change(field, { target: { value: " " } });
  fireEvent.keyDown(field, { key: "Enter" });
  expect(await within(nav()).findByText("Type a name for the group.")).toHaveClass("text-red");
  fireEvent.change(field, { target: { value: "პირადი" } });
  fireEvent.keyDown(field, { key: "Enter" });
  await waitFor(() => expect(within(nav()).queryByRole("textbox")).toBeNull());
  expect(calls).toContainEqual({ cmd: "favorites_group_add", args: { name: "პირადი" } });

  fireEvent.keyDown(within(nav()).getByRole("button", { name: "Hide TSGroup" }), { key: "F2" });
  const rename = within(nav()).getByRole("textbox", { name: "Group name" });
  expect(rename).toHaveValue("TSGroup");
  fireEvent.change(rename, { target: { value: "TS Group" } });
  fireEvent.blur(rename);
  await waitFor(() =>
    expect(calls).toContainEqual({
      cmd: "favorites_group_rename",
      args: { id: 2, name: "TS Group" },
    }),
  );

  fireEvent.contextMenu(within(nav()).getByRole("button", { name: "Hide Insta" }));
  fireEvent.click(await screen.findByRole("menuitem", { name: /Remove group/ }));
  expect(calls).toContainEqual({ cmd: "favorites_group_remove", args: { id: 1 } });
});

test("Ctrl+Shift+D adds the folder the active pane shows", async () => {
  const core = fakeListings({
    path: String.raw`D:\Projects\BMW\Drawings`,
    other: (cmd) => (cmd === "favorites_add" ? 20 : null),
  });
  const command = commandById("addFavorite");
  expect(command && runCommand(command)).toBe(false);

  const tab = addTab("left");
  await tab.open({ kind: "path", path: String.raw`D:\Projects\BMW\Drawings` });
  expect(command && runCommand(command)).toBe(true);
  await waitFor(() =>
    expect(core.calls).toContainEqual({
      cmd: "favorites_add",
      args: { listing: 1, entry: null },
    }),
  );
});

test("an empty Favorites says how to add one", async () => {
  mockIPC(() => null);
  useFavorites.setState({ recent });
  await renderSidebar();
  expect(within(nav()).getByText("Ctrl+Shift+D adds the folder you're in.")).toBeInTheDocument();
});

test("the store loads the favorites and recent folders, follows the core and checks them", async () => {
  const checked: number[] = [];
  const gone: JerryError = { code: "NotFound", path: String.raw`D:\Projects\Audi` };
  mockIPC(
    (cmd, args) => {
      switch (cmd) {
        case "favorites_list":
          return list;
        case "recent_folders_list":
          return recent;
        case "favorites_check": {
          const id = (args as { id: number }).id;
          checked.push(id);
          return id === 12 ? gone : null;
        }
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
  stop = syncFavorites();
  await waitFor(() => expect(useFavorites.getState().favorites).toHaveLength(4));
  expect(useFavorites.getState().recent).toHaveLength(3);
  await waitFor(() => expect(useFavorites.getState().problems.get(12)).toEqual(gone));
  expect(checked.sort()).toEqual([10, 11, 12, 13]);

  // A change from another window: Audi is gone, a group folded here goes with it.
  useFavorites.setState({ folded: new Set([1, 2]) });
  await emit("favorites:changed", {
    groups: [ts],
    favorites: [favorite(10, "Inbox"), favorite(13, "Site", 2)],
  } satisfies FavoriteList);
  expect(useFavorites.getState().favorites.map((each) => each.id)).toEqual([10, 13]);
  expect(useFavorites.getState().problems.size).toBe(0);
  expect([...useFavorites.getState().folded]).toEqual([2]);

  await emit("recent-folders:changed", recent.slice(0, 1));
  expect(useFavorites.getState().recent).toHaveLength(1);

  // The window comes back to the front: checked again.
  checked.length = 0;
  window.dispatchEvent(new Event("focus"));
  await waitFor(() => expect(checked.sort()).toEqual([10, 13]));
});
