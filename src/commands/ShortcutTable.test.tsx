import { mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test } from "vitest";
import { useKeymap } from "../stores/keymap";
import { useOverlay } from "../stores/overlay";
import { useSettings } from "../stores/settings";
import { defaultSettings } from "../stores/settings.testing";
import { useCommandKeys } from "./dispatch";
import { buildKeymap } from "./keymap";
import { COMMANDS } from "./registry";
import { ShortcutTable } from "./ShortcutTable";

let calls: string[];

beforeEach(() => {
  calls = [];
  mockIPC((cmd) => {
    calls.push(cmd);
  });
  useSettings.setState({ settings: defaultSettings });
});

afterEach(() => {
  useSettings.setState({ settings: undefined });
  useOverlay.setState({ overlay: undefined });
  useKeymap.setState({ keymap: buildKeymap(COMMANDS) });
});

function Window() {
  useCommandKeys();
  return <ShortcutTable />;
}

test("F1 shows the shortcuts from the registry; Esc closes them", () => {
  render(<Window />);
  expect(screen.queryByRole("dialog")).toBeNull();

  fireEvent.keyDown(document.body, { code: "F1", key: "F1" });

  const dialog = screen.getByRole("dialog", { name: "Keyboard shortcuts" });
  const view = within(dialog).getByRole("region", { name: "View" });
  expect(within(view).getByText("Row density")).toBeInTheDocument();
  expect(within(view).getByText("Ctrl+Alt+D")).toBeInTheDocument();
  const other = within(dialog).getByRole("region", { name: "Other" });
  expect(within(other).getByText("F1")).toBeInTheDocument();
  expect(within(other).getByText("?")).toBeInTheDocument();
  // Commands that aren't built yet stay out.
  expect(within(dialog).queryByText("Duplicate")).toBeNull();

  // The window's shortcuts don't work under the dialog.
  fireEvent.keyDown(dialog, { code: "KeyD", ctrlKey: true, altKey: true });
  expect(calls).not.toContain("settings_set");

  fireEvent.keyDown(dialog, { code: "Escape", key: "Escape" });
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(useOverlay.getState().overlay).toBeUndefined();
});

test("the table shows the user's shortcuts", () => {
  render(<Window />);
  act(() => {
    useKeymap.setState({
      keymap: buildKeymap(COMMANDS, [{ command: "toggleDensity", shortcut: "Ctrl+Alt+R" }]),
    });
    useOverlay.setState({ overlay: "shortcuts" });
  });
  const dialog = screen.getByRole("dialog", { name: "Keyboard shortcuts" });
  expect(within(dialog).getByText("Ctrl+Alt+R")).toBeInTheDocument();
  expect(within(dialog).queryByText("Ctrl+Alt+D")).toBeNull();
});
