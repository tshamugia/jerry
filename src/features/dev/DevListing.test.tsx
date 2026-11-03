import type { Channel } from "@tauri-apps/api/core";
import { mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { expect, test } from "vitest";
import type { Sort } from "../../ipc";
import {
  batchMessage,
  diffMessage,
  doneMessage,
  errorMessage,
  orderMessage,
} from "../../ipc/listing.testing";
import { DevListing } from "./DevListing";

/**
 * A fake `listing_open` that streams `messages` once it has answered, and a `listing_sort` that
 * sends the order `sorted` gives for the sort through the same stream, as Rust does. `send` puts
 * more messages on the stream: what the watcher would send.
 */
function streams(messages: ArrayBuffer[], sorted?: (sort: Sort) => ArrayBuffer) {
  let channel: Channel<ArrayBuffer> | undefined;
  mockIPC((cmd, args) => {
    if (cmd === "listing_sort") {
      const order = sorted?.((args as { sort: Sort }).sort);
      if (order) {
        setTimeout(() => channel?.onmessage(order), 0);
      }
      return null;
    }
    if (cmd !== "listing_open") {
      return null;
    }
    channel = (args as { onMessage: Channel<ArrayBuffer> }).onMessage;
    const opened = channel;
    setTimeout(() => {
      for (const message of messages) opened.onmessage(message);
    }, 0);
    return { id: 1, path: String.raw`C:\Work` };
  });
  return { send: (message: ArrayBuffer) => act(() => channel?.onmessage(message)) };
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
    () => orderMessage([2, 0, 1], { column: 3 }),
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
    [
      batchMessage(0, [
        { name: "b.txt", size: 1 },
        { name: "a.txt", size: 2 },
      ]),
      orderMessage([1, 0]),
      doneMessage(2, 1, 1),
    ],
    (sort) => (sort.column === "size" ? orderMessage([0, 1], { column: 3 }) : orderMessage([1, 0])),
  );
  render(<DevListing />);
  list(String.raw`C:\Work`);
  expect(await screen.findByText(/^2 items in 1 batch\./)).toBeInTheDocument();

  // By size, then back to name before the size order arrives: that one is not painted.
  fireEvent.click(screen.getByRole("button", { name: "Size" }));
  fireEvent.click(screen.getByRole("button", { name: "Name" }));

  expect(
    await screen.findByText(/^Sorted 2 items by name, ascending: painted/),
  ).toBeInTheDocument();
  expect(rowNames()).toEqual(["a.txt", "b.txt"]);
});

test("a diff repaints new, changed and removed entries", async () => {
  const { send } = streams([
    batchMessage(0, [
      { name: "a.txt", size: 1 },
      { name: "b.txt", size: 2 },
    ]),
    orderMessage([0, 1]),
    doneMessage(2, 1, 1),
  ]);
  render(<DevListing />);
  list(String.raw`C:\Work`);
  expect(await screen.findByText(/^2 items in 1 batch\./)).toBeInTheDocument();

  // a.txt deleted, b.txt grew, ნახაზი.dwg created: the watcher's diff and its order.
  send(
    diffMessage(
      [1, 2],
      [0],
      [
        { name: "b.txt", size: 4096 },
        { name: "ნახაზი.dwg", size: 10 },
      ],
    ),
  );
  send(orderMessage([1, 2]));

  expect(await screen.findByText(/^Live: 1 new, 1 changed, 1 removed/)).toBeInTheDocument();
  expect(rowNames()).toEqual(["b.txt", "ნახაზი.dwg"]);
  expect(screen.getByText("4 KB")).toBeInTheDocument();
  expect(screen.getByText("2 items")).toBeInTheDocument();
});

test("shows a listing error as text", async () => {
  streams([errorMessage(String.raw`{"code":"AccessDenied","path":"C:\\დახურული"}`)]);
  render(<DevListing />);

  list(String.raw`C:\დახურული`);

  expect(
    await screen.findByText("You don't have permission to access “დახურული”."),
  ).toBeInTheDocument();
});
