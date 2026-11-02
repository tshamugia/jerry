import { mockIPC } from "@tauri-apps/api/mocks";
import { render, waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
import type { FrameState } from "../ipc";
import { App } from "./App";
import { CaptionButtons } from "./CaptionButtons";

test("attaches the frame before showing the window", async () => {
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
    if (cmd === "listing_georgian_path") return "C:\\Temp\\jerry-synth\\ქართული სახელები";
    return cmd === "title_bar_attach" ? { mica: true, snapLayouts: true } : undefined;
  });

  render(<App />);

  await waitFor(() => expect(calls).toContain("app_ready"));
  expect(calls.indexOf("title_bar_attach")).toBeLessThan(calls.indexOf("app_ready"));
  expect(calls).toContain("title_bar_set_buttons");
  expect(document.documentElement.dataset.backdrop).toBe("mica");
});

test("draws caption buttons from the native state", () => {
  mockIPC(() => undefined);
  const state: FrameState = { hovered: "close", pressed: null, maximized: true, active: true };

  const { container, rerender } = render(<CaptionButtons state={state} />);

  expect(caption(container, "Close")).toHaveAttribute("data-look", "hover");
  expect(caption(container, "Restore")).toHaveAttribute("data-look", "rest");

  // Held on maximize but moved off it: nothing lights up.
  rerender(<CaptionButtons state={{ ...state, hovered: "minimize", pressed: "maximize" }} />);
  expect(caption(container, "Restore")).toHaveAttribute("data-look", "rest");
  expect(caption(container, "Minimize")).toHaveAttribute("data-look", "rest");

  rerender(<CaptionButtons state={{ ...state, hovered: null, active: false }} />);
  expect(caption(container, "Minimize")).toHaveAttribute("data-look", "inactive");
});

function caption(container: HTMLElement, label: string): Element | null {
  return container.querySelector(`[data-label="${label}"]`);
}
