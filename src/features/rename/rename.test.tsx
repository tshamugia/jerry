import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, expect, test, vi } from "vitest";
import { useCommandKeys } from "../../commands/dispatch";
import type { JerryError } from "../../ipc";
import {
  batchMessage,
  type Call,
  diffMessage,
  doneMessage,
  type FakeListings,
  fakeListings,
  orderMessage,
} from "../../ipc/listing.testing";
import { useSettings } from "../../stores/settings";
import { defaultSettings } from "../../stores/settings.testing";
import { resetTabs } from "../../stores/tabs";
import { paneFolder, paneTabs } from "../../stores/tabs.testing";
import { useWindow } from "../../stores/window";
import { PaneList } from "../file-list/PaneList";
import { RenameDialogs } from "./RenameDialogs";
import { cancelRename, useRename } from "./rename";

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
  cancelRename();
  useRename.setState({ createFailed: undefined });
  resetTabs();
  useSettings.setState({ settings: undefined });
  useWindow.setState({ activePane: "left" });
});

function Window() {
  useCommandKeys();
  return (
    <>
      <PaneList pane="left" />
      <RenameDialogs />
    </>
  );
}

interface Core {
  /** What `rename` answers: nothing for success, or an error. */
  renamed?: (entry: number, name: string) => JerryError | undefined;
  /** What `create_folder` answers: the name it made, or an error. */
  created?: () => string | JerryError;
}

/**
 * The left pane shows the folder ქვესაქაღალდე (0), then ნახაზი.dwg (1), b.txt (2) and c.txt (3).
 * The fake core answers `rename` and `create_folder` from `core`.
 */
async function shown(core: Core = {}, settings = {}) {
  useSettings.setState({ settings: { ...defaultSettings, ...settings } });
  const listings: FakeListings = fakeListings({
    messages: () => [
      batchMessage(0, [
        { name: "ქვესაქაღალდე", attributes: 0x10 },
        { name: "ნახაზი.dwg", size: 10 },
        { name: "b.txt" },
        { name: "c.txt" },
      ]),
      orderMessage([0, 1, 2, 3]),
      doneMessage(4, 1, 1),
    ],
    other: (cmd, args) => {
      if (cmd === "rename") {
        const error = core.renamed?.(args.entry as number, args.name as string);
        if (error) throw error;
        return null;
      }
      if (cmd === "create_folder") {
        const made = core.created?.() ?? (args.name as string);
        if (typeof made !== "string") throw made;
        return made;
      }
      return null;
    },
  });
  render(<Window />);
  const folder = paneFolder("left");
  await act(() => folder.open({ kind: "path", path: String.raw`C:\Work` }));
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  return { folder, ...listings };
}

const grid = () => screen.getByRole("grid", { name: "Left pane files" });
const field = () => screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement;
const press = (code: string, target: Element = grid()) => fireEvent.keyDown(target, { code });
const type = (text: string) => fireEvent.change(field(), { target: { value: text } });
const renames = (calls: Call[]) =>
  calls.filter((call) => call.cmd === "rename").map((call) => call.args);

test("F2 opens the focused name with only the name selected, not the extension", async () => {
  const { folder } = await shown();
  folder.setSelection({ selected: new Set([1]), anchor: 1, focused: 1 });

  press("F2");

  const input = field();
  expect(input.value).toBe("ნახაზი.dwg");
  expect(document.activeElement).toBe(input);
  expect([input.selectionStart, input.selectionEnd]).toEqual([0, "ნახაზი".length]);
});

test("a folder's name is selected whole", async () => {
  const { folder } = await shown();
  folder.setSelection({ selected: new Set([0]), anchor: 0, focused: 0 });

  press("F2");

  expect([field().selectionStart, field().selectionEnd]).toEqual([0, "ქვესაქაღალდე".length]);
});

test("Enter saves the new name in the core and gives the keyboard back to the list", async () => {
  const { folder, calls } = await shown();
  folder.setSelection({ selected: new Set([1]), anchor: 1, focused: 1 });
  press("F2");

  type("გეგმა.dwg");
  press("Enter", field());

  await waitFor(() =>
    expect(renames(calls)).toEqual([{ listing: 1, entry: 1, name: "გეგმა.dwg" }]),
  );
  await waitFor(() => expect(screen.queryByRole("textbox")).toBeNull());
  expect(document.activeElement).toBe(grid());
});

test("Esc keeps the old name", async () => {
  const { folder, calls } = await shown();
  folder.setSelection({ selected: new Set([2]), anchor: 2, focused: 2 });
  press("F2");

  type("x.txt");
  press("Escape", field());

  expect(screen.queryByRole("textbox")).toBeNull();
  expect(renames(calls)).toEqual([]);
  expect(screen.getByText("b.txt")).toBeDefined();
});

test("an invalid name shows why while typing, and Enter doesn't save it", async () => {
  const { folder, calls } = await shown();
  folder.setSelection({ selected: new Set([2]), anchor: 2, focused: 2 });
  press("F2");

  type("a:b.txt");
  expect(screen.getByText(/A name can't contain “:”/)).toBeDefined();
  expect(field().getAttribute("aria-invalid")).toBe("true");
  press("Enter", field());

  type("CON.txt");
  expect(
    screen.getByText("“CON” is a name Windows keeps for a device. Choose another name."),
  ).toBeDefined();
  type("b.txt.");
  expect(screen.getByText("A name can't end with a space or a dot.")).toBeDefined();
  type("");
  expect(screen.getByText("Type a name.")).toBeDefined();
  press("Enter", field());

  expect(renames(calls)).toEqual([]);
  expect(field()).toBeDefined();
});

test("a name another entry has is refused while typing; a change of case is not", async () => {
  const { folder, calls } = await shown();
  folder.setSelection({ selected: new Set([2]), anchor: 2, focused: 2 });
  press("F2");

  type("C.TXT");
  expect(screen.getByText("There's already an item named “C.TXT” here.")).toBeDefined();
  type("B.txt");
  expect(screen.queryByText(/already an item/)).toBeNull();
  press("Enter", field());

  await waitFor(() => expect(renames(calls)).toEqual([{ listing: 1, entry: 2, name: "B.txt" }]));
});

test("Tab saves and goes on to rename the next row", async () => {
  const { folder, calls } = await shown();
  folder.setSelection({ selected: new Set([1]), anchor: 1, focused: 1 });
  press("F2");

  type("გეგმა.dwg");
  press("Tab", field());

  await waitFor(() => expect(field().value).toBe("b.txt"));
  expect(renames(calls)).toEqual([{ listing: 1, entry: 1, name: "გეგმა.dwg" }]);
  expect(folder.store.getState().selected).toEqual(new Set([2]));
  // Unchanged, Tab just goes on.
  press("Tab", field());
  await waitFor(() => expect(field().value).toBe("c.txt"));
  expect(renames(calls)).toHaveLength(1);
});

test("a new extension asks first: Keep editing goes back, Change saves", async () => {
  const { folder, calls } = await shown();
  folder.setSelection({ selected: new Set([2]), anchor: 2, focused: 2 });
  press("F2");

  type("b.md");
  press("Enter", field());
  await screen.findByText(
    "Change the extension from “.txt” to “.md”? The file might not open in its program any more.",
  );
  fireEvent.click(screen.getByRole("button", { name: "Keep editing" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(field().value).toBe("b.md");
  // Back in the field, to go on typing.
  await waitFor(() => expect(document.activeElement).toBe(field()));
  expect(renames(calls)).toEqual([]);

  press("Enter", field());
  fireEvent.click(await screen.findByRole("button", { name: "Change" }));
  await waitFor(() => expect(renames(calls)).toEqual([{ listing: 1, entry: 2, name: "b.md" }]));
});

test("an extension that changes only in case doesn't ask", async () => {
  const { folder, calls } = await shown();
  folder.setSelection({ selected: new Set([2]), anchor: 2, focused: 2 });
  press("F2");

  type("b.TXT");
  press("Enter", field());

  await waitFor(() => expect(renames(calls)).toHaveLength(1));
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("with extensions hidden, only the name is edited and the extension stays", async () => {
  const { folder, calls } = await shown({}, { showExtensions: false });
  folder.setSelection({ selected: new Set([1]), anchor: 1, focused: 1 });
  press("F2");

  expect(field().value).toBe("ნახაზი");
  expect([field().selectionStart, field().selectionEnd]).toEqual([0, "ნახაზი".length]);
  type("ნახაზი 2");
  press("Enter", field());

  await waitFor(() =>
    expect(renames(calls)).toEqual([{ listing: 1, entry: 1, name: "ნახაზი 2.dwg" }]),
  );
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("a name the core refuses keeps the field open with why, and Enter tries again", async () => {
  let tries = 0;
  const { folder, calls } = await shown({
    renamed: () => {
      tries += 1;
      return tries === 1
        ? { code: "InUse", params: { app: null }, path: "C:\\Work\\b.txt" }
        : undefined;
    },
  });
  folder.setSelection({ selected: new Set([2]), anchor: 2, focused: 2 });
  press("F2");

  type("b2.txt");
  press("Enter", field());

  await screen.findByText(
    "“b.txt” is open in another program. Close it there and try again. Enter tries again, Esc keeps the old name.",
  );
  expect(field().value).toBe("b2.txt");
  press("Enter", field());
  await waitFor(() => expect(screen.queryByRole("textbox")).toBeNull());
  expect(renames(calls)).toHaveLength(2);
});

test("a renamed entry scrolls into view once the list has sorted it", async () => {
  const { folder, send } = await shown();
  folder.setSelection({ selected: new Set([3]), anchor: 3, focused: 3 });
  press("F2");
  type("a.txt");
  press("Enter", field());
  await waitFor(() => expect(screen.queryByRole("textbox")).toBeNull());
  const reveal = vi.spyOn(folder, "reveal");

  act(() => send(diffMessage([3], [], [{ name: "a.txt" }])));
  expect(reveal).not.toHaveBeenCalled();
  act(() => send(orderMessage([0, 3, 1, 2])));

  expect(reveal).toHaveBeenCalledWith(3);
  expect(folder.store.getState().scrollTo).toBeUndefined();
  expect(screen.getByText("a.txt")).toBeDefined();
  expect(folder.store.getState().selected).toEqual(new Set([3]));
});

test("Ctrl+Shift+N makes a new folder and opens its name once the list shows it", async () => {
  const { folder, calls, send } = await shown({ created: () => "New folder (2)" });
  folder.setSelection({ selected: new Set([2]), anchor: 2, focused: 2 });

  fireEvent.keyDown(grid(), { code: "KeyN", ctrlKey: true, shiftKey: true });

  await waitFor(() =>
    expect(calls.filter((call) => call.cmd === "create_folder").map((call) => call.args)).toEqual([
      { listing: 1, name: "New folder" },
    ]),
  );
  act(() => {
    send(diffMessage([4], [], [{ name: "New folder (2)", attributes: 0x10 }]));
    send(orderMessage([4, 0, 1, 2, 3]));
  });

  const input = await screen.findByRole("textbox", { name: "Name" });
  expect((input as HTMLInputElement).value).toBe("New folder (2)");
  expect([
    (input as HTMLInputElement).selectionStart,
    (input as HTMLInputElement).selectionEnd,
  ]).toEqual([0, "New folder (2)".length]);
  expect(folder.store.getState().selected).toEqual(new Set([4]));
  // Esc keeps the name it got.
  press("Escape", input);
  expect(screen.getByText("New folder (2)")).toBeDefined();
});

test("a new folder the quick filter would hide clears the filter", async () => {
  const { folder, send } = await shown();
  act(() => folder.setFilter("*.txt"));

  fireEvent.keyDown(grid(), { code: "KeyN", ctrlKey: true, shiftKey: true });
  await waitFor(() => expect(folder.store.getState().filter).toBe(""));
  act(() => {
    send(diffMessage([4], [], [{ name: "New folder", attributes: 0x10 }]));
    send(orderMessage([4, 0, 1, 2, 3]));
  });

  expect(((await screen.findByRole("textbox", { name: "Name" })) as HTMLInputElement).value).toBe(
    "New folder",
  );
});

test("a new folder that can't be made says why and offers Retry", async () => {
  const { calls } = await shown({
    created: () => ({ code: "AccessDenied", path: "C:\\Work" }),
  });

  fireEvent.keyDown(grid(), { code: "KeyN", ctrlKey: true, shiftKey: true });

  await screen.findByText("You don't have permission to access “Work”.");
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(calls.filter((call) => call.cmd === "create_folder")).toHaveLength(2));
});

test("a slow second click on the selected name renames it; a double click opens instead", async () => {
  const { folder } = await shown();
  useWindow.setState({ activePane: "left" });
  folder.setSelection({ selected: new Set([2]), anchor: 2, focused: 2 });
  const name = screen.getByText("b.txt");

  fireEvent.pointerDown(name, { button: 0 });
  fireEvent.pointerUp(name, { button: 0 });
  fireEvent.click(name, { detail: 1 });

  expect(screen.queryByRole("textbox")).toBeNull();
  await waitFor(() => expect(field().value).toBe("b.txt"), { timeout: 2000 });
  cancelRename();

  // A click on a name that wasn't selected only selects it.
  const other = screen.getByText("c.txt");
  fireEvent.pointerDown(other, { button: 0 });
  fireEvent.pointerUp(other, { button: 0 });
  fireEvent.click(other, { detail: 1 });
  await new Promise((resolve) => setTimeout(resolve, 700));
  expect(screen.queryByRole("textbox")).toBeNull();
  expect(folder.store.getState().selected).toEqual(new Set([3]));
});
