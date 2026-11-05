import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, beforeEach, expect, test } from "vitest";
import { useCommandKeys } from "../../commands/dispatch";
import {
  batchMessage,
  doneMessage,
  type FakeListings,
  fakeListings,
  orderMessage,
  type TestEntry,
} from "../../ipc/listing.testing";
import { useSettings } from "../../stores/settings";
import { defaultSettings } from "../../stores/settings.testing";
import { activeTab, addTab, resetTabs, type Tab, useTabs } from "../../stores/tabs";
import { paneTabs } from "../../stores/tabs.testing";
import { useWindow } from "../../stores/window";
import { PaneList } from "../file-list/PaneList";
import { OpenManyDialog } from "./OpenManyDialog";
import { MANY_ITEMS } from "./open";

// jsdom lays nothing out: give the list a viewport, so the virtualizer renders rows.
const clientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
beforeAll(() => {
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get: () => 600,
  });
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
});
afterAll(() => {
  if (clientHeight) Object.defineProperty(HTMLElement.prototype, "clientHeight", clientHeight);
});
let tabs: ReturnType<typeof paneTabs>;
beforeEach(() => {
  tabs = paneTabs();
  useSettings.setState({ settings: defaultSettings });
});
afterEach(() => {
  resetTabs();
  useSettings.setState({ settings: undefined });
  useWindow.setState({ activePane: "left" });
});

const PROJECT: TestEntry[] = [
  { name: "ნახაზები", attributes: 0x10 },
  { name: "Plot", attributes: 0x10 },
  { name: "plan.dwg" },
  { name: "notes.txt" },
];

function Window() {
  useCommandKeys();
  return (
    <>
      <PaneList pane="left" />
      <OpenManyDialog />
    </>
  );
}

/** The left tab shows `entries` in arrival order. */
async function shown(entries: TestEntry[] = PROJECT): Promise<{ core: FakeListings; tab: Tab }> {
  const core = fakeListings({
    messages: () => [
      batchMessage(0, entries),
      orderMessage(entries.map((_, i) => i)),
      doneMessage(entries.length, 1, 1),
    ],
  });
  render(<Window />);
  const tab = tabs.left;
  await act(() => tab.open({ kind: "path", path: String.raw`C:\Project` }));
  await waitFor(() => expect(tab.folder.store.getState().status).toBe("ready"));
  return { core, tab };
}

const grid = () => screen.getByRole("grid", { name: "Left pane files" });
const press = (code: string, init: { ctrlKey?: boolean; shiftKey?: boolean } = {}) =>
  fireEvent.keyDown(grid(), { code, key: code, ...init });
const calls = (core: FakeListings, cmd: string) => core.calls.filter((call) => call.cmd === cmd);
const leftIds = () => useTabs.getState().left.tabs.map((tab) => tab.id);

test("Enter opens the focused folder in the tab, and a file in its program", async () => {
  const { core, tab } = await shown();

  tab.folder.setSelection({ selected: new Set([2]), anchor: 2, focused: 2 });
  press("Enter");
  expect(calls(core, "open_entries").at(-1)?.args).toEqual({ listing: 1, entries: [2] });
  expect(calls(core, "listing_open")).toHaveLength(1);

  // Focused, not selected: the focused row opens.
  tab.folder.setSelection({ selected: new Set(), anchor: 0, focused: 0 });
  press("Enter");
  expect(calls(core, "listing_open").at(-1)?.args.target).toEqual({
    kind: "entry",
    listing: 1,
    entry: 0,
  });
  await waitFor(() => expect(tab.store.getState().history).toHaveLength(2));
});

test("Enter on several items opens folders in background tabs and files in their programs", async () => {
  const { core, tab } = await shown();
  tab.folder.setSelection({ selected: new Set([3, 1, 2, 0]), anchor: 0, focused: 3 });

  press("Enter");
  // Display order: the folders, then the files.
  expect(calls(core, "open_entries").at(-1)?.args).toEqual({ listing: 1, entries: [2, 3] });
  expect(leftIds()).toHaveLength(3);
  expect(useTabs.getState().left.active).toBe(tab.id);
  const opened = calls(core, "listing_open").slice(1);
  expect(opened.map((call) => call.args.target)).toEqual([
    { kind: "entry", listing: 1, entry: 0 },
    { kind: "entry", listing: 1, entry: 1 },
  ]);
});

test("Ctrl+Enter and a middle click open a folder in a new tab in the background", async () => {
  const { core, tab } = await shown();
  tab.folder.setSelection({ selected: new Set([1]), anchor: 1, focused: 1 });

  press("Enter", { ctrlKey: true });
  expect(leftIds()).toHaveLength(2);
  expect(useTabs.getState().left.active).toBe(tab.id);

  fireEvent(screen.getByText("ნახაზები"), new MouseEvent("auxclick", { bubbles: true, button: 1 }));
  // A middle click on a file opens nothing.
  fireEvent(screen.getByText("plan.dwg"), new MouseEvent("auxclick", { bubbles: true, button: 1 }));
  expect(leftIds()).toHaveLength(3);
  expect(calls(core, "open_entries")).toHaveLength(0);
});

test(`more than ${MANY_ITEMS} items at once ask first; Cancel opens nothing`, async () => {
  const many = Array.from({ length: MANY_ITEMS + 1 }, (_, i) => ({ name: `sheet ${i}.pdf` }));
  const { core, tab } = await shown(many);
  press("KeyA", { ctrlKey: true });
  expect(tab.folder.store.getState().selected.size).toBe(many.length);

  press("Enter");
  const dialog = await screen.findByRole("dialog", { name: "Open many items" });
  expect(dialog).toHaveTextContent(`Open ${many.length} items?`);
  fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(calls(core, "open_entries")).toHaveLength(0);

  press("Enter");
  fireEvent.click(await screen.findByRole("button", { name: "Open all" }));
  expect(calls(core, "open_entries").at(-1)?.args).toEqual({
    listing: 1,
    entries: many.map((_, i) => i),
  });
});

test("a tab shown again gets its scroll position back; the keyboard follows Ctrl+Tab", async () => {
  await shown();
  addTab("left");
  const first = activeTab("left");
  grid().focus();
  act(() => {
    grid().scrollTop = 48;
    fireEvent.scroll(grid());
  });
  fakeListings();

  press("Tab", { ctrlKey: true });
  // The other tab has no folder; its list took the keyboard.
  await waitFor(() => expect(activeTab("left")).not.toBe(first));
  expect(document.activeElement).toBe(grid());
  expect(grid().scrollTop).toBe(0);

  press("Tab", { ctrlKey: true });
  await waitFor(() => expect(activeTab("left")).toBe(first));
  expect(grid().scrollTop).toBe(48);
  expect(document.activeElement).toBe(grid());
});
