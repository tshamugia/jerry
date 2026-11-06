import { emit } from "@tauri-apps/api/event";
import { waitFor } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { commandById, runCommand } from "../../commands/registry";
import type { LaunchOpen } from "../../ipc";
import { batchMessage, doneMessage, fakeListings, orderMessage } from "../../ipc/listing.testing";
import { addTab, resetTabs, tabById, useTabs } from "../../stores/tabs";
import { useToasts } from "../../stores/toasts";
import { useWindow } from "../../stores/window";
import { followLaunches } from "./instance";

afterEach(() => {
  resetTabs();
  useToasts.setState({ toasts: [] });
  useWindow.setState({ activePane: "left" });
});

/** A fake core whose `launch_take` hands out `queue`'s opens once each. */
function core(queue: LaunchOpen[][]) {
  return fakeListings({
    events: true,
    messages: () => [
      batchMessage(0, [{ name: "a.dwg" }, { name: "გეგმა.dwg" }]),
      orderMessage([0, 1]),
      doneMessage(2, 2, 2),
    ],
    other: (cmd) => (cmd === "launch_take" ? (queue.shift() ?? []) : null),
  });
}

const opens = (fake: ReturnType<typeof fakeListings>) =>
  fake.calls.filter((call) => call.cmd === "listing_open").map((call) => call.args.target);

test("a start's paths open in new tabs of the active pane, after the restored tabs", async () => {
  const fake = core([
    [
      { kind: "folder", folder: 7, select: null },
      { kind: "folder", folder: 8, select: "გეგმა.dwg" },
    ],
  ]);
  let restore = () => {};
  const restored = new Promise<void>((resolve) => {
    restore = resolve;
  });
  const stop = followLaunches(restored);
  try {
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(fake.calls.some((call) => call.cmd === "launch_take")).toBe(false);

    // The session's tabs come first.
    addTab("left", { activate: true });
    addTab("right", { activate: true });
    useWindow.setState({ activePane: "right" });
    restore();

    await waitFor(() => expect(useTabs.getState().right.tabs).toHaveLength(3));
    expect(opens(fake)).toEqual([
      { kind: "folder", folder: 7 },
      { kind: "folder", folder: 8 },
    ]);
    const { tabs, active } = useTabs.getState().right;
    expect(tabs.map((tab) => tab.id)).toEqual([2, 3, 4]);
    expect(active).toBe(4);
    expect(useTabs.getState().left.tabs).toHaveLength(1);

    // The file the start named is selected.
    const folder = tabById(4)?.folder;
    await waitFor(() => expect(folder?.store.getState().status).toBe("ready"));
    expect(folder?.store.getState()).toMatchObject({ focused: 1, selected: new Set([1]) });
  } finally {
    stop();
  }
});

test("a start while Jerry runs opens its path; one that leads nowhere says why", async () => {
  const fake = core([
    [],
    [
      { kind: "folder", folder: 9, select: null },
      {
        kind: "failed",
        path: String.raw`X:\nowhere`,
        error: { code: "NotFound", path: String.raw`X:\nowhere` },
      },
    ],
  ]);
  addTab("left", { activate: true });
  const stop = followLaunches(Promise.resolve());
  try {
    await waitFor(() =>
      expect(fake.calls.filter((call) => call.cmd === "launch_take")).toHaveLength(1),
    );
    expect(opens(fake)).toEqual([]);

    await emit("launch:opened", null);
    await waitFor(() => expect(useTabs.getState().left.tabs).toHaveLength(2));
    expect(opens(fake)).toEqual([{ kind: "folder", folder: 9 }]);
    expect(useToasts.getState().toasts).toEqual([
      {
        id: expect.any(Number),
        title: "Can't open “X:\\nowhere”",
        detail: "“nowhere” couldn't be found. It may have been moved or deleted.",
      },
    ]);
  } finally {
    stop();
  }
});

test("Exit Jerry quits through the core, with Ctrl+Q", () => {
  const fake = core([]);
  const exit = commandById("exitJerry");
  expect(exit?.keys).toEqual(["Ctrl+Q"]);
  expect(exit && runCommand(exit)).toBe(true);
  expect(fake.calls.map((call) => call.cmd)).toContain("app_exit");
});
