import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import type {
  TransferEnded,
  TransferJob,
  TransferList,
  TransferProgress,
  TransferState,
} from "../../ipc";
import {
  LASTING_MS,
  reorderTransfers,
  resetTransfers,
  syncTransfers,
  useTransfers,
} from "../../stores/transfers";
import { TransferIndicator } from "./TransferIndicator";
import { TransferPanel } from "./TransferPanel";

const MB = 1024 * 1024;

function progress(id: number, overrides: Partial<TransferProgress> = {}): TransferProgress {
  return {
    id,
    phase: "working",
    file: String.raw`C:\Projects\BMW\Drawings\ELV-010 CCTV Layout Rev 0.dwg`,
    filesDone: 125,
    filesTotal: 214,
    bytesDone: 58 * MB,
    bytesTotal: 100 * MB,
    speed: 2 * MB,
    failed: 0,
    ...overrides,
  };
}

function job(id: number, state: TransferState, overrides: Partial<TransferJob> = {}): TransferJob {
  return {
    id,
    kind: "copy",
    state,
    source: String.raw`C:\Projects\BMW\Drawings`,
    sources: 214,
    destination: String.raw`\\INSTA-NAS01\Projects\BMW\_Archive`,
    progress: progress(id),
    ...overrides,
  };
}

const running: TransferState = { kind: "running" };
const queued: TransferState = { kind: "queued" };
const paused: TransferState = { kind: "paused" };
const done: TransferState = { kind: "done" };

/** A fake core that records what the panel asks of it: `pause 1`, `cancel all`, `reorder 3,1`. */
function fakeCore(listed: TransferList = { generation: 0, jobs: [] }): string[] {
  const calls: string[] = [];
  mockIPC(
    (cmd, args) => {
      const given = args as { action?: string; job?: number | null; order?: number[] };
      if (cmd === "transfer_control") {
        calls.push(`${given.action} ${given.job ?? "all"}`);
      } else if (cmd === "transfer_reorder") {
        calls.push(`reorder ${given.order?.join(",")}`);
      } else if (cmd === "transfer_retry_failed") {
        calls.push(`retry ${given.job}`);
      }
      return cmd === "transfer_list" ? listed : null;
    },
    { shouldMockEvents: true },
  );
  return calls;
}

/** The panel and the status bar's indicator, for `jobs` that have all gone on for a second. */
function shown(jobs: TransferJob[]) {
  useTransfers.setState({
    list: { generation: 1, jobs },
    lasting: new Set(jobs.map((each) => each.id)),
  });
  return render(
    <>
      <TransferPanel />
      <TransferIndicator />
    </>,
  );
}

const panel = () => screen.queryByRole("region", { name: "Transfers" });
const button = (name: string | RegExp) => screen.getByRole("button", { name });

let stop: (() => void) | undefined;

afterEach(() => {
  stop?.();
  stop = undefined;
  vi.useRealTimers();
  resetTransfers();
});

/** Follows the fake core's events, once it has listed its jobs. */
async function follow(listed: TransferList = { generation: 0, jobs: [] }) {
  stop = syncTransfers();
  await waitFor(() => expect(useTransfers.getState().list.generation).toBe(listed.generation));
}

test("a transfer that ends within a second never shows the panel", async () => {
  fakeCore();
  await follow();
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  render(<TransferPanel />);
  await act(() => emit("transfer:changed", { generation: 1, jobs: [job(1, running)] }));
  act(() => vi.advanceTimersByTime(LASTING_MS - 100));
  expect(panel()).toBeNull();
  await act(() => emit("transfer:changed", { generation: 2, jobs: [job(1, done)] }));
  act(() => vi.advanceTimersByTime(LASTING_MS));
  expect(panel()).toBeNull();
});

test("a transfer that goes on for a second shows the panel as the design has it", async () => {
  fakeCore();
  await follow();
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  render(<TransferPanel />);
  await act(() => emit("transfer:changed", { generation: 1, jobs: [job(1, running)] }));
  act(() => vi.advanceTimersByTime(LASTING_MS));
  const region = panel();
  expect(region).not.toBeNull();
  expect(region).toHaveTextContent(
    String.raw`Copying 214 items to \\INSTA-NAS01\Projects\BMW\_Archive`,
  );
  expect(region).toHaveTextContent("ELV-010 CCTV Layout Rev 0.dwg · item 126 of 214");
  expect(region).toHaveTextContent("58 MB of 100 MB");
  expect(region).toHaveTextContent("58%");
  expect(region).toHaveTextContent("2 MB/s");
  expect(region).toHaveTextContent("About 21 s left");
  expect(screen.getByRole("progressbar", { name: "Copy progress" })).toHaveAttribute(
    "aria-valuenow",
    "58",
  );
  expect(button("Pause")).toBeInTheDocument();
  expect(button("Cancel")).toBeInTheDocument();
  // One job: nothing waits behind it.
  expect(screen.queryByRole("button", { name: /queue/ })).toBeNull();
  // Progress keeps the row up to date.
  const later = progress(1, { bytesDone: 75 * MB, filesDone: 160 });
  await act(() => emit("transfer:progress", [later]));
  expect(panel()).toHaveTextContent("75%");
  expect(panel()).toHaveTextContent("item 161 of 214");
});

test("Pause, Resume and Cancel act on the job in the row", () => {
  const calls = fakeCore();
  const { rerender } = shown([job(1, running)]);
  fireEvent.click(button("Pause"));
  fireEvent.click(button("Cancel"));
  act(() => useTransfers.setState({ list: { generation: 2, jobs: [job(1, paused)] } }));
  rerender(<TransferPanel />);
  expect(panel()).toHaveTextContent("Paused");
  fireEvent.click(button("Resume"));
  expect(calls).toEqual(["pause 1", "cancel 1", "resume 1"]);
});

test("the queue opens with the other jobs, each with its own buttons", async () => {
  const calls = fakeCore();
  shown([
    job(1, queued),
    job(2, running),
    job(3, paused, { sources: 1, source: "C:\\ნახაზი.dwg" }),
  ]);
  // The running job is in the row, though it isn't first in the queue.
  expect(panel()).toHaveTextContent("item 126 of 214");
  const open = button("Show transfer queue, 2 waiting");
  expect(open).toHaveTextContent("Queue · 2");
  expect(open).toHaveAttribute("aria-expanded", "false");
  await act(async () => {
    fireEvent.click(open);
    await import("./SortableQueue");
  });
  const queue = screen.getByRole("list", { name: "Transfer queue" });
  const lines = within(queue).getAllByRole("listitem");
  expect(lines).toHaveLength(2);
  expect(lines[0]).toHaveTextContent("Copying 214 items to _Archive");
  expect(lines[0]).toHaveTextContent("Queued · 125 of 214 files · 58 MB of 100 MB");
  expect(lines[1]).toHaveTextContent("Copying ნახაზი.dwg to _Archive");
  expect(lines[1]).toHaveTextContent("Paused");
  expect(panel()).toHaveTextContent("3 transfers");

  fireEvent.click(within(lines[0] as HTMLElement).getByRole("button", { name: "Pause" }));
  fireEvent.click(within(lines[1] as HTMLElement).getByRole("button", { name: "Resume" }));
  fireEvent.click(within(lines[1] as HTMLElement).getByRole("button", { name: "Cancel" }));
  expect(calls).toEqual(["pause 1", "resume 3", "cancel 3"]);

  // Once dnd-kit is in, each line has a grip to drag it by, or to move it with the keys.
  await waitFor(() =>
    expect(
      within(lines[0] as HTMLElement).getByRole("button", {
        name: "Move Copying 214 items to _Archive in the queue",
      }),
    ).toHaveAttribute("aria-roledescription", "sortable"),
  );

  fireEvent.click(button("Hide transfer queue"));
  expect(screen.queryByRole("list", { name: "Transfer queue" })).toBeNull();
});

test("Pause all, Resume all and Cancel all act on every job", () => {
  const calls = fakeCore();
  shown([job(1, running), job(2, queued)]);
  fireEvent.click(button(/Show transfer queue/));
  fireEvent.click(button("Pause all"));
  fireEvent.click(button("Cancel all"));
  // Nothing is paused yet: there is nothing to resume.
  expect(button("Resume all")).toBeDisabled();
  expect(calls).toEqual(["pause all", "cancel all"]);
  act(() =>
    useTransfers.setState({ list: { generation: 2, jobs: [job(1, paused), job(2, paused)] } }),
  );
  expect(button("Pause all")).toBeDisabled();
  fireEvent.click(button("Resume all"));
  expect(calls).toEqual(["pause all", "cancel all", "resume all"]);
});

test("a reorder shows at once and goes to the core", async () => {
  const calls = fakeCore();
  shown([job(1, queued), job(2, running), job(3, queued)]);
  act(() => reorderTransfers([3, 2, 1]));
  expect(useTransfers.getState().list.jobs.map((each) => each.id)).toEqual([3, 2, 1]);
  await waitFor(() => expect(calls).toEqual(["reorder 3,2,1"]));
});

test("closed, the panel leaves the transfers going and the status bar shows them", async () => {
  const calls = fakeCore();
  await follow();
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  render(
    <>
      <TransferPanel />
      <TransferIndicator />
    </>,
  );
  await act(() => emit("transfer:changed", { generation: 1, jobs: [job(1, running)] }));
  act(() => vi.advanceTimersByTime(LASTING_MS));
  expect(screen.queryByRole("button", { name: /Show transfers/ })).toBeNull();
  // While it shows, the toasts in the corner know its height and stay above it.
  const height = () => document.documentElement.style.getPropertyValue("--jerry-transfer-panel-h");
  expect(height()).toMatch(/^\d+px$/);
  fireEvent.click(button("Hide transfers"));
  expect(panel()).toBeNull();
  expect(height()).toBe("");
  expect(calls).toEqual([]);
  const indicator = button("Show transfers: 58%");
  expect(indicator).toHaveTextContent("58%");

  // Progress goes on in the status bar.
  await act(() => emit("transfer:progress", [progress(1, { bytesDone: 80 * MB })]));
  expect(button("Show transfers: 80%")).toBeInTheDocument();
  fireEvent.click(button(/Show transfers/));
  expect(panel()).not.toBeNull();

  // Closed again, a new transfer brings it back.
  fireEvent.click(button("Hide transfers"));
  const two = { generation: 2, jobs: [job(1, running), job(2, queued)] };
  await act(() => emit("transfer:changed", two));
  expect(panel()).not.toBeNull();
});

test("a job that ended with failed items leaves a summary with the list and Retry", async () => {
  const calls = fakeCore();
  await follow();
  render(
    <>
      <TransferPanel />
      <TransferIndicator />
    </>,
  );
  const ended: TransferEnded = {
    id: 1,
    state: done,
    files: 211,
    folders: 2,
    bytes: 97 * MB,
    skipped: 0,
    failed: [
      {
        source: String.raw`C:\Projects\BMW\Drawings\ნახაზი.dwg`,
        target: String.raw`\\INSTA-NAS01\Projects\BMW\_Archive\ნახაზი.dwg`,
        error: { code: "InUse", path: null, params: { app: "AutoCAD" } },
        copied: false,
      },
      {
        source: String.raw`C:\Projects\BMW\Drawings\plan.pdf`,
        target: String.raw`\\INSTA-NAS01\Projects\BMW\_Archive\plan.pdf`,
        error: { code: "AccessDenied", path: null },
        copied: true,
      },
    ],
  };
  await act(() => emit("transfer:changed", { generation: 1, jobs: [job(1, running)] }));
  await act(() => emit("transfer:ended", ended));
  await act(() => emit("transfer:changed", { generation: 2, jobs: [job(1, done)] }));
  // It shows at once: the job is over, and this needs the user's eye.
  expect(panel()).toHaveTextContent("2 files could not be copied");
  expect(panel()).toHaveTextContent("211 files copied to _Archive");

  fireEvent.click(button("Show list"));
  const failures = screen.getByRole("list", { name: "Items that failed" });
  const items = within(failures).getAllByRole("listitem");
  expect(items[0]).toHaveTextContent("ნახაზი.dwg");
  expect(items[0]).toHaveTextContent("AutoCAD");
  expect(items[1]).toHaveTextContent("plan.pdf");
  expect(items[1]).toHaveTextContent("Copied, but the original stays.");
  expect(button("Hide list")).toHaveAttribute("aria-expanded", "true");

  fireEvent.click(button("Retry"));
  expect(calls).toEqual(["retry 1"]);
  expect(panel()).toBeNull();
});

test("a summary can be dismissed, and closed it shows in the status bar", async () => {
  fakeCore();
  await follow();
  render(
    <>
      <TransferPanel />
      <TransferIndicator />
    </>,
  );
  const failed: TransferState = {
    kind: "failed",
    error: { code: "NoSpace", path: null, params: { needed: 10 * MB, available: MB } },
  };
  await act(() => emit("transfer:changed", { generation: 1, jobs: [job(1, queued)] }));
  await act(() =>
    emit("transfer:ended", {
      id: 1,
      state: failed,
      files: 0,
      folders: 0,
      bytes: 0,
      skipped: 0,
      failed: [],
    }),
  );
  await act(() => emit("transfer:changed", { generation: 2, jobs: [job(1, failed)] }));
  expect(panel()).toHaveTextContent("Couldn't copy 214 items to _Archive");
  // Nothing failed one by one: there is no list to show.
  expect(screen.queryByRole("button", { name: "Show list" })).toBeNull();
  fireEvent.click(button("Hide transfers"));
  expect(button(/Show transfers: Couldn't copy/)).toBeInTheDocument();
  fireEvent.click(button(/Show transfers/));
  fireEvent.click(button("Dismiss"));
  expect(panel()).toBeNull();
  expect(screen.queryByRole("button", { name: /Show transfers/ })).toBeNull();
});

test("a job an earlier Jerry didn't finish offers Resume and Discard", () => {
  const calls = fakeCore();
  shown([job(1, { kind: "unfinished" })]);
  expect(panel()).toHaveTextContent("Jerry closed before it finished");
  expect(panel()).toHaveTextContent("Not finished");
  fireEvent.click(button("Resume"));
  fireEvent.click(button("Discard"));
  expect(calls).toEqual(["resume 1", "cancel 1"]);
});

test("a job waiting for its drive resumes only once the drive is back", () => {
  fakeCore();
  const waiting: TransferState = {
    kind: "waiting",
    error: { code: "DeviceGone", path: null },
    ready: false,
  };
  shown([job(1, waiting)]);
  expect(panel()).toHaveTextContent("Waiting for the drive");
  expect(button("Resume")).toBeDisabled();
  act(() =>
    useTransfers.setState({
      list: { generation: 2, jobs: [job(1, { ...waiting, ready: true })] },
    }),
  );
  expect(panel()).toHaveTextContent("Ready to resume");
  expect(button("Resume")).toBeEnabled();
});
