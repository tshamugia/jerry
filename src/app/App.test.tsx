import { mockIPC } from "@tauri-apps/api/mocks";
import { act, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { useAppearance } from "../stores/appearance";
import { useSettings } from "../stores/settings";
import { defaultSettings, windows11 } from "../stores/settings.testing";
import { App } from "./App";

afterEach(() => {
  useSettings.setState({ settings: undefined });
  useAppearance.setState({ appearance: undefined });
});

test("renders the frame and shows the window only once the settings and appearance are in", () => {
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
  });

  render(<App />);
  expect(screen.queryByRole("banner")).toBeNull();

  act(() => useSettings.setState({ settings: defaultSettings }));
  expect(calls).not.toContain("app_ready");

  act(() => useAppearance.setState({ appearance: windows11 }));
  expect(screen.getByRole("banner")).toBeInTheDocument();
  expect(screen.getByRole("region", { name: "Left pane, active" })).toBeInTheDocument();
  expect(calls).toContain("app_ready");
});
