import { mockIPC } from "@tauri-apps/api/mocks";
import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { App } from "./App";

test("shows the window and the version from the core", async () => {
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
    if (cmd === "app_version") return "1.2.3";
    if (cmd === "sample_list") return [];
    return undefined;
  });

  render(<App />);

  expect(await screen.findByText("Version 1.2.3")).toBeInTheDocument();
  expect(calls).toContain("app_ready");
});
