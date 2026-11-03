import type { Channel } from "@tauri-apps/api/core";
import { mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { batchMessage, doneMessage, errorMessage } from "../../ipc/listing.testing";
import { DevListing } from "./DevListing";

/** A fake `listing_open` that streams `messages` once it has answered. */
function streams(messages: ArrayBuffer[]) {
  mockIPC((cmd, args) => {
    if (cmd !== "listing_open") {
      return null;
    }
    const channel = (args as { onMessage: Channel<ArrayBuffer> }).onMessage;
    setTimeout(() => {
      for (const message of messages) channel.onmessage(message);
    }, 0);
    return { id: 1, path: String.raw`C:\Work` };
  });
}

function list(path: string) {
  fireEvent.change(screen.getByRole("textbox", { name: "Folder path" }), {
    target: { value: path },
  });
  fireEvent.click(screen.getByRole("button", { name: "List" }));
}

test("paints the first rows and reports the timings", async () => {
  streams([
    batchMessage(0, [
      { name: "ნახაზი.dwg", size: 2048 },
      { name: "პროექტი", attributes: 0x10 },
    ]),
    batchMessage(2, [{ name: "c.txt" }]),
    doneMessage(3, 2, 1.5),
  ]);
  render(<DevListing />);

  list(String.raw`C:\Work`);

  expect(await screen.findByText("ნახაზი.dwg")).toBeInTheDocument();
  expect(screen.getByText("2 KB")).toBeInTheDocument();
  expect(await screen.findByText(/^3 items in 2 batches\. First rows painted/)).toBeInTheDocument();
  expect(screen.getByText("3 items")).toBeInTheDocument();
});

test("shows a listing error as text", async () => {
  streams([errorMessage(String.raw`{"code":"AccessDenied","path":"C:\\დახურული"}`)]);
  render(<DevListing />);

  list(String.raw`C:\დახურული`);

  expect(
    await screen.findByText("You don't have permission to access “დახურული”."),
  ).toBeInTheDocument();
});

test("rows show their icons: their own by entry, the rest by type", async () => {
  streams([
    batchMessage(0, [
      { name: "ნახაზი.dwg", iconKey: 7 },
      { name: "setup.exe", iconKey: 0, modified: 1_767_225_600_000 },
    ]),
    doneMessage(2, 1, 0.5),
  ]);
  const { container } = render(<DevListing />);

  list(String.raw`C:\Work`);

  expect(await screen.findByText(/^2 items in 1 batch\./)).toBeInTheDocument();
  const icons = [...container.querySelectorAll("img")]
    .map((img) => new URL(img.src))
    .filter((url) => !url.pathname.startsWith("/place-") && !url.pathname.startsWith("/drive-"));
  expect(icons.map((url) => url.pathname.replace(/-\d+$/, ""))).toEqual(["/type-7", "/entry-1-1"]);
  expect(icons[1]?.searchParams.get("m")).toBe("1767225600000");
});
