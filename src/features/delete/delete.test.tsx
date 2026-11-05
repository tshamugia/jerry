import type { Channel } from "@tauri-apps/api/core";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, expect, test } from "vitest";
import { useCommandKeys } from "../../commands/dispatch";
import type { DeleteOutcome, JerryError, Tally } from "../../ipc";
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
import { DeleteDialog } from "./DeleteDialog";
import { useDeleteDialog } from "./flow";

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
  useDeleteDialog.setState({ dialog: undefined });
});

function Window() {
  useCommandKeys();
  return (
    <>
      <PaneList pane="left" />
      <DeleteDialog />
    </>
  );
}

const done: DeleteOutcome = { deleted: 0, noRecycleBin: [], failed: [] };

interface Core {
  system?: boolean;
  recycled?: (entries: number[]) => DeleteOutcome;
  deleted?: (entries: number[]) => DeleteOutcome;
  tally?: Tally;
}

/**
 * The left pane shows ნახაზი.dwg (0), b.txt (2) and c.txt (3); desktop.ini (1) is hidden. The
 * fake core answers the delete commands from `core`.
 */
async function shown(core: Core = {}, settings = {}) {
  useSettings.setState({ settings: { ...defaultSettings, ...settings } });
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
    other: (cmd, args) => {
      const entries = args.entries as number[];
      switch (cmd) {
        case "delete_check":
          return { system: core.system ?? false };
        case "delete_recycle":
          return core.recycled?.(entries) ?? { ...done, deleted: entries.length };
        case "delete_permanently":
          return core.deleted?.(entries) ?? { ...done, deleted: entries.length };
        case "delete_measure": {
          const tally = core.tally ?? { files: 1, folders: 0, bytes: 10 };
          (args.onProgress as Channel<Tally>).onmessage({ ...tally, files: 0 });
          return tally;
        }
        default:
          return null;
      }
    },
  });
  render(<Window />);
  const folder = paneFolder("left");
  await act(() => folder.open({ kind: "path", path: String.raw`C:\Work` }));
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  return { folder, calls };
}

const press = (code: string, init: { shiftKey?: boolean } = {}) =>
  fireEvent.keyDown(screen.getByRole("grid", { name: "Left pane files" }), { code, ...init });

const sent = (calls: Call[], cmd: string) =>
  calls.filter((call) => call.cmd === cmd).map((call) => call.args);

test("Del recycles the shown selected rows in display order, without asking", async () => {
  const { folder, calls } = await shown();
  folder.setSelection({ selected: new Set([3, 1, 0]), anchor: 0, focused: 3 });

  press("Delete");

  await waitFor(() =>
    expect(sent(calls, "delete_recycle")).toEqual([{ listing: 1, entries: [0, 3], system: false }]),
  );
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("without a selection Del recycles the focused row", async () => {
  const { folder, calls } = await shown();
  folder.setSelection({ selected: new Set(), anchor: 2, focused: 2 });

  press("Delete");

  await waitFor(() =>
    expect(sent(calls, "delete_recycle")).toEqual([{ listing: 1, entries: [2], system: false }]),
  );
});

test("with the setting on, Del asks first; Cancel deletes nothing", async () => {
  const { folder, calls } = await shown({}, { confirmRecycle: true });
  folder.setSelection({ selected: new Set([0]), anchor: 0, focused: 0 });

  press("Delete");
  await screen.findByText("Move “ნახაზი.dwg” to the Recycle Bin?");
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(sent(calls, "delete_recycle")).toEqual([]);

  press("Delete");
  fireEvent.click(await screen.findByRole("button", { name: "Move to Recycle Bin" }));
  await waitFor(() => expect(sent(calls, "delete_recycle")).toHaveLength(1));
});

test("Shift+Del asks with what goes, counted in the core, and deletes for good", async () => {
  const { folder, calls } = await shown({
    tally: { files: 1204, folders: 3, bytes: 5 * 1024 ** 3 },
  });
  folder.setSelection({ selected: new Set([0, 2]), anchor: 0, focused: 2 });

  press("Delete", { shiftKey: true });

  await screen.findByText("Permanently delete 2 items?");
  await screen.findByText("This can't be undone. 3 folders, 1,204 files, 5 GB");
  expect(sent(calls, "delete_measure")[0]).toMatchObject({ listing: 1, entries: [0, 2] });
  fireEvent.click(screen.getByRole("button", { name: "Delete permanently" }));

  await waitFor(() =>
    expect(sent(calls, "delete_permanently")).toEqual([
      { listing: 1, entries: [0, 2], system: false },
    ]),
  );
  expect(sent(calls, "delete_recycle")).toEqual([]);
  expect(sent(calls, "delete_measure_cancel")).toEqual([{ listing: 1 }]);
});

test("a system folder needs the extra confirmation, which the delete passes on", async () => {
  const { folder, calls } = await shown({ system: true });
  folder.setSelection({ selected: new Set([2]), anchor: 2, focused: 2 });

  press("Delete");

  const confirm = await screen.findByRole("button", { name: "Move to Recycle Bin" });
  expect(confirm).toHaveProperty("disabled", true);
  fireEvent.click(screen.getByLabelText("I understand. Delete the system folder anyway."));
  expect(confirm).toHaveProperty("disabled", false);
  fireEvent.click(confirm);

  await waitFor(() =>
    expect(sent(calls, "delete_recycle")).toEqual([{ listing: 1, entries: [2], system: true }]),
  );
});

test("items without a Recycle Bin stay until the user says to delete them for good", async () => {
  const { folder, calls } = await shown({
    recycled: () => ({ deleted: 1, noRecycleBin: [3, 0], failed: [] }),
  });
  folder.setSelection({ selected: new Set([0, 2, 3]), anchor: 0, focused: 0 });

  press("Delete");

  await screen.findByText("2 items can't go to the Recycle Bin. Delete them permanently?");
  fireEvent.click(screen.getByRole("button", { name: "Delete permanently" }));
  await waitFor(() =>
    expect(sent(calls, "delete_permanently")).toEqual([
      { listing: 1, entries: [0, 3], system: false },
    ]),
  );
});

test("what stayed is listed, and Retry gives each entry the delete it failed, no more", async () => {
  const inUse: JerryError = {
    code: "InUse",
    params: { app: null },
    path: String.raw`C:\Work\ნახაზი.dwg`,
  };
  const { folder, calls } = await shown({
    recycled: (entries) =>
      entries.includes(2)
        ? { deleted: 0, noRecycleBin: [2], failed: [{ entry: 0, error: inUse }] }
        : { ...done, deleted: entries.length },
  });
  folder.setSelection({ selected: new Set([0, 2]), anchor: 0, focused: 0 });

  press("Delete");
  // b.txt has no Recycle Bin: keep it. ნახაზი.dwg was open in a program.
  fireEvent.click(await screen.findByRole("button", { name: "Keep it" }));
  await screen.findByText("1 item couldn't be deleted.");
  screen.getByText("“ნახაზი.dwg” is open in another program. Close it there and try again.");
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));

  await waitFor(() => expect(sent(calls, "delete_recycle")).toHaveLength(2));
  expect(sent(calls, "delete_recycle")[1]).toEqual({ listing: 1, entries: [0], system: false });
  expect(sent(calls, "delete_permanently")).toEqual([]);
});
