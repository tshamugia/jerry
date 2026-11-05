import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, expect, test, vi } from "vitest";
import { useCommandKeys } from "../../commands/dispatch";
import type { Crumb, ListingTarget } from "../../ipc";
import {
  batchMessage,
  doneMessage,
  type FakeListings,
  fakeListings,
  orderMessage,
} from "../../ipc/listing.testing";
import { useSettings } from "../../stores/settings";
import { defaultSettings } from "../../stores/settings.testing";
import { addTab, resetTabs, type Tab } from "../../stores/tabs";
import { useWindow } from "../../stores/window";
import { AddressBar, fold } from "./AddressBar";
import { useAddressEdit } from "./address";

beforeAll(() => {
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
});

/** `D:\Projects\BMW\Drawings` and its ancestors, by token 1–4; `C:\Other` is 9. */
const CRUMBS: Crumb[] = [
  { name: "D:", folder: 1 },
  { name: "Projects", folder: 2 },
  { name: "BMW", folder: 3 },
  { name: "Drawings", folder: 4 },
];
const OTHER: Crumb = { name: "Other", folder: 9 };

let core: FakeListings;
let tab: Tab;
let resolved: (text: string) => unknown;
beforeEach(async () => {
  resolved = () => ({ folder: 9, select: null });
  core = fakeListings({
    opened: (target: ListingTarget) => {
      const folder = target.kind === "folder" ? target.folder : 4;
      const crumbs = folder === 9 ? [OTHER] : CRUMBS.slice(0, folder);
      const last = crumbs.at(-1) ?? OTHER;
      return { folder, name: last.name, crumbs, path: crumbs.map((c) => c.name).join("\\") };
    },
    messages: () => [
      batchMessage(0, [{ name: "plan.dwg" }, { name: "ნახაზი.dwg" }]),
      orderMessage([0, 1]),
      doneMessage(2, 1, 1),
    ],
    other: (cmd, args) => {
      switch (cmd) {
        case "folder_subfolders":
          return { folders: [{ name: "Audi", folder: 5 }, CRUMBS[2]], more: false };
        case "autocomplete_path":
          return String(args.text).endsWith("\\B")
            ? [String.raw`D:\Projects\BMW`, String.raw`D:\Projects\Bentley`]
            : [];
        case "resolve_path":
          return resolved(String(args.text));
        default:
          return null;
      }
    },
  });
  useSettings.setState({ settings: defaultSettings });
  tab = addTab("left");
  await tab.open({ kind: "path", path: String.raw`D:\Projects\BMW\Drawings` });
});
afterEach(() => {
  resetTabs();
  useAddressEdit.setState({ pane: undefined });
  useSettings.setState({ settings: undefined });
  useWindow.setState({ activePane: "left" });
  vi.useRealTimers();
});

function Bar() {
  useCommandKeys();
  return (
    <fieldset aria-label="Address">
      <AddressBar tab={tab} pane="left" />
    </fieldset>
  );
}

const opens = () => core.calls.filter((call) => call.cmd === "listing_open");
const field = () => screen.getByRole("combobox", { name: "Type an address" });

test("segments show the path, the current one bold; one opens its folder", () => {
  render(<Bar />);
  const bar = screen.getByRole("group", { name: "Address" });
  expect(within(bar).getAllByRole("button", { name: /^Folders in / })).toHaveLength(4);
  const current = within(bar)
    .getAllByText("Drawings")
    .find((each) => each.closest("button") === null);
  expect(current).toHaveClass("font-semibold");

  fireEvent.click(within(bar).getByRole("button", { name: "Projects" }));
  expect(opens().at(-1)?.args.target).toEqual({ kind: "folder", folder: 2 });
});

test("a segment's arrow lists its subfolders, the one on the path bold", async () => {
  render(<Bar />);
  fireEvent.pointerDown(screen.getByRole("button", { name: "Folders in Projects" }), {
    button: 0,
    ctrlKey: false,
  });
  const menu = await screen.findByRole("menu", { name: "Folders in Projects" });
  const bmw = await within(menu).findByRole("menuitem", { name: "BMW" });
  expect(within(bmw).getByText("BMW")).toHaveClass("font-semibold");
  expect(core.calls.find((call) => call.cmd === "folder_subfolders")?.args).toEqual({
    folder: 2,
    showHidden: false,
  });

  fireEvent.click(within(menu).getByRole("menuitem", { name: "Audi" }));
  expect(opens().at(-1)?.args.target).toEqual({ kind: "folder", folder: 5 });
});

test("a click on the bar or Ctrl+L types an address; Esc goes back to the segments", () => {
  render(<Bar />);
  fireEvent.click(screen.getAllByText("Drawings")[0] as HTMLElement);
  expect(field()).toHaveValue(String.raw`D:\Projects\BMW\Drawings`);
  expect(field()).toHaveFocus();
  fireEvent.keyDown(field(), { key: "Escape" });
  expect(screen.queryByRole("combobox")).toBeNull();

  fireEvent.keyDown(window, { code: "KeyL", key: "l", ctrlKey: true });
  expect(field()).toHaveFocus();
});

test("typing suggests folders; the arrows pick one and Enter opens it", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  render(<Bar />);
  act(() => useAddressEdit.setState({ pane: "left" }));

  fireEvent.change(field(), { target: { value: String.raw`D:\Projects\B` } });
  await act(async () => {
    vi.advanceTimersByTime(150);
  });
  vi.useRealTimers();
  const list = await screen.findByRole("listbox", { name: "Suggestions" });
  expect(
    within(list)
      .getAllByRole("option")
      .map((o) => o.textContent),
  ).toEqual([String.raw`D:\Projects\BMW`, String.raw`D:\Projects\Bentley`]);
  expect(core.calls.find((call) => call.cmd === "autocomplete_path")?.args).toMatchObject({
    from: 4,
    showHidden: false,
  });

  fireEvent.keyDown(field(), { key: "ArrowDown" });
  fireEvent.keyDown(field(), { key: "ArrowDown" });
  expect(field()).toHaveValue(String.raw`D:\Projects\Bentley`);
  fireEvent.keyDown(field(), { key: "Enter" });
  await waitFor(() => expect(opens().at(-1)?.args.target).toEqual({ kind: "folder", folder: 9 }));
  expect(core.calls.find((call) => call.cmd === "resolve_path")?.args).toEqual({
    text: String.raw`D:\Projects\Bentley`,
    from: 4,
  });
  expect(screen.queryByRole("combobox")).toBeNull();
});

test("a file's path opens its folder with the file selected", async () => {
  resolved = () => ({ folder: 9, select: "ნახაზი.dwg" });
  render(<Bar />);
  act(() => useAddressEdit.setState({ pane: "left" }));
  fireEvent.change(field(), { target: { value: String.raw`"C:\Other\ნახაზი.dwg"` } });
  fireEvent.keyDown(field(), { key: "Enter" });

  await waitFor(() => expect(tab.current()?.folder).toBe(9));
  await waitFor(() => expect(tab.folder.store.getState().focused).toBe(1));
  expect(tab.folder.store.getState().selected).toEqual(new Set([1]));
});

test("an address that leads nowhere says so at the field and stays", async () => {
  resolved = () => Promise.reject({ code: "NotFound", path: String.raw`D:\Nope` });
  render(<Bar />);
  act(() => useAddressEdit.setState({ pane: "left" }));
  const before = opens().length;
  fireEvent.change(field(), { target: { value: String.raw`D:\Nope` } });
  fireEvent.keyDown(field(), { key: "Enter" });

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Jerry can't find “D:\\Nope”. Check the spelling and try again.",
  );
  expect(field()).toHaveAttribute("aria-invalid", "true");
  expect(opens()).toHaveLength(before);
  expect(tab.current()?.folder).toBe(4);
});

test("a narrow bar keeps the root and the last segments, folding the middle", () => {
  // A bar `room` px wide, its segments `widths` px.
  const bar = (room: number, widths: number[]) =>
    fold(
      { clientWidth: room } as HTMLElement,
      { children: widths.map((offsetWidth) => ({ offsetWidth })) } as unknown as HTMLElement,
    );
  expect(bar(500, [30, 80, 80, 80])).toBeUndefined();
  // Not laid out yet: nothing folds.
  expect(bar(0, [30, 80, 80, 80])).toBeUndefined();
  expect(bar(250, [30, 80, 80, 80])).toEqual({ lo: 1, hi: 2 });
  expect(bar(200, [30, 80, 80, 80])).toEqual({ lo: 1, hi: 3 });
  // Too narrow for the root too.
  expect(bar(110, [30, 80, 80, 80])).toEqual({ lo: 0, hi: 3 });
});
