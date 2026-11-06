import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { resetSidebar, useSidebar } from "../features/sidebar/store";
import type { Drive, ListingTarget, Space } from "../ipc";
import {
  batchMessage,
  doneMessage,
  errorMessage,
  fakeListings,
  orderMessage,
  type TestEntry,
} from "../ipc/listing.testing";
import { useSettings } from "../stores/settings";
import { defaultSettings } from "../stores/settings.testing";
import { addTab, resetTabs, type Tab } from "../stores/tabs";
import { useWindow } from "../stores/window";
import { StatusBar } from "./StatusBar";
import { SPACE_EVERY } from "./status";

const GB = 1024 ** 3;

const entries: TestEntry[] = [
  { name: "Drawings", attributes: 0x10 },
  { name: "plan.dwg", size: 1536 },
  { name: "spec.pdf", size: 2048 },
  { name: "desktop.ini", size: 100, attributes: 0x22 },
];

function drive(letter: string, more: Partial<Drive> = {}): Drive {
  return {
    letter,
    kind: "fixed",
    label: null,
    remote: null,
    connected: true,
    ejectable: false,
    space: { total: 1862 * GB, free: 612 * GB },
    ...more,
  };
}

beforeEach(() => {
  useSettings.setState({ settings: defaultSettings });
});

afterEach(() => {
  vi.useRealTimers();
  resetTabs();
  resetSidebar();
  useWindow.setState({ activePane: "left" });
  useSettings.setState({ settings: undefined });
});

/** A fake core whose folders have `entries`, and whose `folder_space` answers `space()`. */
function core(paths: Record<string, string>, space: () => Space | null = () => null) {
  return fakeListings({
    messages: () => [batchMessage(0, entries), orderMessage([0, 1, 2, 3]), doneMessage(4, 1, 1)],
    opened: (target: ListingTarget) =>
      target.kind === "path" ? { path: paths[target.path] ?? target.path } : {},
    other: (cmd) => (cmd === "folder_space" ? space() : null),
  });
}

async function openTab(pane: "left" | "right", path: string): Promise<Tab> {
  const tab = addTab(pane);
  await tab.open({ kind: "path", path });
  return tab;
}

const bar = () => screen.getByRole("contentinfo");

test("counts what the list shows, and the selection with its files' size", async () => {
  core({});
  const tab = await openTab("left", String.raw`C:\Work`);
  render(<StatusBar density="comfortable" dual />);

  // The hidden desktop.ini is left out, as in the list.
  await waitFor(() => expect(bar()).toHaveTextContent("3 items"));
  expect(bar()).not.toHaveTextContent("selected");

  const select = (ids: number[]) =>
    act(() => tab.folder.setSelection({ selected: new Set(ids), anchor: ids[0], focused: ids[0] }));
  select([1, 2]);
  expect(screen.getByText("2 selected · 3.5 KB")).not.toHaveAttribute("title");
  select([0, 1]);
  expect(screen.getByText("2 selected · 1.5 KB")).toHaveAttribute(
    "title",
    "Size of the selected files. Folder sizes aren't counted.",
  );
  select([0]);
  expect(screen.getByText("1 selected")).toBeInTheDocument();

  act(() => tab.folder.setFilter("*.pdf"));
  expect(bar()).toHaveTextContent("1 of 3 items");
  // The filter took the folder out of the selection.
  expect(bar()).not.toHaveTextContent("selected");

  act(() => useSettings.setState({ settings: { ...defaultSettings, showHidden: true } }));
  act(() => tab.folder.setFilter(""));
  expect(bar()).toHaveTextContent("4 items");
});

test("shows the active pane's drive and its free space from the drive list", async () => {
  core({ "D:\\": "D:\\" }, () => {
    throw new Error("a local drive's space comes from the list");
  });
  useSidebar.setState({
    drives: { generation: 1, drives: [drive("C"), drive("D", { space: null })] },
  });
  await openTab("left", String.raw`C:\Work`);
  await openTab("right", "D:\\");
  render(<StatusBar density="comfortable" dual />);

  const free = await screen.findByText("C: 612 GB free of 1.8 TB");
  expect(free).toHaveAttribute("title", "C:\\");

  // D: hasn't been read yet: nothing until the list has its space.
  act(() => useWindow.setState({ activePane: "right" }));
  expect(bar()).not.toHaveTextContent("free");
  act(() =>
    useSidebar.setState({
      drives: {
        generation: 2,
        drives: [drive("C"), drive("D", { space: { total: 2 * GB, free: GB } })],
      },
    }),
  );
  expect(bar()).toHaveTextContent("D: 1 GB free of 2 GB");
});

test("a network folder's free space is read on demand, and again while it shows", async () => {
  let answer: Space | null = { total: 4096 * GB, free: 1229 * GB };
  const fake = core({ share: String.raw`\\INSTA-NAS01\Projects\BMW` }, () => answer);
  const asked = () => fake.calls.filter((call) => call.cmd === "folder_space");
  await openTab("left", "share");
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  const { unmount } = render(<StatusBar density="comfortable" dual />);

  await vi.waitFor(() => expect(bar()).toHaveTextContent("Projects: 1.2 TB free of 4 TB"));
  expect(screen.getByText(/^Projects:/)).toHaveAttribute(
    "title",
    String.raw`\\INSTA-NAS01\Projects`,
  );
  expect(asked()).toEqual([{ cmd: "folder_space", args: { listing: 1 } }]);

  // The server doesn't answer in time: nothing shows.
  answer = null;
  await act(() => vi.advanceTimersByTimeAsync(SPACE_EVERY));
  expect(asked()).toHaveLength(2);
  expect(bar()).not.toHaveTextContent("free");

  unmount();
  await act(() => vi.advanceTimersByTimeAsync(3 * SPACE_EVERY));
  expect(asked()).toHaveLength(2);
});

test("a folder that isn't open or failed shows no counts and no free space", async () => {
  core({});
  useSidebar.setState({ drives: { generation: 1, drives: [drive("C")] } });
  const tab = addTab("left");
  render(<StatusBar density="comfortable" dual />);
  expect(bar()).not.toHaveTextContent("item");

  fakeListings({
    messages: () => [errorMessage(String.raw`{"code":"AccessDenied","path":"C:\\დახურული"}`)],
  });
  await act(() => tab.open({ kind: "path", path: String.raw`C:\დახურული` }));
  await waitFor(() => expect(tab.folder.store.getState().status).toBe("failed"));
  expect(bar()).not.toHaveTextContent("item");
  expect(bar()).not.toHaveTextContent("free");
});
