import { afterEach, expect, test } from "vitest";
import { activateTab, closeTab, setTabs, type TabView, useTabs } from "./tabs";

afterEach(() => {
  useTabs.setState({ left: { tabs: [], active: null }, right: { tabs: [], active: null } });
});

function tab(id: number, pinned = false): TabView {
  return { id, name: `tab ${id}`, pinned };
}

test("panes start without tabs", () => {
  expect(useTabs.getState().left).toEqual({ tabs: [], active: null });
});

test("pinned tabs go first; a missing shown tab falls back to the first", () => {
  setTabs("left", [tab(1), tab(2, true), tab(3)], 9);
  expect(useTabs.getState().left).toEqual({ tabs: [tab(2, true), tab(1), tab(3)], active: 2 });
});

test("closing the shown tab shows its right neighbour, or else its left one", () => {
  setTabs("right", [tab(1), tab(2), tab(3)], 2);

  closeTab("right", 2);
  expect(useTabs.getState().right).toEqual({ tabs: [tab(1), tab(3)], active: 3 });
  closeTab("right", 3);
  expect(useTabs.getState().right).toEqual({ tabs: [tab(1)], active: 1 });
  closeTab("right", 1);
  expect(useTabs.getState().right).toEqual({ tabs: [], active: null });
});

test("closing another tab keeps the shown one; a pinned tab doesn't close", () => {
  setTabs("left", [tab(1, true), tab(2), tab(3)], 3);

  closeTab("left", 2);
  closeTab("left", 1);
  expect(useTabs.getState().left).toEqual({ tabs: [tab(1, true), tab(3)], active: 3 });

  activateTab("left", 1);
  expect(useTabs.getState().left.active).toBe(1);
  activateTab("left", 42);
  expect(useTabs.getState().left.active).toBe(1);
});
