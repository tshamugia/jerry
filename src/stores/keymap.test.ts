import { mockIPC } from "@tauri-apps/api/mocks";
import { renderHook } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { buildKeymap, shortcutsOf } from "../commands/keymap";
import { COMMANDS } from "../commands/registry";
import type { Keybinding } from "../ipc";
import { loadKeymap, useCommandLabel, useKeymap } from "./keymap";

afterEach(() => {
  useKeymap.setState({ keymap: buildKeymap(COMMANDS) });
});

test("the defaults work until the user's shortcuts arrive from the core", async () => {
  expect(shortcutsOf(useKeymap.getState().keymap, "toggleDensity")).toEqual(["Ctrl+Alt+D"]);

  const changed: Keybinding[] = [{ command: "toggleDensity", shortcut: "Ctrl+Shift+Y" }];
  mockIPC((cmd) => (cmd === "keybindings_get" ? changed : undefined));
  await loadKeymap();

  expect(shortcutsOf(useKeymap.getState().keymap, "toggleDensity")).toEqual(["Ctrl+Shift+Y"]);
  expect(renderHook(() => useCommandLabel("toggleDensity")).result.current).toBe(
    "Row density (Ctrl+Shift+Y)",
  );
});

test("if they can't be read, the defaults stay", async () => {
  mockIPC((cmd) => {
    if (cmd === "keybindings_get") {
      throw { code: "accessDenied", path: null };
    }
  });
  await loadKeymap();
  expect(shortcutsOf(useKeymap.getState().keymap, "toggleDensity")).toEqual(["Ctrl+Alt+D"]);
});

test("a tooltip without a shortcut is just the title", () => {
  useKeymap.setState({
    keymap: buildKeymap(COMMANDS, [{ command: "refresh", shortcut: null }]),
  });
  expect(renderHook(() => useCommandLabel("refresh")).result.current).toBe("Refresh");
  expect(renderHook(() => useCommandLabel("back")).result.current).toBe("Back (Alt+←)");
});
