import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test } from "vitest";
import { useCommandKeys } from "../../commands/dispatch";
import { batchMessage, doneMessage, fakeListings, orderMessage } from "../../ipc/listing.testing";
import { useOverlay } from "../../stores/overlay";
import { useSettings } from "../../stores/settings";
import { defaultSettings } from "../../stores/settings.testing";
import { resetTabs } from "../../stores/tabs";
import { paneFolder, paneTabs } from "../../stores/tabs.testing";
import { SelectByPattern } from "./SelectByPattern";

beforeEach(() => {
  paneTabs();
});
afterEach(() => {
  resetTabs();
  useOverlay.setState({ overlay: undefined });
  useSettings.setState({ settings: undefined });
});

function Window() {
  useCommandKeys();
  return <SelectByPattern />;
}

/** The left pane: three drawings and notes, plan_rev2.dwg selected; a hidden drawing not shown. */
async function opened() {
  useSettings.setState({ settings: defaultSettings });
  fakeListings({
    messages: () => [
      batchMessage(0, [
        { name: "plan_rev2.dwg" },
        { name: "ნახაზი_rev1.DWG" },
        { name: "notes.txt" },
        { name: "plan.dwg" },
        { name: "old_rev9.dwg", attributes: 0x2 },
      ]),
      orderMessage([0, 1, 2, 3, 4]),
      doneMessage(5, 1, 1),
    ],
  });
  const folder = paneFolder("left");
  await folder.open({ kind: "path", path: String.raw`C:\Work` });
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  folder.setSelection({ selected: new Set([0]), anchor: 0, focused: 0 });
  render(<Window />);
  return folder;
}

const selected = () => [...paneFolder("left").store.getState().selected].sort();

test("Ctrl+Shift+A asks for a pattern, counts the matches and selects them on Enter", async () => {
  await opened();

  fireEvent.keyDown(document.body, { code: "KeyA", key: "A", ctrlKey: true, shiftKey: true });
  const input = await screen.findByRole("textbox", { name: "Pattern" });
  expect(screen.getByRole("dialog", { name: "Select by pattern" })).toBeInTheDocument();

  fireEvent.change(input, { target: { value: "*_rev*" } });
  expect(screen.getByText("2 of 4 match. Enter selects them.")).toBeInTheDocument();
  fireEvent.submit(input.closest("form") as HTMLFormElement);

  // The Georgian name with a capital extension matches; old_rev9.dwg isn't shown, so it isn't.
  expect(selected()).toEqual([0, 1]);
  expect(useOverlay.getState().overlay).toBeUndefined();
});

test("Esc leaves the selection as it was; the last pattern comes back next time", async () => {
  await opened();

  act(() => useOverlay.setState({ overlay: "selectByPattern" }));
  const input = await screen.findByRole("textbox", { name: "Pattern" });
  expect(input).toHaveValue("*_rev*");
  fireEvent.change(input, { target: { value: "*.txt" } });
  fireEvent.keyDown(input, { code: "Escape", key: "Escape" });

  await waitFor(() => expect(useOverlay.getState().overlay).toBeUndefined());
  expect(selected()).toEqual([0]);
});

test("an empty pattern selects nothing and keeps the dialog open", async () => {
  await opened();
  act(() => useOverlay.setState({ overlay: "selectByPattern" }));
  const input = await screen.findByRole("textbox", { name: "Pattern" });
  fireEvent.change(input, { target: { value: "" } });
  expect(screen.getByText("* stands for any characters, ? for one.")).toBeInTheDocument();

  fireEvent.submit(input.closest("form") as HTMLFormElement);
  expect(selected()).toEqual([0]);
  expect(useOverlay.getState().overlay).toBe("selectByPattern");
});
