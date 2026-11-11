import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, expect, test } from "vitest";
import { useCommandKeys } from "../../commands/dispatch";
import { fakeListings } from "../../ipc/listing.testing";
import { searchBatch, searchDone, searchOrder } from "../../ipc/search.testing";
import type { SearchSpec } from "../../stores/folder";
import { useSettings } from "../../stores/settings";
import { defaultSettings } from "../../stores/settings.testing";
import { activeTab, resetTabs, useTabs } from "../../stores/tabs";
import { paneTabs } from "../../stores/tabs.testing";
import { FileList } from "../file-list/FileList";
import { snapshot } from "../session/save";
import { EMPTY_SEARCH } from "./query";
import { SearchBar } from "./SearchBar";

/** A results tab in the window: its list, its bar, the commands on it. */

const ROOT = String.raw`D:\Projects`;
const spec: SearchSpec = {
  search: { ...EMPTY_SEARCH, name: "*.dwg", modified: { kind: "thisWeek" } },
  folder: 5,
  name: "Projects",
  drive: "D:",
};

// jsdom lays nothing out: give the list a viewport, so the virtualizer renders rows.
const clientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
beforeAll(() => {
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get: () => 600,
  });
});
afterAll(() => {
  if (clientHeight) Object.defineProperty(HTMLElement.prototype, "clientHeight", clientHeight);
});
afterEach(() => {
  resetTabs();
  useSettings.setState({ settings: undefined });
});

function Window() {
  useCommandKeys();
  const tab = useTabs((state) => state.left.active);
  const shown = activeTab("left");
  return shown ? (
    <>
      <SearchBar folder={shown.folder} />
      <FileList key={tab} tab={shown} pane="left" />
    </>
  ) : null;
}

/** The left pane's tab shows the results of `spec`: two drawings in two folders. */
async function results() {
  useSettings.setState({ settings: defaultSettings });
  const core = fakeListings({
    path: ROOT,
    searches: () => [
      searchBatch(
        0,
        [
          { name: "plan_rev2.dwg", size: 100 },
          { name: "ნახაზი 1.dwg", size: 200 },
        ],
        [0, 1],
        0,
        [String.raw`D:\Projects\BMW\Drawings`, String.raw`D:\Projects\ბათუმი`],
      ),
      searchOrder([0, 1]),
      searchDone(2, 3),
    ],
  });
  paneTabs();
  const tab = activeTab("left");
  if (!tab) throw new Error("no tab");
  render(<Window />);
  await act(() => tab.search(spec));
  await waitFor(() => expect(tab.folder.store.getState().status).toBe("ready"));
  return { core, tab };
}

test("results show their Location from the folder searched down, the full path as a tip", async () => {
  await results();

  const grid = screen.getByRole("grid", { name: "Left pane files" });
  const header = within(grid)
    .getAllByRole("columnheader")
    .map((cell) => cell.textContent);
  expect(header.slice(0, 2)).toEqual(["Name", "Location"]);
  const [first] = within(grid).getAllByRole("row").slice(1);
  const location = within(first as HTMLElement).getAllByRole("gridcell")[1];
  expect(location).toHaveTextContent(String.raw`Projects\BMW\Drawings`);
  expect(location).toHaveAttribute("title", String.raw`D:\Projects\BMW\Drawings`);
  // The bar where the address would be says what was searched, where, and how it went.
  expect(screen.getByText("*.dwg")).toBeInTheDocument();
  expect(screen.getByText("This week · in Projects")).toBeInTheDocument();
  expect(screen.getByText(/^Found 2 in/)).toBeInTheDocument();
});

test("Ctrl+Shift+E opens each chosen result's folder in a new tab, with it selected", async () => {
  const { core, tab } = await results();
  tab.folder.setSelection({ selected: new Set([0, 1]), anchor: 0, focused: 1 });
  const list = screen.getByRole("grid", { name: "Left pane files" });

  fireEvent.keyDown(list, { code: "KeyE", key: "E", ctrlKey: true, shiftKey: true });

  const opens = core.calls.filter((call) => call.cmd === "listing_open");
  expect(opens.map((call) => call.args.target)).toEqual([
    { kind: "containing", listing: 1, entry: 0 },
    { kind: "containing", listing: 1, entry: 1 },
  ]);
  const { tabs, active } = useTabs.getState().left;
  expect(tabs).toHaveLength(3);
  // The first folder's tab shows, right after the results.
  expect(active).toBe(tabs[1]?.id);
});

test("a results tab is saved as the folder it searched, without the Location column", async () => {
  const { tab } = await results();
  tab.folder.sortBy("location");

  const [saved] = snapshot().left.tabs;

  expect(saved?.history).toEqual([{ folder: 5, focus: null }]);
  expect(saved?.view.columns.order).not.toContain("location");
  expect(saved?.view.sort.column).toBe("name");
});
