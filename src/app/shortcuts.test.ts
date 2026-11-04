import { mockIPC } from "@tauri-apps/api/mocks";
import { renderHook } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { useSettings } from "../stores/settings";
import { defaultSettings } from "../stores/settings.testing";
import { isToggleDensity, useDensityShortcut } from "./shortcuts";

afterEach(() => {
  useSettings.setState({ settings: undefined });
});

function key(init: KeyboardEventInit, target?: EventTarget): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  if (target) {
    Object.defineProperty(event, "target", { value: target });
  }
  return event;
}

test("Ctrl+Alt+D switches the density, on any keyboard layout", () => {
  const calls: unknown[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
  });
  useSettings.setState({ settings: defaultSettings });
  renderHook(() => useDensityShortcut());

  // The Georgian layout sends key "დ" with code KeyD.
  const event = key({ code: "KeyD", key: "დ", ctrlKey: true, altKey: true });
  window.dispatchEvent(event);

  expect(event.defaultPrevented).toBe(true);
  expect(calls).toEqual([{ cmd: "settings_set", args: { key: "density", value: "compact" } }]);
});

test("other keys and modifiers don't", () => {
  expect(isToggleDensity(key({ code: "KeyD", ctrlKey: true }))).toBe(false);
  expect(isToggleDensity(key({ code: "KeyD", ctrlKey: true, altKey: true, shiftKey: true }))).toBe(
    false,
  );
  expect(isToggleDensity(key({ code: "KeyF", ctrlKey: true, altKey: true }))).toBe(false);
  expect(isToggleDensity(key({ code: "KeyD", ctrlKey: true, altKey: true, repeat: true }))).toBe(
    false,
  );
});

test("AltGr in a text field types instead", () => {
  const input = document.createElement("input");
  input.type = "text";
  const altGr = key({ code: "KeyD", ctrlKey: true, altKey: true, modifierAltGraph: true }, input);
  expect(isToggleDensity(altGr)).toBe(false);
  // Outside a text field, AltGr+D is Ctrl+Alt+D as Windows reports it.
  expect(
    isToggleDensity(key({ code: "KeyD", ctrlKey: true, altKey: true, modifierAltGraph: true })),
  ).toBe(true);
});
