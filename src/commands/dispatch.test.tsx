import { mockIPC } from "@tauri-apps/api/mocks";
import { renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useKeymap } from "../stores/keymap";
import { useOverlay } from "../stores/overlay";
import { useSettings } from "../stores/settings";
import { defaultSettings } from "../stores/settings.testing";
import { keyTarget, RENAME_SCOPE, useCommandKeys } from "./dispatch";
import { buildKeymap } from "./keymap";
import { COMMANDS } from "./registry";

let calls: { cmd: string; args: unknown }[];

beforeEach(() => {
  calls = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
  });
  useSettings.setState({ settings: defaultSettings });
});

afterEach(() => {
  useSettings.setState({ settings: undefined });
  useOverlay.setState({ overlay: undefined });
  useKeymap.setState({ keymap: buildKeymap(COMMANDS) });
  document.body.replaceChildren();
});

function element(html: string): Element {
  const host = document.createElement("div");
  host.innerHTML = html;
  document.body.append(host);
  const target = host.querySelector("[data-target]");
  if (!target) throw new Error("no [data-target]");
  return target;
}

/** Presses a key at `target` (the window if none), as WebView2 delivers it. */
function press(init: KeyboardEventInit, target?: Element): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  (target ?? window).dispatchEvent(event);
  return event;
}

const densitySet = (value: string) => ({
  cmd: "settings_set",
  args: { key: "density", value },
});

describe("keyTarget", () => {
  it("tells text fields, buttons and the rest apart", () => {
    expect(keyTarget(element('<input data-target type="text">'))).toEqual({
      scope: "window",
      focus: "text",
    });
    expect(keyTarget(element("<textarea data-target></textarea>")).focus).toBe("text");
    expect(keyTarget(element('<input data-target type="search">')).focus).toBe("text");
    expect(keyTarget(element('<input data-target type="checkbox">')).focus).toBe("button");
    expect(keyTarget(element("<button data-target>Go</button>")).focus).toBe("button");
    expect(keyTarget(element('<div role="tab" data-target></div>')).focus).toBe("button");
    expect(keyTarget(element('<div data-target tabindex="0"></div>'))).toEqual({
      scope: "window",
      focus: "other",
    });
    expect(keyTarget(window)).toEqual({ scope: "window", focus: "other" });
  });

  it("finds the innermost dialog or rename field", () => {
    expect(keyTarget(element('<div role="dialog"><input data-target></div>'))).toEqual({
      scope: "dialog",
      focus: "text",
    });
    expect(keyTarget(element('<div role="alertdialog"><b data-target></b></div>')).scope).toBe(
      "dialog",
    );
    const rename = document.createElement("input");
    for (const [name, value] of Object.entries(RENAME_SCOPE)) {
      rename.setAttribute(name, value);
    }
    document.body.append(rename);
    expect(keyTarget(rename)).toEqual({ scope: "rename", focus: "text" });
  });
});

describe("useCommandKeys", () => {
  it("runs Ctrl+Alt+D on the Georgian layout and keeps the key from doing anything else", () => {
    renderHook(() => useCommandKeys());
    // The Georgian layout sends key "დ" with code KeyD.
    const event = press({ code: "KeyD", key: "დ", ctrlKey: true, altKey: true });
    expect(event.defaultPrevented).toBe(true);
    expect(calls).toEqual([densitySet("compact")]);
  });

  it("runs a held shortcut once, unless the command repeats", () => {
    renderHook(() => useCommandKeys());
    const held = press({ code: "KeyD", key: "d", ctrlKey: true, altKey: true, repeat: true });
    expect(held.defaultPrevented).toBe(true);
    expect(calls).toEqual([]);
  });

  it("lets AltGr type in a text field", () => {
    renderHook(() => useCommandKeys());
    const input = element('<input data-target type="text">');
    const altGr = press(
      { code: "KeyD", key: "", ctrlKey: true, altKey: true, modifierAltGraph: true },
      input,
    );
    expect(altGr.defaultPrevented).toBe(false);
    expect(calls).toEqual([]);
    // Outside a text field, AltGr+D is Ctrl+Alt+D as Windows reports it.
    press({ code: "KeyD", key: "", ctrlKey: true, altKey: true, modifierAltGraph: true });
    expect(calls).toEqual([densitySet("compact")]);
  });

  it("opens the shortcut table with F1 or ?, but lets ? type in a text field", () => {
    renderHook(() => useCommandKeys());
    const input = element('<input data-target type="text">');
    const typed = press({ code: "Slash", key: "?", shiftKey: true }, input);
    expect(typed.defaultPrevented).toBe(false);
    expect(useOverlay.getState().overlay).toBeUndefined();

    expect(press({ code: "F1", key: "F1" }, input).defaultPrevented).toBe(true);
    expect(useOverlay.getState().overlay).toBe("shortcuts");

    useOverlay.setState({ overlay: undefined });
    press({ code: "Slash", key: "?", shiftKey: true });
    expect(useOverlay.getState().overlay).toBe("shortcuts");
  });

  it("leaves keys of commands that can't run to their default", () => {
    renderHook(() => useCommandKeys());
    // Duplicate isn't built yet.
    expect(press({ code: "KeyD", key: "დ", ctrlKey: true }).defaultPrevented).toBe(false);
    // Row density needs the settings.
    useSettings.setState({ settings: undefined });
    expect(press({ code: "KeyD", ctrlKey: true, altKey: true }).defaultPrevented).toBe(false);
    expect(calls).toEqual([]);
  });

  it("leaves keys a component handled alone", () => {
    renderHook(() => useCommandKeys());
    const input = element('<input data-target type="text">');
    input.addEventListener("keydown", (event) => event.preventDefault());
    press({ code: "KeyD", ctrlKey: true, altKey: true }, input);
    expect(calls).toEqual([]);
  });

  it("follows the user's shortcuts", () => {
    useKeymap.setState({
      keymap: buildKeymap(COMMANDS, [{ command: "toggleDensity", shortcut: "Ctrl+Shift+D" }]),
    });
    renderHook(() => useCommandKeys());
    expect(press({ code: "KeyD", ctrlKey: true, altKey: true }).defaultPrevented).toBe(false);
    press({ code: "KeyD", ctrlKey: true, shiftKey: true });
    expect(calls).toEqual([densitySet("compact")]);
  });

  it("stops listening when unmounted", () => {
    const { unmount } = renderHook(() => useCommandKeys());
    unmount();
    press({ code: "KeyD", ctrlKey: true, altKey: true });
    expect(calls).toEqual([]);
  });
});
