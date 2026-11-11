import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import type { TransferJob, TransferList, TransferProgress, TransferState } from "../../ipc";
import { resetTransfers, syncTransfers, useTransfers } from "../../stores/transfers";
import { lastName, UnfinishedNotice } from "./UnfinishedNotice";

const MB = 1024 * 1024;

function progress(id: number, done = 1, total = 3): TransferProgress {
  return {
    id,
    phase: "working",
    file: String.raw`C:\Users\თენგო\პროექტი\ნახაზი.dwg`,
    filesDone: done,
    filesTotal: total,
    bytesDone: done * 8 * MB,
    bytesTotal: total * 8 * MB,
    speed: 0,
    failed: 0,
  };
}

function job(id: number, overrides: Partial<TransferJob> = {}): TransferJob {
  return {
    id,
    kind: "copy",
    state: { kind: "unfinished" },
    source: String.raw`C:\Users\თენგო\პროექტი`,
    sources: 3,
    destination: String.raw`D:\არქივი`,
    progress: progress(id),
    ...overrides,
  };
}

function shown(jobs: TransferJob[]) {
  useTransfers.setState({ list: { generation: 1, jobs } });
  return render(<UnfinishedNotice />);
}

/** A fake core that records `transfer_control` calls as `resume 2`, `cancel 3`. */
function fakeCore(listed: TransferList = { generation: 0, jobs: [] }): string[] {
  const calls: string[] = [];
  mockIPC(
    (cmd, args) => {
      if (cmd === "transfer_control") {
        const { action, job } = args as { action: string; job: number };
        calls.push(`${action} ${job}`);
      }
      return cmd === "transfer_list" ? listed : null;
    },
    { shouldMockEvents: true },
  );
  return calls;
}

afterEach(() => {
  resetTransfers();
});

test("nothing unfinished, no notice", () => {
  const running: TransferState = { kind: "running" };
  const { container } = shown([job(1, { state: running })]);
  expect(container).toBeEmptyDOMElement();
});

test("an unfinished job says what it was doing and how far it came", () => {
  shown([job(1)]);
  const notice = screen.getByRole("status");
  expect(notice).toHaveTextContent("Jerry closed before a transfer finished");
  expect(notice).toHaveTextContent("Copying 3 items to არქივი");
  expect(notice).toHaveTextContent("1 of 3 files · 8 MB of 24 MB");
  expect(screen.getByRole("button", { name: "Resume" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Discard" })).toBeInTheDocument();
});

test("one source is named, and a job that hadn't counted yet says so", () => {
  shown([
    job(1, { kind: "move", sources: 1, source: String.raw`C:\ნახაზი.dwg` }),
    job(2, { progress: progress(2, 0, 0) }),
  ]);
  const notice = screen.getByRole("status");
  expect(notice).toHaveTextContent("Jerry closed before 2 transfers finished");
  expect(notice).toHaveTextContent("Moving ნახაზი.dwg to არქივი");
  expect(notice).toHaveTextContent("Nothing done yet");
});

test("past three jobs the rest are counted", () => {
  shown([job(1), job(2), job(3), job(4), job(5)]);
  const items = within(screen.getByRole("list")).getAllByRole("listitem");
  expect(items).toHaveLength(4);
  expect(items[3]).toHaveTextContent("and 2 more");
});

test("Resume and Discard answer for every unfinished job, and only those", () => {
  const calls = fakeCore();
  const done: TransferState = { kind: "done" };
  shown([job(1), job(2, { state: done }), job(3)]);
  fireEvent.click(screen.getByRole("button", { name: "Resume" }));
  expect(calls).toEqual(["resume 1", "resume 3"]);
  fireEvent.click(screen.getByRole("button", { name: "Discard" }));
  expect(calls).toEqual(["resume 1", "resume 3", "cancel 1", "cancel 3"]);
});

test("the notice goes once the core says the jobs go on", async () => {
  fakeCore();
  const stop = syncTransfers();
  shown([job(1)]);
  const queued: TransferState = { kind: "queued" };
  await act(() => emit("transfer:changed", { generation: 2, jobs: [job(1, { state: queued })] }));
  expect(screen.queryByRole("status")).toBeNull();
  stop();
});

test("the store keeps the newest list and the running jobs' progress", async () => {
  fakeCore({ generation: 5, jobs: [job(1)] });
  const stop = syncTransfers();
  await waitFor(() => expect(useTransfers.getState().list.generation).toBe(5));
  await act(() => emit("transfer:changed", { generation: 4, jobs: [] }));
  expect(useTransfers.getState().list.jobs).toHaveLength(1);
  await act(() => emit("transfer:progress", [progress(1, 2, 3)]));
  expect(useTransfers.getState().list.jobs[0]?.progress.filesDone).toBe(2);
  stop();
});

test("a path's last part is its name, a drive stays whole", () => {
  expect(lastName(String.raw`D:\არქივი\2026`)).toBe("2026");
  expect(lastName("D:\\")).toBe("D:\\");
  expect(lastName(String.raw`\\nas\share`)).toBe("share");
  expect(lastName("\\\\nas\\share\\")).toBe("share");
});
