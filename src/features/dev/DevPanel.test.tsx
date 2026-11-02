import { mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
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
