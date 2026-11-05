import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test } from "vitest";
import {
  batchMessage,
  diffMessage,
  doneMessage,
  errorMessage,
  fakeListings,
  orderMessage,
} from "../../ipc/listing.testing";
import { resetTabs } from "../../stores/tabs";
import { paneFolder, paneTabs } from "../../stores/tabs.testing";
import { useWindow } from "../../stores/window";
import { DevListing } from "./DevListing";

beforeEach(() => {
  paneTabs();
});
afterEach(() => {
  resetTabs();
  useWindow.setState({ activePane: "left" });
});

function list(path: string) {
  fireEvent.change(screen.getByRole("textbox", { name: "Folder path" }), {
    target: { value: path },
  });
  fireEvent.click(screen.getByRole("button", { name: "List" }));
}

test("opens the folder in the active pane's list and reports the timings", async () => {
  const core = fakeListings({
    messages: () => [
      batchMessage(0, [{ name: "ნახაზი.dwg" }, { name: "პროექტი", attributes: 0x10 }]),
      batchMessage(2, [{ name: "c.txt" }]),
      orderMessage([1, 0, 2]),
      doneMessage(3, 2, 1.5),
    ],
  });
  useWindow.setState({ activePane: "right" });
  render(<DevListing />);

  list(String.raw`C:\Work`);

  expect(await screen.findByText(/^3 items in 2 batches\. First rows painted/)).toBeInTheDocument();
  expect(screen.getByText("3 items")).toBeInTheDocument();
  const opened = core.calls.find((call) => call.cmd === "listing_open");
  expect(opened?.args).toMatchObject({
    tab: 2,
    target: { kind: "path", path: String.raw`C:\Work` },
  });
  expect(paneFolder("right").store.getState().status).toBe("ready");
});

test("shows a listing error as text", async () => {
  fakeListings({
    messages: () => [errorMessage(String.raw`{"code":"AccessDenied","path":"C:\\დახურული"}`)],
  });
  render(<DevListing />);

  list(String.raw`C:\დახურული`);

  expect(
    await screen.findByText("You don't have permission to access “დახურული”."),
  ).toBeInTheDocument();
});

test("times a re-sort and reports live changes", async () => {
  const core = fakeListings({
    messages: () => [
      batchMessage(0, [
        { name: "a.txt", size: 1 },
        { name: "b.txt", size: 2 },
      ]),
      orderMessage([0, 1]),
      doneMessage(2, 1, 1),
    ],
    sorted: () => orderMessage([1, 0], { column: 3, descending: true }),
  });
  render(<DevListing />);
  list(String.raw`C:\Work`);
  expect(await screen.findByText(/^2 items in 1 batch\./)).toBeInTheDocument();

  const sorted = await act(() =>
    window.jerryDev?.sort({ column: "size", descending: true, foldersFirst: true }),
  );
  expect(sorted?.status).toBe("ok");
  expect(screen.getByText(/^Sorted 2 items by size, descending: painted/)).toBeInTheDocument();

  act(() => {
    core.send(diffMessage([1, 2], [0], [{ name: "b.txt", size: 4096 }, { name: "ნახაზი.dwg" }]));
    core.send(orderMessage([2, 1], { column: 3, descending: true }));
  });
  expect(await screen.findByText(/^Live: 1 new, 1 changed, 1 removed/)).toBeInTheDocument();
  expect(screen.getByText("2 items")).toBeInTheDocument();
});

test("times quick filter keys and leaves the filter off", async () => {
  fakeListings({
    messages: () => [
      batchMessage(0, [{ name: "ნახაზი.dwg" }, { name: "report.pdf" }]),
      orderMessage([0, 1]),
      doneMessage(2, 1, 1),
    ],
  });
  render(<DevListing />);
  fireEvent.click(screen.getByRole("button", { name: "Filter test" }));
  expect(
    await screen.findByText("List a folder first, and wait until it is all in."),
  ).toBeInTheDocument();

  list(String.raw`C:\Work`);
  expect(await screen.findByText(/^2 items in 1 batch\./)).toBeInTheDocument();
  const filtered = await act(() => window.jerryDev?.filter());

  expect(filtered?.status).toBe("ok");
  expect(
    screen.getByText(/^Filtered 2 items, 24 keys typed: filter and render median/),
  ).toBeInTheDocument();
  expect(paneFolder("left").store.getState().filter).toBe("");
});
