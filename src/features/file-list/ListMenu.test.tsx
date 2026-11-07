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
import { DeleteDialog } from "../delete/DeleteDialog";
import { useDeleteDialog } from "../delete/flow";
import { ShellDialog } from "../navigation/ShellDialog";
import { useShellFailure } from "../navigation/shell";
import { cancelRename } from "../rename/rename";
import { PaneList } from "./PaneList";

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
beforeEach(() => {
  paneTabs();
});
afterEach(() => {
  cancelRename();
  resetTabs();
  useSettings.setState({ settings: undefined });
  useWindow.setState({ activePane: "left" });
  useDeleteDialog.setState({ dialog: undefined });
  useShellFailure.setState({ failed: undefined });
});

function Window() {
  useCommandKeys();
  return (
    <>
      <PaneList pane="left" />
      <DeleteDialog />
      <ShellDialog />
    </>
  );
}

/**
 * The left pane shows Plot (0, a folder), ნახაზი.dwg (1) and b.txt (2). The fake core says the
 * clipboard holds files when `files`, and answers other commands with `answer`.
 */
async function shown({
  files = true,
  answer = () => null,
}: {
  files?: boolean;
  answer?: (cmd: string, args: Record<string, unknown>) => unknown;
} = {}) {
  useSettings.setState({ settings: defaultSettings });
  const { calls } = fakeListings({
    messages: () => [
      batchMessage(0, [
        { name: "Plot", attributes: 0x10 },
        { name: "ნახაზი.dwg", size: 10 },
        { name: "b.txt" },
      ]),
      orderMessage([0, 1, 2]),
      doneMessage(3, 1, 1),
    ],
    other: (cmd, args) => {
      if (cmd === "clipboard_has_files") return files;
      if (cmd === "delete_check") return { system: false };
      // The permanent delete's question counts what goes.
      if (cmd === "delete_measure") return { files: 1, folders: 0, bytes: 10 };
      return answer(cmd, args);
    },
  });
  render(<Window />);
  const folder = paneFolder("left");
  await act(() => folder.open({ kind: "path", path: String.raw`C:\Work` }));
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  return { folder, calls };
}

const grid = () => screen.getByRole("grid", { name: "Left pane files" });

const sent = (calls: Call[], cmd: string) =>
  calls.filter((call) => call.cmd === cmd).map((call) => call.args);

const row = (name: string) => {
  const found = within(grid()).getByText(name).closest('[role="row"]');
  if (!(found instanceof HTMLElement)) throw new Error(`no row for ${name}`);
  return found;
};

/** A right click on `target`, as the mouse makes it: the press selects, then the menu opens. */
async function rightClick(target: Element) {
  fireEvent.pointerDown(target, { button: 2, pointerId: 1 });
  fireEvent.pointerUp(target, { button: 2, pointerId: 1 });
  fireEvent.contextMenu(target);
  return screen.findByRole("menu");
}

/** The menu's items: title and shortcut as shown, "(off)" when grayed out, "—" for separators. */
function items(menu: HTMLElement): string[] {
  return Array.from(menu.querySelectorAll('[role^="menuitem"], [role="separator"]')).map((item) => {
    if (item.getAttribute("role") === "separator") {
      return "—";
    }
    const text = item.textContent ?? "";
    // The shortcut is drawn, not read: the item has it as `aria-keyshortcuts`.
    const shortcut = item.querySelector("span[aria-hidden]")?.textContent ?? "";
    const title = text.slice(0, text.length - shortcut.length);
    const off = item.hasAttribute("data-disabled") ? " (off)" : "";
    return `${title}${shortcut && ` ${shortcut}`}${off}`;
  });
}

/** Empty space under the rows: the list's content, outside every row. */
const emptySpace = () => {
  const content = grid().firstElementChild;
  if (!(content instanceof HTMLElement)) throw new Error("no list content");
  return content;
};

test("a file's menu: Jerry's commands with their shortcuts, Windows options last", async () => {
  await shown();
  const menu = await rightClick(within(row("ნახაზი.dwg")).getByText("ნახაზი.dwg"));

  await waitFor(() => expect(items(menu)).toContain("Paste Ctrl+V"));
  expect(items(menu)).toEqual([
    "Open Enter",
    "—",
    "Cut Ctrl+X",
    "Copy Ctrl+C",
    "Paste Ctrl+V",
    "—",
    "Rename F2",
    "Delete Del",
    "—",
    "Copy path Ctrl+Shift+C",
    "Copy name Ctrl+Alt+C",
    "—",
    "Open in Terminal",
    "Show in Explorer",
    "—",
    "Windows options… (off)",
  ]);
  expect(menu).toHaveAccessibleName("Selected items");
  // Screen readers get the shortcut apart from the title.
  expect(within(menu).getByRole("menuitem", { name: "Copy path" })).toHaveAttribute(
    "aria-keyshortcuts",
    "Control+Shift+C",
  );
});

test("a folder's menu opens it in a new tab and adds it to the favorites", async () => {
  const { calls } = await shown();
  let menu = await rightClick(within(row("Plot")).getByText("Plot"));
  expect(items(menu).slice(0, 2)).toEqual(["Open Enter", "Open in new tab Ctrl+Enter"]);
  expect(items(menu)).toContain("Add to favorites Ctrl+Shift+D");

  fireEvent.click(within(menu).getByRole("menuitem", { name: /Add to favorites/ }));
  await waitFor(() => expect(sent(calls, "favorites_add")).toEqual([{ listing: 1, entry: 0 }]));

  menu = await rightClick(within(row("Plot")).getByText("Plot"));
  fireEvent.click(within(menu).getByRole("menuitem", { name: /Open in Terminal/ }));
  await waitFor(() => expect(sent(calls, "open_terminal")).toEqual([{ listing: 1, entry: 0 }]));
});

test("Paste on a folder pastes into it, and is grayed out without files to paste", async () => {
  const { calls } = await shown();
  const menu = await rightClick(within(row("Plot")).getByText("Plot"));
  const paste = await within(menu).findByRole("menuitem", { name: /Paste/ });
  await waitFor(() => expect(paste).not.toHaveAttribute("data-disabled"));
  fireEvent.click(paste);
  await waitFor(() => expect(sent(calls, "clipboard_paste")).toEqual([{ listing: 1, into: 0 }]));
});

test("without files on the clipboard Paste stays grayed out", async () => {
  await shown({ files: false });
  const menu = await rightClick(emptySpace());
  await waitFor(() => expect(items(menu)).toContain("Paste Ctrl+V (off)"));
});

test("the menu acts on the selected items: copy, Show in Explorer", async () => {
  const { folder, calls } = await shown();
  folder.setSelection({ selected: new Set([1, 2]), anchor: 1, focused: 2 });
  // A right click on a selected row keeps the selection.
  let menu = await rightClick(within(row("b.txt")).getByText("b.txt"));
  fireEvent.click(within(menu).getByRole("menuitem", { name: "Copy" }));
  await waitFor(() =>
    expect(sent(calls, "clipboard_copy")).toEqual([{ listing: 1, entries: [1, 2], cut: false }]),
  );

  menu = await rightClick(within(row("b.txt")).getByText("b.txt"));
  fireEvent.click(within(menu).getByRole("menuitem", { name: "Show in Explorer" }));
  await waitFor(() =>
    expect(sent(calls, "show_in_explorer")).toEqual([{ listing: 1, entries: [1, 2] }]),
  );
});

test("Delete moves to the Recycle Bin, and with Shift asks to delete for good", async () => {
  const outcome = { deleted: 1, noRecycleBin: [], failed: [] };
  const { calls } = await shown({
    answer: (cmd) => (cmd === "delete_recycle" ? outcome : null),
  });
  let menu = await rightClick(within(row("b.txt")).getByText("b.txt"));
  fireEvent.click(within(menu).getByRole("menuitem", { name: "Delete" }));
  await waitFor(() =>
    expect(sent(calls, "delete_recycle")).toEqual([{ listing: 1, entries: [2], system: false }]),
  );

  menu = await rightClick(within(row("b.txt")).getByText("b.txt"));
  const item = within(menu).getByRole("menuitem", { name: "Delete" });
  fireEvent.pointerDown(item, { shiftKey: true, pointerType: "mouse" });
  fireEvent.click(item, { shiftKey: true });
  expect(await screen.findByRole("dialog", { name: /Delete permanently/ })).toBeTruthy();
  expect(sent(calls, "delete_recycle")).toHaveLength(1);
});

test("Rename opens the name for editing once the menu has closed", async () => {
  await shown();
  const menu = await rightClick(within(row("b.txt")).getByText("b.txt"));
  fireEvent.click(within(menu).getByRole("menuitem", { name: /^Rename/ }));
  await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  const field = await within(grid()).findByRole("textbox");
  expect(field).toHaveValue("b.txt");
  expect(document.activeElement).toBe(field);
});

test("empty space has the folder's menu, and the selection is cleared", async () => {
  const { folder, calls } = await shown();
  folder.setSelection({ selected: new Set([1]), anchor: 1, focused: 1 });
  const menu = await rightClick(emptySpace());

  expect(folder.store.getState().selected.size).toBe(0);
  await waitFor(() => expect(items(menu)).toContain("Paste Ctrl+V"));
  expect(items(menu)).toEqual([
    "New folder Ctrl+Shift+N",
    "—",
    "Paste Ctrl+V",
    "—",
    "View",
    "Sort by",
    "Refresh Ctrl+R",
    "—",
    "Open in Terminal",
    "—",
    "Windows options… (off)",
  ]);
  expect(menu).toHaveAccessibleName("This folder");

  // The folder itself, though an item still has the focus.
  fireEvent.click(within(menu).getByRole("menuitem", { name: "Open in Terminal" }));
  await waitFor(() => expect(sent(calls, "open_terminal")).toEqual([{ listing: 1, entry: null }]));
});

test("Sort by sorts the folder by a column, then in a direction", async () => {
  const { folder } = await shown();
  let menu = await rightClick(emptySpace());
  fireEvent.keyDown(within(menu).getByRole("menuitem", { name: "Sort by" }), { key: "Enter" });
  let sub = await screen.findByRole("menu", { name: "Sort by" });
  expect(within(sub).getByRole("menuitemradio", { name: "Name" })).toBeChecked();
  fireEvent.click(within(sub).getByRole("menuitemradio", { name: "Date modified" }));
  expect(folder.store.getState().sort).toMatchObject({ column: "modified", descending: false });

  menu = await rightClick(emptySpace());
  fireEvent.keyDown(within(menu).getByRole("menuitem", { name: "Sort by" }), { key: "Enter" });
  sub = await screen.findByRole("menu", { name: "Sort by" });
  fireEvent.click(within(sub).getByRole("menuitemradio", { name: "Descending" }));
  expect(folder.store.getState().sort).toMatchObject({ column: "modified", descending: true });
});

test("View shows Details, and switches hidden files and compact rows", async () => {
  const { calls } = await shown();
  const menu = await rightClick(emptySpace());
  fireEvent.keyDown(within(menu).getByRole("menuitem", { name: "View" }), { key: "Enter" });
  const sub = await screen.findByRole("menu", { name: "View" });
  expect(items(sub)).toEqual([
    "Details view Ctrl+Shift+1",
    "List view Ctrl+Shift+2 (off)",
    "Grid view Ctrl+Shift+3 (off)",
    "—",
    "Hidden files Ctrl+H",
    "Compact rows Ctrl+Alt+D",
  ]);
  expect(within(sub).getByRole("menuitemradio", { name: /Details/ })).toBeChecked();
  fireEvent.click(within(sub).getByRole("menuitemcheckbox", { name: /Hidden files/ }));
  await waitFor(() =>
    expect(sent(calls, "settings_set")).toContainEqual({ key: "showHidden", value: true }),
  );
});

test("Shift+F10 opens the menu on the selection, the Menu key on the folder without one", async () => {
  const { folder } = await shown();
  folder.setSelection({ selected: new Set([2]), anchor: 2, focused: 2 });
  act(() => grid().focus());
  fireEvent.keyDown(grid(), { key: "F10", code: "F10", shiftKey: true });
  let menu = await screen.findByRole("menu", { name: "Selected items" });
  expect(items(menu)[0]).toBe("Open Enter");
  fireEvent.keyDown(menu, { key: "Escape" });
  await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  // The focus is back on the list.
  expect(document.activeElement).toBe(grid());

  folder.setSelection({ selected: new Set(), anchor: 2, focused: 2 });
  fireEvent.keyDown(grid(), { key: "ContextMenu", code: "ContextMenu" });
  menu = await screen.findByRole("menu", { name: "This folder" });
  expect(items(menu)[0]).toBe("New folder Ctrl+Shift+N");
});

test("a command that fails says why; Retry runs it again", async () => {
  const gone = { code: "NotFound", path: String.raw`C:\Work\Plot`, params: {} } as JerryError;
  let fail = true;
  const { calls } = await shown({
    answer: (cmd) => {
      if (cmd === "open_terminal" && fail) {
        fail = false;
        return Promise.reject(gone);
      }
      return null;
    },
  });
  const menu = await rightClick(emptySpace());
  fireEvent.click(within(menu).getByRole("menuitem", { name: "Open in Terminal" }));

  const dialog = await screen.findByRole("dialog", { name: "Couldn't open a terminal" });
  fireEvent.click(within(dialog).getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(sent(calls, "open_terminal")).toHaveLength(2));
  expect(screen.queryByRole("dialog")).toBeNull();
});
