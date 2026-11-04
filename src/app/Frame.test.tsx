import { mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test } from "vitest";
import type { Settings } from "../ipc";
import { defaultSettings } from "../stores/settings.testing";
import { useWindow } from "../stores/window";
import { Frame } from "./Frame";

let calls: { cmd: string; args: unknown }[];

beforeEach(() => {
  calls = [];
  mockIPC((cmd, args) => {
    // The title bar's own IPC has its tests in TitleBar.test.tsx.
    if (!cmd.startsWith("title_bar_")) {
      calls.push({ cmd, args });
    }
  });
  resizeTo(1440);
});

afterEach(() => {
  useWindow.setState({ activePane: "left" });
});

function resizeTo(width: number) {
  act(() => {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: width });
    window.dispatchEvent(new Event("resize"));
  });
}

function columns(): string {
  const root = screen.getByRole("banner").parentElement;
  return root?.style.getPropertyValue("--frame-columns") ?? "";
}

test("the zones show at the design's sizes", () => {
  render(<Frame settings={defaultSettings} />);

  expect(screen.getByRole("banner")).toHaveClass("h-titlebar");
  expect(screen.getByRole("navigation", { name: "Navigation" })).toBeInTheDocument();
  expect(screen.getByRole("region", { name: "Left pane, active" })).toBeInTheDocument();
  expect(screen.getByRole("region", { name: "Right pane" })).toBeInTheDocument();
  expect(screen.getByRole("complementary", { name: "Preview" })).toBeInTheDocument();
  expect(screen.getByRole("contentinfo")).toHaveClass("h-statusbar");
  expect(columns()).toBe("232px minmax(0, 0.5fr) 1px minmax(0, 0.5fr) 288px");
  // The transfer panel shows only while a transfer runs.
  expect(screen.queryByRole("region", { name: "Transfers" })).toBeNull();
});

test("single-pane mode shows one pane, without the divider", () => {
  render(<Frame settings={{ ...defaultSettings, dualPane: false }} />);
  expect(screen.getByRole("region", { name: "Left pane, active" })).toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Right pane" })).toBeNull();
  expect(screen.queryByRole("separator", { name: "Resize panes" })).toBeNull();
  expect(columns()).toBe("232px minmax(0, 1fr) 288px");
});

test("the zones use the saved sizes", () => {
  const settings: Settings = { ...defaultSettings, sidebarWidth: 260, previewWidth: 320 };
  render(<Frame settings={{ ...settings, paneSplit: 0.4 }} />);
  expect(columns()).toBe("260px minmax(0, 0.4fr) 1px minmax(0, 0.6fr) 320px");
});

test("a narrow window hides the preview, then the sidebar", () => {
  render(<Frame settings={defaultSettings} />);

  resizeTo(900);
  expect(screen.queryByRole("complementary", { name: "Preview" })).toBeNull();
  expect(screen.getByRole("navigation")).toBeInTheDocument();

  resizeTo(700);
  expect(screen.queryByRole("navigation")).toBeNull();
  expect(columns()).toBe("minmax(0, 0.5fr) 1px minmax(0, 0.5fr)");

  resizeTo(1440);
  expect(screen.getByRole("complementary", { name: "Preview" })).toBeInTheDocument();
});

test("dragging an edge resizes the zone and the drop saves it", () => {
  render(<Frame settings={defaultSettings} />);
  const handle = screen.getByRole("separator", { name: "Resize sidebar" });

  fireEvent.pointerDown(handle, { button: 0, clientX: 232 });
  fireEvent.pointerMove(handle, { clientX: 300 });
  expect(columns()).toMatch(/^300px /);
  expect(calls).toEqual([]);

  fireEvent.pointerUp(handle, { clientX: 300 });
  expect(calls).toEqual([{ cmd: "settings_set", args: { key: "sidebarWidth", value: 300 } }]);
  // Until the saved value comes back, the dragged one shows.
  expect(columns()).toMatch(/^300px /);
});

test("the divider sets the panes' split and the preview edge its width", () => {
  render(<Frame settings={defaultSettings} />);
  const panes = 1440 - 232 - 288 - 1;

  const divider = screen.getByRole("separator", { name: "Resize panes" });
  fireEvent.pointerDown(divider, { button: 0, clientX: 692 });
  fireEvent.pointerMove(divider, { clientX: 232 + panes * 0.6 });
  fireEvent.pointerUp(divider);

  const preview = screen.getByRole("separator", { name: "Resize preview" });
  fireEvent.pointerDown(preview, { button: 0, clientX: 1152 });
  fireEvent.pointerMove(preview, { clientX: 1100 });
  fireEvent.pointerUp(preview);

  expect(calls).toEqual([
    { cmd: "settings_set", args: { key: "paneSplit", value: 0.6 } },
    { cmd: "settings_set", args: { key: "previewWidth", value: 340 } },
  ]);
});

test("a pointer that only moves over a handle drags nothing", () => {
  render(<Frame settings={defaultSettings} />);
  const handle = screen.getByRole("separator", { name: "Resize sidebar" });
  fireEvent.pointerMove(handle, { clientX: 300 });
  fireEvent.pointerUp(handle);
  expect(columns()).toMatch(/^232px /);
  expect(calls).toEqual([]);
});

test("clicking a pane makes it the active one, with the accent bar", () => {
  render(<Frame settings={defaultSettings} />);
  const right = screen.getByRole("region", { name: "Right pane" });

  fireEvent.pointerDown(right);

  expect(screen.getByRole("region", { name: "Right pane, active" })).toBe(right);
  expect(right.querySelector(".bg-accent")).not.toBeNull();
  expect(screen.getByRole("region", { name: "Left pane" }).querySelector(".bg-accent")).toBeNull();
});

test("the status bar switches the density", () => {
  render(<Frame settings={defaultSettings} />);
  expect(screen.getByRole("button", { name: "Comfortable rows" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );

  fireEvent.click(screen.getByRole("button", { name: "Compact rows" }));

  expect(calls).toEqual([{ cmd: "settings_set", args: { key: "density", value: "compact" } }]);
});

test("the toolbar skeleton is in place, its controls disabled until their features arrive", () => {
  render(<Frame settings={defaultSettings} />);
  const left = screen.getByRole("region", { name: "Left pane, active" });
  for (const name of [
    "Back (Alt+Left)",
    "Forward (Alt+Right)",
    "Up (Alt+Up)",
    "Refresh (Ctrl+R)",
  ]) {
    expect(left.querySelector(`button[aria-label="${name}"]`)).toBeDisabled();
  }
  expect(left.querySelector('input[aria-label="Quick filter"]')).toHaveAttribute(
    "placeholder",
    "Filter this folder",
  );
  expect(left.querySelector('button[aria-label="Details view (Ctrl+Shift+1)"]')).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});
