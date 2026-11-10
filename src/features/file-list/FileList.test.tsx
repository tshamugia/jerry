import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, expect, test, vi } from "vitest";
import type { Settings } from "../../ipc";
import {
  batchMessage,
  diffMessage,
  doneMessage,
  errorMessage,
  type FakeListings,
  fakeListings,
  orderMessage,
  type TestEntry,
} from "../../ipc/listing.testing";
import { useSettings } from "../../stores/settings";
import { defaultSettings } from "../../stores/settings.testing";
import { Tab } from "../../stores/tabs";
import { useToasts } from "../../stores/toasts";
import { useWindow } from "../../stores/window";
import { FileList } from "./FileList";

// jsdom lays nothing out: give the list a viewport, so the virtualizer renders rows.
const clientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
beforeAll(() => {
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get: () => 600,
  });
  // Radix menus measure their content.
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
});
afterAll(() => {
  if (clientHeight) Object.defineProperty(HTMLElement.prototype, "clientHeight", clientHeight);
});
afterEach(() => {
  useSettings.setState({ settings: undefined });
  useWindow.setState({ activePane: "left" });
});

const at = (day: number, hour: number, minute: number) =>
  new Date(2026, 9, day, hour, minute).getTime();

const WORK: TestEntry[] = [
  { name: "ELV-001 LAN Layout GF.dwg", size: 5_033_164, modified: at(30, 15, 12) },
  { name: "Plot", attributes: 0x10, modified: at(28, 17, 40) },
  { name: "გეგმა.ნახ", size: 2048, modified: at(31, 11, 48), created: at(1, 9, 0) },
  { name: "desktop.ini", attributes: 0x2 | 0x4, size: 282 },
];

/** Opens a folder with `entries` (in arrival order) shown in `order`, and renders its list. */
async function listed(
  entries: TestEntry[],
  order: number[],
  settings: Partial<Settings> = {},
  options: Parameters<typeof fakeListings>[0] = {},
) {
  useSettings.setState({ settings: { ...defaultSettings, ...settings } });
  const core = fakeListings({
    messages: () => [
      batchMessage(0, entries),
      orderMessage(order),
      doneMessage(entries.length, 1, 1),
    ],
    ...options,
  });
  const tab = new Tab(7);
  render(<FileList tab={tab} pane="left" />);
  await act(() => tab.open({ kind: "path", path: String.raw`C:\Work` }));
  const { folder } = tab;
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  return { core, folder, tab };
}

const grid = () => screen.getByRole("grid", { name: "Left pane files" });
const rows = () => within(grid()).getAllByRole("row").slice(1);
const rowNames = () => rows().map((row) => within(row).getAllByRole("gridcell")[0]?.textContent);
const cells = (name: string) => {
  const row = rows().find((each) => each.textContent?.startsWith(name));
  return within(row as HTMLElement)
    .getAllByRole("gridcell")
    .map((cell) => cell.textContent);
};
const header = (name: string) => screen.getByRole("columnheader", { name: new RegExp(`^${name}`) });

test("rows show the design's columns: name with its icon, date, type and size", async () => {
  await listed(WORK, [1, 0, 2, 3]);

  expect(
    within(grid())
      .getAllByRole("columnheader")
      .map((cell) => cell.textContent),
  ).toEqual(["Name", "Date modified", "Type", "Size"]);
  expect(rowNames()).toEqual(["Plot", "ELV-001 LAN Layout GF.dwg", "გეგმა.ნახ"]);
  expect(cells("Plot")).toEqual(["Plot", "28.10.2026 17:40", "File folder", ""]);
  expect(cells("ELV")).toEqual([
    "ELV-001 LAN Layout GF.dwg",
    "30.10.2026 15:12",
    "DWG File",
    "4.8 MB",
  ]);
  // Georgian stays Mkhedruli in the Type column.
  expect(cells("გეგმა")).toEqual(["გეგმა.ნახ", "31.10.2026 11:48", "ნახ File", "2 KB"]);

  const icon = rows()[1]?.querySelector("img");
  expect(new URL(icon?.src ?? "").pathname).toMatch(/^\/type-3-16$/);
  expect(header("Name")).toHaveAttribute("aria-sort", "ascending");
});

test("rows have the density's fixed height, one after another", async () => {
  await listed(WORK, [1, 0, 2, 3]);
  const [first, second] = rows();
  expect(first).toHaveClass("h-row");
  expect(first?.style.transform).toBe("translateY(0px)");
  expect(second?.style.transform).toBe("translateY(30px)");
  expect(grid()).toHaveAttribute("aria-rowcount", "4");
});

test("hidden and system files show dimmed only with the setting on", async () => {
  await listed(WORK, [1, 0, 2, 3], { showHidden: true });
  expect(rowNames()).toContain("desktop.ini");
  const name = within(rows()[3] as HTMLElement).getByText("desktop.ini");
  expect(name).toHaveClass("opacity-60");

  act(() => useSettings.setState({ settings: { ...defaultSettings, showHidden: false } }));
  expect(rowNames()).not.toContain("desktop.ini");
});

test("extensions hide with the setting off", async () => {
  await listed(WORK, [1, 0, 2, 3], { showExtensions: false });
  expect(rowNames()).toEqual(["Plot", "ELV-001 LAN Layout GF", "გეგმა"]);
});

test("clicking a column header sorts by it; clicking it again flips the direction", async () => {
  const { core } = await listed(
    WORK,
    [1, 0, 2, 3],
    {},
    {
      sorted: (sort) =>
        orderMessage(sort.descending ? [1, 0, 2, 3] : [1, 2, 0, 3], {
          column: 3,
          descending: sort.descending,
        }),
    },
  );

  fireEvent.click(within(header("Size")).getByRole("button"));
  await waitFor(() =>
    expect(rowNames()).toEqual(["Plot", "გეგმა.ნახ", "ELV-001 LAN Layout GF.dwg"]),
  );
  expect(header("Size")).toHaveAttribute("aria-sort", "ascending");
  expect(header("Name")).toHaveAttribute("aria-sort", "none");

  fireEvent.click(within(header("Size")).getByRole("button"));
  await waitFor(() =>
    expect(rowNames()).toEqual(["Plot", "ELV-001 LAN Layout GF.dwg", "გეგმა.ნახ"]),
  );
  expect(header("Size")).toHaveAttribute("aria-sort", "descending");
  expect(
    core.calls.filter((call) => call.cmd === "listing_sort").map((call) => call.args.sort),
  ).toEqual([
    { column: "size", descending: false, foldersFirst: true },
    { column: "size", descending: true, foldersFirst: true },
  ]);
});

test("the folders-first setting re-sorts the list", async () => {
  const { core } = await listed(WORK, [1, 0, 2, 3]);
  act(() => useSettings.setState({ settings: { ...defaultSettings, foldersFirst: false } }));
  expect(
    core.calls.filter((call) => call.cmd === "listing_sort").map((call) => call.args.sort),
  ).toEqual([{ column: "name", descending: false, foldersFirst: false }]);
});

test("changes in the folder repaint the rows", async () => {
  const { core } = await listed(WORK, [1, 0, 2, 3]);

  act(() => {
    core.send(diffMessage([2, 4], [0], [{ name: "გეგმა.ნახ", size: 4096 }, { name: "new.txt" }]));
    core.send(orderMessage([1, 2, 4, 3]));
  });

  expect(rowNames()).toEqual(["Plot", "გეგმა.ნახ", "new.txt"]);
  expect(cells("გეგმა")[3]).toBe("4 KB");
});

/** A press and release of the left button, and the click that follows, as a mouse makes them. */
function click(target: Element, init: { ctrlKey?: boolean; shiftKey?: boolean } = {}) {
  fireEvent.pointerDown(target, { button: 0, pointerId: 1, ...init });
  fireEvent.pointerUp(target, { button: 0, pointerId: 1, ...init });
  fireEvent.click(target, init);
}
const nameOf = (name: string) => within(grid()).getByText(name);
/** A row's Date modified cell: a press there can start a marquee. */
const dateOf = (index: number) =>
  within(rows()[index] as HTMLElement).getAllByRole("gridcell")[1] as HTMLElement;
const selectedNames = () =>
  rows()
    .filter((row) => row.getAttribute("aria-selected") === "true")
    .map((row) => within(row).getAllByRole("gridcell")[0]?.textContent);
const key = (code: string, init: { key?: string; ctrlKey?: boolean; shiftKey?: boolean } = {}) =>
  fireEvent.keyDown(grid(), { code, key: code, ...init });

/** Ten files, f0.txt to f9.txt, shown in id order. */
const TEN: TestEntry[] = Array.from({ length: 10 }, (_, i) => ({ name: `f${i}.txt` }));
const IN_ORDER = TEN.map((_, i) => i);

test("a click selects a row; selection is accent-tinted only in the active pane", async () => {
  await listed(WORK, [1, 0, 2, 3]);
  const row = () => rows()[1] as HTMLElement;

  click(nameOf("ELV-001 LAN Layout GF.dwg"));
  expect(row()).toHaveAttribute("aria-selected", "true");
  expect(row()).toHaveClass("bg-sel");
  expect(rows()[0]).toHaveClass("hover:bg-hover");
  expect(grid()).toHaveAttribute("aria-multiselectable", "true");

  act(() => useWindow.setState({ activePane: "right" }));
  expect(row()).toHaveClass("bg-sel-inactive");
});

test("Ctrl+click toggles rows, Shift+click selects a range, Ctrl+Shift+click adds one", async () => {
  await listed(TEN, IN_ORDER);

  click(nameOf("f1.txt"));
  click(nameOf("f3.txt"), { ctrlKey: true });
  expect(selectedNames()).toEqual(["f1.txt", "f3.txt"]);
  click(nameOf("f1.txt"), { ctrlKey: true });
  expect(selectedNames()).toEqual(["f3.txt"]);

  // As in Explorer, the anchor is the last row clicked without Shift: f1, though deselected.
  click(nameOf("f4.txt"), { shiftKey: true });
  expect(selectedNames()).toEqual(["f1.txt", "f2.txt", "f3.txt", "f4.txt"]);
  click(nameOf("f2.txt"), { shiftKey: true });
  expect(selectedNames()).toEqual(["f1.txt", "f2.txt"]);

  click(nameOf("f8.txt"), { ctrlKey: true });
  click(nameOf("f9.txt"), { ctrlKey: true, shiftKey: true });
  expect(selectedNames()).toEqual(["f1.txt", "f2.txt", "f8.txt", "f9.txt"]);
});

test("a press on a selected row keeps the selection until release, then selects only it", async () => {
  await listed(TEN, IN_ORDER);
  click(nameOf("f1.txt"));
  click(nameOf("f4.txt"), { shiftKey: true });

  fireEvent.pointerDown(nameOf("f2.txt"), { button: 0, pointerId: 1 });
  expect(selectedNames()).toEqual(["f1.txt", "f2.txt", "f3.txt", "f4.txt"]);
  fireEvent.pointerUp(nameOf("f2.txt"), { button: 0, pointerId: 1 });
  expect(selectedNames()).toEqual(["f2.txt"]);
});

test("a click on empty space clears the selection; a right click selects a row not selected", async () => {
  await listed(TEN.slice(0, 3), [0, 1, 2]);
  click(nameOf("f0.txt"));
  click(nameOf("f1.txt"), { ctrlKey: true });

  // Right click on a selected row keeps the selection; on another row, selects it alone.
  fireEvent.pointerDown(nameOf("f1.txt"), { button: 2, pointerId: 1 });
  expect(selectedNames()).toEqual(["f0.txt", "f1.txt"]);
  fireEvent.pointerDown(nameOf("f2.txt"), { button: 2, pointerId: 1 });
  expect(selectedNames()).toEqual(["f2.txt"]);

  // Below the rows.
  const empty = grid().firstElementChild as HTMLElement;
  click(empty);
  expect(selectedNames()).toEqual([]);
});

test("a marquee from a row's other columns selects the rows it crosses, then is gone", async () => {
  await listed(TEN, IN_ORDER);
  click(nameOf("f0.txt"));

  // jsdom lays nothing out: client y is y from the first row's top; rows are 30 px.
  fireEvent.pointerDown(dateOf(1), { button: 0, pointerId: 1, clientX: 300, clientY: 35 });
  expect(selectedNames()).toEqual(["f0.txt"]);
  fireEvent.pointerMove(grid(), { pointerId: 1, clientX: 320, clientY: 95 });
  expect(selectedNames()).toEqual(["f1.txt", "f2.txt", "f3.txt"]);
  const drawn = grid().querySelector<HTMLElement>("[data-marquee]");
  expect(drawn?.style.top).toBe("35px");
  expect(drawn?.style.height).toBe("60px");

  // Back up: rows it no longer crosses leave the selection.
  fireEvent.pointerMove(grid(), { pointerId: 1, clientX: 320, clientY: 50 });
  expect(selectedNames()).toEqual(["f1.txt"]);
  fireEvent.pointerUp(grid(), { pointerId: 1, clientX: 320, clientY: 50 });
  fireEvent.click(grid());
  expect(grid().querySelector("[data-marquee]")).toBeNull();
  expect(selectedNames()).toEqual(["f1.txt"]);
});

test("a marquee with Ctrl toggles the rows it crosses; a press on a name never starts one", async () => {
  await listed(TEN, IN_ORDER);
  click(nameOf("f1.txt"));
  click(nameOf("f4.txt"), { ctrlKey: true });

  fireEvent.pointerDown(dateOf(0), { button: 0, pointerId: 1, ctrlKey: true, clientY: 5 });
  fireEvent.pointerMove(grid(), { pointerId: 1, ctrlKey: true, clientY: 65 });
  fireEvent.pointerUp(grid(), { pointerId: 1, ctrlKey: true, clientY: 65 });
  expect(selectedNames()).toEqual(["f0.txt", "f2.txt", "f4.txt"]);

  fireEvent.pointerDown(nameOf("f6.txt"), { button: 0, pointerId: 1, clientY: 185 });
  fireEvent.pointerMove(grid(), { pointerId: 1, clientY: 275 });
  fireEvent.pointerUp(grid(), { pointerId: 1, clientY: 275 });
  expect(selectedNames()).toEqual(["f6.txt"]);
});

const dragsStarted = (core: FakeListings) =>
  core.calls.filter((call) => call.cmd === "drag_start").map((call) => call.args);

test("a selected item pressed and moved past the threshold drags the whole selection out", async () => {
  const { core } = await listed(TEN, [0, 4, 3, 2, 1, 5, 6, 7, 8, 9]);
  click(nameOf("f4.txt"));
  click(nameOf("f2.txt"), { shiftKey: true });

  // A move within Windows' threshold (4 px) is still a click.
  fireEvent.pointerDown(nameOf("f3.txt"), { button: 0, pointerId: 1, clientX: 40, clientY: 65 });
  fireEvent.pointerMove(grid(), { pointerId: 1, buttons: 1, clientX: 42, clientY: 67 });
  expect(dragsStarted(core)).toEqual([]);
  fireEvent.pointerMove(grid(), { pointerId: 1, buttons: 1, clientX: 46, clientY: 70 });
  // In display order; paths stay in Rust.
  expect(dragsStarted(core)).toEqual([{ listing: 1, entries: [4, 3, 2] }]);

  // Windows took the drag: the release a page might still see doesn't click the row.
  fireEvent.pointerUp(nameOf("f3.txt"), { button: 0, pointerId: 1 });
  fireEvent.pointerMove(grid(), { pointerId: 1, buttons: 1, clientX: 90, clientY: 90 });
  expect(selectedNames()).toEqual(["f4.txt", "f3.txt", "f2.txt"]);
  expect(dragsStarted(core)).toHaveLength(1);
  expect(grid().querySelector("[data-marquee]")).toBeNull();
});

test("a right press drags its item out; a move without the button held drags nothing", async () => {
  const { core } = await listed(TEN, IN_ORDER);
  click(nameOf("f1.txt"));

  fireEvent.pointerDown(nameOf("f6.txt"), { button: 2, pointerId: 1, clientY: 185 });
  fireEvent.pointerMove(grid(), { pointerId: 1, buttons: 0, clientY: 215 });
  expect(dragsStarted(core)).toEqual([]);
  fireEvent.pointerMove(grid(), { pointerId: 1, buttons: 2, clientY: 215 });
  expect(dragsStarted(core)).toEqual([{ listing: 1, entries: [6] }]);

  // A right press on a row's other columns isn't a drag.
  fireEvent.pointerDown(dateOf(2), { button: 2, pointerId: 1, clientY: 65 });
  fireEvent.pointerMove(grid(), { pointerId: 1, buttons: 2, clientY: 125 });
  expect(dragsStarted(core)).toHaveLength(1);
});

test("a drag that can't start says why", async () => {
  const gone = { code: "NotFound", path: String.raw`C:\Work\f1.txt`, params: {} };
  await listed(
    TEN,
    IN_ORDER,
    {},
    {
      other: (cmd) => (cmd === "drag_start" ? Promise.reject(gone) : null),
    },
  );
  fireEvent.pointerDown(nameOf("f1.txt"), { button: 0, pointerId: 1, clientY: 35 });
  fireEvent.pointerMove(grid(), { pointerId: 1, buttons: 1, clientY: 45 });
  await waitFor(() =>
    expect(useToasts.getState().toasts.at(-1)).toMatchObject({
      title: "Couldn't drag the files",
      detail: "“f1.txt” couldn't be found. It may have been moved or deleted.",
    }),
  );
});

test("a small move doesn't make a marquee: the release clicks the row", async () => {
  await listed(TEN, IN_ORDER);
  fireEvent.pointerDown(dateOf(2), { button: 0, pointerId: 1, clientY: 65 });
  fireEvent.pointerMove(grid(), { pointerId: 1, clientY: 67 });
  expect(grid().querySelector("[data-marquee]")).toBeNull();
  fireEvent.pointerUp(dateOf(2), { button: 0, pointerId: 1, clientY: 67 });
  expect(selectedNames()).toEqual(["f2.txt"]);
});

test("arrows, Home and End move the selection; Shift selects a range; Ctrl moves the focus only", async () => {
  await listed(TEN, IN_ORDER);
  const focusedName = () =>
    rows()
      .find((row) => row.id === grid().getAttribute("aria-activedescendant"))
      ?.textContent?.slice(0, 6);

  key("ArrowDown");
  expect(selectedNames()).toEqual(["f0.txt"]);
  key("ArrowDown");
  key("ArrowDown");
  expect(selectedNames()).toEqual(["f2.txt"]);
  key("ArrowDown", { shiftKey: true });
  key("ArrowDown", { shiftKey: true });
  expect(selectedNames()).toEqual(["f2.txt", "f3.txt", "f4.txt"]);
  key("ArrowUp", { shiftKey: true });
  expect(selectedNames()).toEqual(["f2.txt", "f3.txt"]);

  key("ArrowDown", { ctrlKey: true });
  key("ArrowDown", { ctrlKey: true });
  expect(focusedName()).toBe("f5.txt");
  expect(selectedNames()).toEqual(["f2.txt", "f3.txt"]);

  key("End");
  expect(selectedNames()).toEqual(["f9.txt"]);
  key("Home", { shiftKey: true });
  expect(selectedNames()).toHaveLength(10);
  key("ArrowUp");
  expect(selectedNames()).toEqual(["f0.txt"]);
});

test("PageDown goes to the last row in view, then a page on, scrolling it into view", async () => {
  const many: TestEntry[] = Array.from({ length: 100 }, (_, i) => ({ name: `f${i}.txt` }));
  const { folder } = await listed(
    many,
    many.map((_, i) => i),
  );
  // jsdom sends no scroll events, so the rows rendered stay the first ones: ask the store.
  const selectedNames = () => {
    const { selected, entries } = folder.store.getState();
    return [...selected].sort((a, b) => a - b).map((id) => entries.names[id]);
  };

  key("Home");
  // 600 px of rows at 30 px: rows 0 to 19 are in view.
  key("PageDown");
  expect(selectedNames()).toEqual(["f19.txt"]);
  expect(grid().scrollTop).toBe(0);
  key("PageDown");
  expect(selectedNames()).toEqual(["f38.txt"]);
  // Scrolled just enough: row 38 at the bottom, rows 19 to 38 in view.
  expect(grid().scrollTop).toBe(39 * 30 - 600);
  key("PageUp", { shiftKey: true });
  expect(selectedNames()[0]).toBe("f19.txt");
  expect(selectedNames()).toHaveLength(20);
  expect(grid().scrollTop).toBe(39 * 30 - 600);
});

test("typing the first letters of a name selects it, Georgian too", async () => {
  await listed(
    [
      { name: "Plot", attributes: 0x10 },
      { name: "plan_rev2.dwg" },
      { name: "გეგმა.ნახ" },
      { name: "გეგმა 2.ნახ" },
      { name: "ნახაზი.dwg" },
    ],
    [0, 1, 2, 3, 4],
  );

  key("KeyG", { key: "გ" });
  expect(selectedNames()).toEqual(["გეგმა.ნახ"]);
  for (const [code, char] of [
    ["KeyE", "ე"],
    ["KeyG", "გ"],
    ["KeyM", "მ"],
    ["KeyA", "ა"],
    ["Space", " "],
  ] as const) {
    key(code, { key: char });
  }
  expect(selectedNames()).toEqual(["გეგმა 2.ნახ"]);

  // A key bound to a command ("?" opens the shortcut table) isn't typed.
  const question = fireEvent.keyDown(grid(), { code: "Slash", key: "?", shiftKey: true });
  expect(question).toBe(true);
});

test("double click opens a folder; with single click, one click does", async () => {
  const { core } = await listed(WORK, [1, 0, 2, 3]);
  const opens = () => core.calls.filter((call) => call.cmd === "listing_open");

  fireEvent.click(screen.getByText("Plot"));
  expect(opens()).toHaveLength(1);
  fireEvent.doubleClick(screen.getByText("Plot"));
  expect(opens().at(-1)?.args.target).toEqual({ kind: "entry", listing: 1, entry: 1 });

  act(() => useSettings.setState({ settings: { ...defaultSettings, clickToOpen: "single" } }));
  await waitFor(() => expect(rowNames()).toContain("Plot"));
  fireEvent.click(screen.getByText("Plot"));
  expect(opens()).toHaveLength(3);
});

test("an empty folder says so", async () => {
  await listed([], []);
  expect(screen.getByText("This folder is empty. Drop files here")).toBeInTheDocument();
});

test("a folder that can't be read says why and offers Retry", async () => {
  useSettings.setState({ settings: defaultSettings });
  const core = fakeListings({
    messages: () => [errorMessage(String.raw`{"code":"AccessDenied","path":"C:\\დახურული"}`)],
  });
  const tab = new Tab(7);
  render(<FileList tab={tab} pane="left" />);
  await act(() => tab.open({ kind: "path", path: String.raw`C:\დახურული` }));

  expect(
    await screen.findByText("You don't have permission to access “დახურული”."),
  ).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() =>
    expect(core.calls.filter((call) => call.cmd === "listing_open").at(-1)?.args.target).toEqual({
      kind: "listing",
      listing: 1,
    }),
  );
});

test("a slow folder shows the loading bar and its first rows at once", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  try {
    useSettings.setState({ settings: defaultSettings });
    const core: FakeListings = fakeListings();
    const tab = new Tab(7);
    render(<FileList tab={tab} pane="left" />);
    await act(() => tab.open({ kind: "path", path: String.raw`\\server\share` }));
    act(() => core.send(batchMessage(0, [{ name: "first.txt" }])));

    expect(rowNames()).toEqual(["first.txt"]);
    expect(grid()).toHaveAttribute("aria-busy", "true");
    expect(grid().querySelector(".animate-loading")).toBeNull();
    act(() => vi.advanceTimersByTime(250));
    expect(grid().querySelector(".animate-loading")).not.toBeNull();
  } finally {
    vi.useRealTimers();
  }
});

test("the header menu shows and hides columns and moves them", async () => {
  const { folder } = await listed(WORK, [1, 0, 2, 3]);

  fireEvent.contextMenu(header("Type"));
  // The menu stays open while columns are switched.
  fireEvent.click(await screen.findByRole("menuitemcheckbox", { name: "Date created" }));
  fireEvent.click(screen.getByRole("menuitemcheckbox", { name: "Size" }));
  expect(screen.getByRole("menuitemcheckbox", { name: "Name" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
  await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  expect(
    within(grid())
      .getAllByRole("columnheader")
      .map((cell) => cell.textContent),
  ).toEqual(["Name", "Date modified", "Type", "Date created"]);
  expect(cells("გეგმა")).toEqual(["გეგმა.ნახ", "31.10.2026 11:48", "ნახ File", "01.10.2026 09:00"]);

  fireEvent.contextMenu(header("Type"));
  fireEvent.click(await screen.findByRole("menuitem", { name: "Move left" }));
  await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  expect(
    within(grid())
      .getAllByRole("columnheader")
      .map((cell) => cell.textContent),
  ).toEqual(["Name", "Type", "Date modified", "Date created"]);

  fireEvent.contextMenu(header("Type"));
  fireEvent.click(await screen.findByRole("menuitem", { name: "Reset columns" }));
  expect(folder.store.getState().columns.hidden).toEqual(["created", "extension"]);
});

test("dragging a column's edge resizes it, the same in the header and the rows", async () => {
  const { folder } = await listed(WORK, [1, 0, 2, 3]);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(
    DOMRect.fromRect({ width: 84 }),
  );
  try {
    const edge = screen.getByRole("separator", { name: "Resize Type" });
    // Type is right of Name: its left edge moves, and dragging it left widens it.
    fireEvent.pointerDown(edge, { button: 0, clientX: 500 });
    fireEvent.pointerMove(edge, { clientX: 470 });
    fireEvent.pointerUp(edge);
  } finally {
    vi.restoreAllMocks();
  }

  expect(folder.store.getState().columns.widths).toEqual({ type: 114 });
  // Header and row cells read their width from the variable.
  expect(grid().style.getPropertyValue("--col-type")).toBe("114px");
  expect(header("Type")).toHaveClass("w-(--col-type)");
  expect(within(rows()[0] as HTMLElement).getAllByRole("gridcell")[2]).toHaveClass(
    "w-(--col-type)",
  );
});
