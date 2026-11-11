import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test } from "vitest";
import { useCommandKeys } from "../../commands/dispatch";
import type { SavedSearch } from "../../ipc";
import { batchMessage, doneMessage, fakeListings, orderMessage } from "../../ipc/listing.testing";
import { searchBatch, searchDone, searchOrder } from "../../ipc/search.testing";
import { useOverlay } from "../../stores/overlay";
import { useSettings } from "../../stores/settings";
import { defaultSettings } from "../../stores/settings.testing";
import { activeTab, resetTabs, useTabs } from "../../stores/tabs";
import { paneFolder, paneTabs } from "../../stores/tabs.testing";
import { EMPTY_SEARCH } from "./query";
import { SearchDialog } from "./SearchDialog";
import { useSearchHistory } from "./search";

const WORK = String.raw`D:\Projects`;
const recent: SavedSearch[] = [
  { ...EMPTY_SEARCH, name: "ნახაზი", items: "files" },
  { ...EMPTY_SEARCH, name: "*.pdf", wholeDrive: true },
];

beforeEach(() => {
  paneTabs();
});
afterEach(() => {
  resetTabs();
  useOverlay.setState({ overlay: undefined });
  useSettings.setState({ settings: undefined });
  useSearchHistory.setState({ searches: undefined });
});

function Window() {
  useCommandKeys();
  return <SearchDialog />;
}

/** The left pane shows D:\Projects; the core keeps two recent searches. */
async function opened() {
  useSettings.setState({ settings: defaultSettings });
  const core = fakeListings({
    path: WORK,
    messages: () => [
      batchMessage(0, [{ name: "BMW", attributes: 0x10 }]),
      orderMessage([0]),
      doneMessage(1, 1, 1),
    ],
    opened: () => ({
      crumbs: [
        { name: "D:", folder: 1 },
        { name: "Projects", folder: 101 },
      ],
    }),
    searches: () => [
      searchBatch(0, [{ name: "plan.dwg" }], [0], 0, [String.raw`D:\Projects\BMW`]),
      searchOrder([0]),
      searchDone(1, 2),
    ],
    other: (cmd) => (cmd === "search_history_list" ? recent : null),
  });
  await activeTab("left")?.open({ kind: "path", path: WORK });
  await waitFor(() => expect(paneFolder("left").store.getState().status).toBe("ready"));
  render(<Window />);
  return core;
}

const ctrlShiftF = () =>
  fireEvent.keyDown(document.body, { code: "KeyF", key: "F", ctrlKey: true, shiftKey: true });

test("Ctrl+Shift+F searches the folder: the results open in a tab of their own", async () => {
  const core = await opened();

  ctrlShiftF();
  const input = await screen.findByRole("textbox", { name: "Name" });
  const dialog = screen.getByRole("dialog", { name: "Search" });
  expect(within(dialog).getByRole("button", { name: "Projects and subfolders" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(within(dialog).getByRole("button", { name: "Whole drive D:" })).toBeInTheDocument();
  // The recent searches drop below the field.
  expect(await within(dialog).findByRole("option", { name: /ნახაზი/ })).toBeInTheDocument();

  fireEvent.change(input, { target: { value: "*.dwg" } });
  // Typing narrows the recent searches: none has this name.
  expect(within(dialog).queryByRole("listbox")).toBeNull();
  fireEvent.keyDown(input, { key: "Enter", code: "Enter" });

  expect(useOverlay.getState().overlay).toBeUndefined();
  const { tabs, active } = useTabs.getState().left;
  expect(tabs).toHaveLength(2);
  expect(active).toBe(tabs[1]?.id);
  await waitFor(() => expect(paneFolder("left").store.getState().search?.phase).toBe("done"));
  const start = core.calls.find((call) => call.cmd === "search_start")?.args;
  expect(start).toMatchObject({
    within: { kind: "folder", folder: 101 },
    query: { name: "*.dwg" },
  });
  expect(core.calls.find((call) => call.cmd === "search_history_add")?.args).toEqual({
    search: { ...EMPTY_SEARCH, name: "*.dwg" },
  });
  expect(activeTab("left")?.current()).toMatchObject({ name: "*.dwg", path: WORK });
});

test("↓ and Enter run a recent search, filters and scope too; Delete forgets one", async () => {
  const core = await opened();
  ctrlShiftF();
  const input = await screen.findByRole("textbox", { name: "Name" });
  await screen.findByRole("option", { name: /ნახაზი/ });

  fireEvent.keyDown(input, { key: "ArrowDown" });
  fireEvent.keyDown(input, { key: "ArrowDown" });
  expect(screen.getByRole("option", { name: /\*\.pdf/ })).toHaveAttribute("aria-selected", "true");
  fireEvent.keyDown(input, { key: "Delete" });
  expect(screen.queryByRole("option", { name: /\*\.pdf/ })).toBeNull();
  expect(core.calls.find((call) => call.cmd === "search_history_remove")?.args).toEqual({
    search: recent[1],
  });

  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() => expect(paneFolder("left").store.getState().search).toBeDefined());
  const start = core.calls.find((call) => call.cmd === "search_start")?.args;
  expect(start).toMatchObject({ query: { name: "ნახაზი", items: "files" } });
});

test("a custom size that doesn't read says so and doesn't search", async () => {
  const core = await opened();
  ctrlShiftF();
  const input = await screen.findByRole("textbox", { name: "Name" });

  // The Size menu's Custom size… opens two fields.
  act(() => {
    fireEvent.pointerDown(screen.getByRole("button", { name: "Size: Any size" }), {
      button: 0,
      pointerType: "mouse",
    });
  });
  fireEvent.click(await screen.findByRole("menuitem", { name: "Custom size…" }));
  fireEvent.change(await screen.findByRole("textbox", { name: "At least" }), {
    target: { value: "lots" },
  });
  expect(screen.getByRole("alert")).toHaveTextContent("Sizes look like 500 KB, 10 MB or 2 GB.");
  fireEvent.keyDown(input, { key: "Enter" });
  expect(core.calls.some((call) => call.cmd === "search_start")).toBe(false);

  fireEvent.change(screen.getByRole("textbox", { name: "At least" }), {
    target: { value: "10 MB" },
  });
  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() => expect(core.calls.some((call) => call.cmd === "search_start")).toBe(true));
  const start = core.calls.find((call) => call.cmd === "search_start")?.args;
  expect(start).toMatchObject({ query: { size: { min: 10 * 1024 * 1024, max: null } } });
});

test("from a results tab, the window edits its search and runs it there", async () => {
  const core = await opened();
  ctrlShiftF();
  fireEvent.change(await screen.findByRole("textbox", { name: "Name" }), {
    target: { value: "*.dwg" },
  });
  fireEvent.keyDown(screen.getByRole("textbox", { name: "Name" }), { key: "Enter" });
  await waitFor(() => expect(paneFolder("left").store.getState().search?.phase).toBe("done"));

  ctrlShiftF();
  const input = await screen.findByRole("textbox", { name: "Name" });
  expect(input).toHaveValue("*.dwg");
  fireEvent.click(screen.getByRole("button", { name: "Whole drive D:" }));
  fireEvent.keyDown(input, { key: "Enter" });

  await waitFor(() =>
    expect(core.calls.filter((call) => call.cmd === "search_start")).toHaveLength(2),
  );
  expect(useTabs.getState().left.tabs).toHaveLength(2);
  const again = core.calls.filter((call) => call.cmd === "search_start")[1]?.args;
  // The folder the first search started from, now with its drive.
  expect(again).toMatchObject({ within: { kind: "drive", folder: 101 } });
});
