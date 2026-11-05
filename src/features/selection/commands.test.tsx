import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, expect, test } from "vitest";
import { useCommandKeys } from "../../commands/dispatch";
import { batchMessage, doneMessage, fakeListings, orderMessage } from "../../ipc/listing.testing";
import { useSettings } from "../../stores/settings";
import { defaultSettings } from "../../stores/settings.testing";
import { resetTabs } from "../../stores/tabs";
import { paneFolder, paneTabs } from "../../stores/tabs.testing";
import { useWindow } from "../../stores/window";
import { PaneList } from "../file-list/PaneList";

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
});

function Window() {
  useCommandKeys();
  return (
    <>
      <PaneList pane="left" />
      <PaneList pane="right" />
    </>
  );
}

/** The left pane shows a.txt, b.txt and c.txt; desktop.ini is hidden. */
async function shown() {
  useSettings.setState({ settings: { ...defaultSettings, showHidden: false } });
  fakeListings({
    messages: () => [
      batchMessage(0, [
        { name: "a.txt" },
        { name: "desktop.ini", attributes: 0x2 | 0x4 },
        { name: "b.txt" },
        { name: "c.txt" },
      ]),
      orderMessage([0, 1, 2, 3]),
      doneMessage(4, 1, 1),
    ],
  });
  render(<Window />);
  const folder = paneFolder("left");
  await act(() => folder.open({ kind: "path", path: String.raw`C:\Work` }));
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  return folder;
}

const press = (code: string, init: { ctrlKey?: boolean; shiftKey?: boolean } = {}) =>
  fireEvent.keyDown(screen.getByRole("grid", { name: "Left pane files" }), { code, ...init });
const selected = () => [...paneFolder("left").store.getState().selected].sort();

test("Ctrl+A selects only the rows the list shows; Esc clears them", async () => {
  await shown();

  press("KeyA", { ctrlKey: true });
  expect(selected()).toEqual([0, 2, 3]);
  press("Escape");
  expect(selected()).toEqual([]);
});

test("Ctrl+I inverts within the shown rows; Ctrl+Space toggles the focused row", async () => {
  const folder = await shown();
  folder.setSelection({ selected: new Set([2]), anchor: 2, focused: 2 });

  press("KeyI", { ctrlKey: true });
  expect(selected()).toEqual([0, 3]);
  press("Space", { ctrlKey: true });
  expect(selected()).toEqual([0, 2, 3]);
  press("Space", { ctrlKey: true });
  expect(selected()).toEqual([0, 3]);
});

test("the keys act on the active pane only", async () => {
  await shown();
  act(() => useWindow.setState({ activePane: "right" }));

  press("KeyA", { ctrlKey: true });
  expect(selected()).toEqual([]);
});

test("hiding hidden files again drops them from the selection", async () => {
  const folder = await shown();
  act(() => useSettings.setState({ settings: { ...defaultSettings, showHidden: true } }));
  press("KeyA", { ctrlKey: true });
  expect(selected()).toEqual([0, 1, 2, 3]);

  act(() => useSettings.setState({ settings: { ...defaultSettings, showHidden: false } }));
  expect(selected()).toEqual([0, 2, 3]);
  expect(folder.store.getState().focused).toBeUndefined();
});
