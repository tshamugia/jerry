import type { Channel } from "@tauri-apps/api/core";
import { mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, test } from "vitest";
import { batchMessage, doneMessage, errorMessage, orderMessage } from "../../ipc/listing.testing";
import { DevListing } from "./DevListing";

/**
 * A fake `listing_open` that streams `messages` once it has answered, and a `listing_sort` that
 * answers with `sorted`.
 */
function streams(messages: ArrayBuffer[], sorted?: ArrayBuffer) {
  mockIPC((cmd, args) => {
    if (cmd === "listing_sort") {
      return sorted ?? null;
    }
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

const rowNames = () =>
  within(screen.getByRole("list"))
    .getAllByRole("listitem")
    .map((row) => row.firstChild?.textContent);

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

test("paints the stream's order and re-sorts by a column", async () => {
  streams(
    [
      batchMessage(0, [
        { name: "file10.txt", size: 10 },
        { name: "file2.txt", size: 2048 },
        { name: "პროექტი", attributes: 0x10 },
      ]),
      orderMessage([2, 1, 0]),
      doneMessage(3, 1, 1.5),
    ],
    orderMessage([2, 0, 1], { column: 3 }),
  );
  render(<DevListing />);

  list(String.raw`C:\Work`);

  expect(await screen.findByText(/^3 items in 1 batch\./)).toBeInTheDocument();
  expect(rowNames()).toEqual(["პროექტი", "file2.txt", "file10.txt"]);

  fireEvent.click(screen.getByRole("button", { name: "Size" }));

  expect(
    await screen.findByText(/^Sorted 3 items by size, ascending: painted/),
  ).toBeInTheDocument();
  expect(rowNames()).toEqual(["პროექტი", "file10.txt", "file2.txt"]);
  expect(screen.getByRole("button", { name: "Size ↑" })).toBeInTheDocument();
});

test("ignores an order for a sort it no longer wants", async () => {
  streams(
    [batchMessage(0, [{ name: "a.txt" }, { name: "b.txt" }]), doneMessage(2, 1, 1)],
    // Answers for name, ascending, though the click asks for name, descending.
    orderMessage([1, 0]),
  );
  render(<DevListing />);
  list(String.raw`C:\Work`);
  expect(await screen.findByText(/^2 items in 1 batch\./)).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "Name ↑" }));

  expect(await screen.findByText(/^No new order yet/)).toBeInTheDocument();
  expect(rowNames()).toEqual(["a.txt", "b.txt"]);
});

test("shows a listing error as text", async () => {
  streams([errorMessage(String.raw`{"code":"AccessDenied","path":"C:\\დახურული"}`)]);
  render(<DevListing />);

  list(String.raw`C:\დახურული`);

  expect(
    await screen.findByText("You don't have permission to access “დახურული”."),
  ).toBeInTheDocument();
});
