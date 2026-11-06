import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, expect, test } from "vitest";
import { useCommandKeys } from "../../commands/dispatch";
import type { JerryError } from "../../ipc";
import {
  batchMessage,
  type Call,
  doneMessage,
  fakeListings,
  orderMessage,
} from "../../ipc/listing.testing";
import { useSettings } from "../../stores/settings";
import { defaultSettings } from "../../stores/settings.testing";
import { resetTabs } from "../../stores/tabs";
import { paneFolder, paneTabs } from "../../stores/tabs.testing";
import { useWindow } from "../../stores/window";
import { PaneList } from "../file-list/PaneList";
import { ClipboardDialog } from "./ClipboardDialog";
import { syncClipboard, useClipboard } from "./clipboard";

beforeAll(() => {
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get: () => 600,
  });
});
beforeEach(() => {
  paneTabs();
});
afterEach(() => {
  resetTabs();
  useSettings.setState({ settings: undefined });
  useWindow.setState({ activePane: "left" });
  useClipboard.setState({ cut: false, version: 0, failed: undefined });
});

function Window() {
  useCommandKeys();
  return (
    <>
      <PaneList pane="left" />
      <ClipboardDialog />
    </>
  );
}

/**
 * The left pane shows ნახაზი.dwg (0), b.txt (2) and c.txt (3); desktop.ini (1) is hidden. The fake
 * core answers the clipboard commands with `answer`, null by default.
 */
async function shown(answer: (cmd: string, args: Record<string, unknown>) => unknown = () => null) {
  useSettings.setState({ settings: defaultSettings });
  const { calls } = fakeListings({
    messages: () => [
      batchMessage(0, [
        { name: "ნახაზი.dwg", size: 10 },
        { name: "desktop.ini", attributes: 0x2 | 0x4 },
        { name: "b.txt" },
        { name: "c.txt" },
      ]),
      orderMessage([0, 1, 2, 3]),
      doneMessage(4, 1, 1),
    ],
    other: answer,
  });
  render(<Window />);
  const folder = paneFolder("left");
  await act(() => folder.open({ kind: "path", path: String.raw`C:\Work` }));
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  return { folder, calls };
}

const press = (code: string, init: { ctrlKey?: boolean; shiftKey?: boolean; altKey?: boolean }) =>
  fireEvent.keyDown(screen.getByRole("grid", { name: "Left pane files" }), { code, ...init });

const sent = (calls: Call[], cmd: string) =>
  calls.filter((call) => call.cmd === cmd).map((call) => call.args);

const row = (name: string) => {
  const cell = screen.getByText(name);
  const found = cell.closest('[role="row"]');
  if (!(found instanceof HTMLElement)) throw new Error(`no row for ${name}`);
  return found;
};

test("Ctrl+C and Ctrl+X put the shown selected rows on the clipboard, in display order", async () => {
  const { folder, calls } = await shown();
  folder.setSelection({ selected: new Set([3, 1, 0]), anchor: 0, focused: 3 });

  press("KeyC", { ctrlKey: true });
  press("KeyX", { ctrlKey: true });

  await waitFor(() =>
    expect(sent(calls, "clipboard_copy")).toEqual([
      { listing: 1, entries: [0, 3], cut: false },
      { listing: 1, entries: [0, 3], cut: true },
    ]),
  );
});

test("without a selection they act on the focused row, and with nothing chosen not at all", async () => {
  const { folder, calls } = await shown();
  folder.setSelection({ selected: new Set(), anchor: 2, focused: 2 });
  press("KeyC", { ctrlKey: true });
  await waitFor(() =>
    expect(sent(calls, "clipboard_copy")).toEqual([{ listing: 1, entries: [2], cut: false }]),
  );

  folder.setSelection({ selected: new Set(), anchor: undefined, focused: undefined });
  press("KeyX", { ctrlKey: true });
  press("KeyC", { ctrlKey: true, shiftKey: true });
  await act(async () => {});
  expect(sent(calls, "clipboard_copy")).toHaveLength(1);
  expect(sent(calls, "clipboard_copy_text")).toEqual([]);
});

test("Ctrl+V pastes into the pane's folder, whatever is selected", async () => {
  const { folder, calls } = await shown();
  folder.setSelection({ selected: new Set([2]), anchor: 2, focused: 2 });

  press("KeyV", { ctrlKey: true });

  await waitFor(() => expect(sent(calls, "clipboard_paste")).toEqual([{ listing: 1, into: null }]));
});

test("Ctrl+Shift+C copies the paths and Ctrl+Alt+C the names", async () => {
  const { folder, calls } = await shown();
  folder.setSelection({ selected: new Set([0, 2]), anchor: 0, focused: 2 });

  press("KeyC", { ctrlKey: true, shiftKey: true });
  press("KeyC", { ctrlKey: true, altKey: true });

  await waitFor(() =>
    expect(sent(calls, "clipboard_copy_text")).toEqual([
      { listing: 1, entries: [0, 2], text: "path" },
      { listing: 1, entries: [0, 2], text: "name" },
    ]),
  );
});

test("a failed copy says why; Retry runs it again", async () => {
  const gone: JerryError = { code: "NotFound", path: String.raw`C:\Work\b.txt` } as JerryError;
  let fail = true;
  const { folder, calls } = await shown((cmd) => {
    if (cmd === "clipboard_copy" && fail) {
      fail = false;
      return Promise.reject(gone);
    }
    return null;
  });
  folder.setSelection({ selected: new Set([2]), anchor: 2, focused: 2 });

  press("KeyC", { ctrlKey: true });

  const dialog = await screen.findByRole("dialog");
  expect(within(dialog).getByText("Couldn't copy")).toBeTruthy();
  fireEvent.click(within(dialog).getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(sent(calls, "clipboard_copy")).toHaveLength(2));
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("Jerry's cut shows dimmed until the clipboard holds something else", async () => {
  await shown((cmd) => (cmd === "clipboard_cut_entries" ? [2] : null));
  expect(row("b.txt").innerHTML).not.toContain("opacity-50");

  act(() => useClipboard.setState({ cut: true, version: 1 }));
  await waitFor(() => expect(row("b.txt").innerHTML).toContain("opacity-50"));
  expect(row("c.txt").innerHTML).not.toContain("opacity-50");

  act(() => useClipboard.setState({ cut: false, version: 2 }));
  await waitFor(() => expect(row("b.txt").innerHTML).not.toContain("opacity-50"));
});

test("clipboard:changed reaches the store", async () => {
  mockIPC(() => null, { shouldMockEvents: true });
  const stop = syncClipboard();
  try {
    await waitFor(async () => {
      await emit("clipboard:changed", { cut: true });
      expect(useClipboard.getState().cut).toBe(true);
    });
    const { version } = useClipboard.getState();
    await emit("clipboard:changed", { cut: false });
    expect(useClipboard.getState()).toMatchObject({ cut: false, version: version + 1 });
  } finally {
    stop();
  }
});
