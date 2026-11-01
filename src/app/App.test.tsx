import { mockIPC } from "@tauri-apps/api/mocks";
import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { App } from "./App";

test("shows the window and the spike's sample folder", async () => {
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
    if (cmd === "sample_list") return [{ id: 0, name: "ნახაზი.dwg", isDir: false, size: 12 }];
    return undefined;
  });

  render(<App />);

  expect(await screen.findByText("ნახაზი.dwg")).toBeInTheDocument();
  expect(calls).toContain("app_ready");
});
