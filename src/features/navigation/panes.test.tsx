import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, beforeEach, expect, test } from "vitest";
import { Frame } from "../../app/Frame";
import { useCommandKeys } from "../../commands/dispatch";
import { commandById, runCommand } from "../../commands/registry";
import type { Settings } from "../../ipc";
import { type FakeListings, fakeListings } from "../../ipc/listing.testing";
import { useSettings } from "../../stores/settings";
import { defaultSettings } from "../../stores/settings.testing";
import { addTab, resetTabs, type Tab, useTabs } from "../../stores/tabs";
import { paneTabs } from "../../stores/tabs.testing";
import { useWindow } from "../../stores/window";
import { PaneList } from "../file-list/PaneList";

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

let core: FakeListings;
let tabs: Record<"left" | "right", Tab>;
beforeEach(async () => {
  // As the core: a folder opened by its token keeps that token.
  core = fakeListings({
    opened: (target) => (target.kind === "folder" ? { folder: target.folder } : {}),
  });
  tabs = paneTabs();
  await tabs.left.open({ kind: "path", path: String.raw`C:\Work` });
  await tabs.right.open({ kind: "path", path: String.raw`D:\Issued` });
  useSettings.setState({ settings: defaultSettings });
});
afterEach(() => {
  resetTabs();
  useSettings.setState({ settings: undefined });
  useWindow.setState({ activePane: "left" });
});

/** The window as the app draws it: the frame with each pane's list, the keyboard commands on. */
function Window() {
  useCommandKeys();
  const settings = useSettings((state) => state.settings);
  return settings ? (
    <Frame settings={settings} left={<PaneList pane="left" />} right={<PaneList pane="right" />} />
  ) : null;
}

const setDual = (dualPane: boolean) =>
  act(() =>
    useSettings.setState({ settings: { ...defaultSettings, dualPane } satisfies Settings }),
  );
const list = (pane: "Left" | "Right") => screen.getByRole("grid", { name: `${pane} pane files` });

test("F3 and the status bar switch to one pane and back; both panes keep their tabs", () => {
  render(<Window />);
  addTab("right");

  fireEvent.keyDown(window, { code: "F3", key: "F3" });
  expect(core.calls.at(-1)).toEqual({
    cmd: "settings_set",
    args: { key: "dualPane", value: false },
  });
  // The core answers with settings:changed.
  setDual(false);
  expect(screen.queryByRole("grid", { name: "Right pane files" })).toBeNull();
  expect(screen.getAllByRole("tablist")).toHaveLength(1);
  expect(useTabs.getState().right.tabs).toHaveLength(2);

  const switches = screen.getByRole("group", { name: "Dual pane (F3)" });
  expect(within(switches).getByRole("button", { name: "One pane" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  fireEvent.click(within(switches).getByRole("button", { name: "Two panes" }));
  expect(core.calls.at(-1)?.args).toEqual({ key: "dualPane", value: true });
  setDual(true);
  expect(list("Right")).toBeInTheDocument();
  expect(screen.getAllByRole("tab", { selected: true })).toHaveLength(2);
});

test("with one pane, the left pane is the active one and its list takes the keyboard", () => {
  render(<Window />);
  act(() => list("Right").focus());
  expect(useWindow.getState().activePane).toBe("right");

  setDual(false);
  expect(useWindow.getState().activePane).toBe("left");
  expect(document.activeElement).toBe(list("Left"));
});

test("Tab in a file list makes the other pane active and gives it the keyboard", () => {
  render(<Window />);
  act(() => list("Left").focus());

  fireEvent.keyDown(list("Left"), { code: "Tab", key: "Tab" });
  expect(useWindow.getState().activePane).toBe("right");
  expect(document.activeElement).toBe(list("Right"));
  fireEvent.keyDown(list("Right"), { code: "Tab", key: "Tab" });
  expect(useWindow.getState().activePane).toBe("left");

  // On a toolbar button Tab moves on through the controls, and with one pane it does nothing.
  const refresh = within(screen.getByRole("region", { name: /^Left pane/ })).getByRole("button", {
    name: "Refresh (Ctrl+R)",
  });
  act(() => refresh.focus());
  expect(fireEvent.keyDown(refresh, { code: "Tab", key: "Tab" })).toBe(true);
  setDual(false);
  act(() => list("Left").focus());
  expect(fireEvent.keyDown(list("Left"), { code: "Tab", key: "Tab" })).toBe(true);
});

test("Same folder in other pane sends the other pane's tab to this folder", async () => {
  render(<Window />);
  const command = commandById("sameFolderInOtherPane");
  if (!command) throw new Error("the command is registered");
  const leftFolder = tabs.left.current()?.folder;

  expect(runCommand(command)).toBe(true);
  await waitFor(() => expect(tabs.right.current()?.folder).toBe(leftFolder));
  expect(tabs.right.store.getState().history).toHaveLength(2);
  expect(useWindow.getState().activePane).toBe("left");

  // The right tab's menu does it the other way, once it shows its own folder again.
  act(() => tabs.right.back());
  fireEvent.contextMenu(screen.getAllByRole("tab", { selected: true })[1] as HTMLElement);
  const menu = await screen.findByRole("menu", { name: "Tab" });
  fireEvent.click(within(menu).getByRole("menuitem", { name: "Open in other pane" }));
  await waitFor(() => expect(tabs.left.store.getState().history).toHaveLength(2));

  // With one pane there is no other one.
  setDual(false);
  expect(runCommand(command)).toBe(false);
});
