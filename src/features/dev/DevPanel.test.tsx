import { mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
import type { Settings } from "../../ipc";
import { useSettings } from "../../stores/settings";
import { DevPanel } from "./DevPanel";

test("shows a command's JerryError as text", async () => {
  mockIPC((cmd) =>
    cmd === "dev_sample_error"
      ? Promise.reject({ code: "NotFound", path: String.raw`C:\Temp\jerry-dev-missing\ნახაზი.dwg` })
      : undefined,
  );
  render(<DevPanel />);

  fireEvent.click(screen.getByRole("button", { name: "Sample error" }));

  expect(await screen.findByRole("status")).toHaveTextContent(
    "“ნახაზი.dwg” couldn't be found. It may have been moved or deleted.",
  );
});

test("a panicked job shows as an internal error", async () => {
  mockIPC(() => Promise.reject({ code: "Internal", path: null }));
  render(<DevPanel />);

  fireEvent.click(screen.getByRole("button", { name: "Panic in a job" }));

  expect(await screen.findByRole("status")).toHaveTextContent(
    "Something went wrong in Jerry. Details are in the log.",
  );
});

test("shows the settings from the store and changes them through the core", async () => {
  const sent: unknown[] = [];
  mockIPC((cmd, args) => {
    if (cmd === "settings_set") sent.push(args);
  });
  useSettings.setState({
    settings: {
      theme: "dark",
      showHidden: true,
      showExtensions: true,
      clickToOpen: "double",
    } as Settings,
  });
  render(<DevPanel />);

  expect(
    screen.getByText(
      "Settings: hidden files shown, extensions shown, double click to open, theme dark.",
    ),
  ).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Toggle hidden files" }));
  fireEvent.click(screen.getByRole("button", { name: "Next theme" }));

  await waitFor(() =>
    expect(sent).toEqual([
      { key: "showHidden", value: false },
      { key: "theme", value: "system" },
    ]),
  );
  useSettings.setState({ settings: undefined });
});
