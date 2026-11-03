import { beforeEach, expect, test } from "vitest";
import { defaultSettings, windows11 } from "../stores/settings.testing";
import { applyAppearance } from "./appearance";

let root: HTMLElement;

beforeEach(() => {
  root = document.createElement("html");
});

test("the System theme follows the window's color scheme; Light and Dark don't", () => {
  applyAppearance(root, defaultSettings, windows11, true);
  expect(root.dataset.theme).toBe("dark");
  applyAppearance(root, defaultSettings, windows11, false);
  expect(root.dataset.theme).toBe("light");
  applyAppearance(root, { ...defaultSettings, theme: "dark" }, windows11, false);
  expect(root.dataset.theme).toBe("dark");
  applyAppearance(root, { ...defaultSettings, theme: "light" }, windows11, true);
  expect(root.dataset.theme).toBe("light");
});

test("density and Mica come from the settings; Mica only where Windows can draw it", () => {
  applyAppearance(root, { ...defaultSettings, density: "compact" }, windows11, false);
  expect(root.dataset.density).toBe("compact");
  expect(root.dataset.backdrop).toBe("mica");

  applyAppearance(root, { ...defaultSettings, mica: false }, windows11, false);
  expect(root.dataset.backdrop).toBe("solid");

  const windows10 = { mica: false, accent: windows11.accent };
  applyAppearance(root, defaultSettings, windows10, false);
  expect(root.dataset.backdrop).toBe("solid");
});

test("Match Windows sets the Windows accent; Jerry's colors come from the tokens", () => {
  const plum = { mica: true, accent: { light: "#8a3b7a", dark: "#c49dbc" } };
  applyAppearance(root, defaultSettings, plum, false);
  expect(root.style.getPropertyValue("--jerry-accent-base")).toBe("#8a3b7a");
  expect(root.style.getPropertyValue("--jerry-accent-base-dark")).toBe("#c49dbc");
  expect(root.dataset.accent).toBeUndefined();

  applyAppearance(root, { ...defaultSettings, accent: "teal" }, plum, false);
  expect(root.style.getPropertyValue("--jerry-accent-base")).toBe("");
  expect(root.dataset.accent).toBe("teal");

  applyAppearance(root, { ...defaultSettings, accent: "blue" }, plum, false);
  expect(root.dataset.accent).toBeUndefined();
});

test("Match Windows without a Windows accent falls back to Jerry's blue", () => {
  applyAppearance(root, defaultSettings, { mica: true, accent: null }, false);
  expect(root.style.getPropertyValue("--jerry-accent-base")).toBe("");
  expect(root.dataset.accent).toBeUndefined();
});

test("before the settings arrive only the theme is set", () => {
  applyAppearance(root, undefined, undefined, true);
  expect(root.dataset.theme).toBe("dark");
  expect(root.dataset.density).toBeUndefined();
  expect(root.dataset.backdrop).toBeUndefined();
});
