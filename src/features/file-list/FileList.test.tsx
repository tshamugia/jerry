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
import { Folder } from "../../stores/folder";
import { useSettings } from "../../stores/settings";
import { defaultSettings } from "../../stores/settings.testing";
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
  const folder = new Folder(7);
  render(<FileList folder={folder} pane="left" />);
  await act(() => folder.open({ kind: "path", path: String.raw`C:\Work` }));
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  return { core, folder };
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

test("a click selects a row; selection is accent-tinted only in the active pane", async () => {
  await listed(WORK, [1, 0, 2, 3]);
  const row = () => rows()[1] as HTMLElement;

  fireEvent.click(within(row()).getByText("ELV-001 LAN Layout GF.dwg"));
  expect(row()).toHaveAttribute("aria-selected", "true");
  expect(row()).toHaveClass("bg-sel");
  expect(rows()[0]).toHaveClass("hover:bg-hover");

  act(() => useWindow.setState({ activePane: "right" }));
  expect(row()).toHaveClass("bg-sel-inactive");
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
  const folder = new Folder(7);
  render(<FileList folder={folder} pane="left" />);
  await act(() => folder.open({ kind: "path", path: String.raw`C:\დახურული` }));

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
    const folder = new Folder(7);
    render(<FileList folder={folder} pane="left" />);
    await act(() => folder.open({ kind: "path", path: String.raw`\\server\share` }));
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
