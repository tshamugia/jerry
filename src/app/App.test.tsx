import { mockIPC } from "@tauri-apps/api/mocks";
import { act, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { useAppearance } from "../stores/appearance";
import { useSettings } from "../stores/settings";
import { defaultSettings, windows11 } from "../stores/settings.testing";
import { resetTabs } from "../stores/tabs";
import { paneTabs } from "../stores/tabs.testing";
import { App } from "./App";

// The address bar folds its segments by size; jsdom has no layout, and no ResizeObserver.
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

afterEach(() => {
  useSettings.setState({ settings: undefined });
  useAppearance.setState({ appearance: undefined });
  resetTabs();
});

test("renders the frame and shows the window only once the settings, appearance and tabs are in", () => {
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
  });

  render(<App />);
  expect(screen.queryByRole("banner")).toBeNull();

  act(() => useSettings.setState({ settings: defaultSettings }));
  act(() => useAppearance.setState({ appearance: windows11 }));
  expect(screen.queryByRole("banner")).toBeNull();
  expect(calls).not.toContain("app_ready");

  // The restored session's tabs.
  act(() => void paneTabs());
  expect(screen.getByRole("banner")).toBeInTheDocument();
  expect(screen.getByRole("region", { name: "Left pane, active" })).toBeInTheDocument();
  expect(calls).toContain("app_ready");
});
