import { expect, test } from "vitest";
import type { TransferEnded, TransferJob, TransferProgress, TransferState } from "../../ipc";
import { strings } from "../../strings";
import {
  canPause,
  canResume,
  jobDetail,
  jobPace,
  jobTitle,
  overallPercent,
  panelView,
  percentOf,
  placeOrder,
  queueDetail,
  secondsLeft,
  summaryDetail,
  summaryTitle,
} from "./panel";

const MB = 1024 * 1024;

function progress(overrides: Partial<TransferProgress> = {}): TransferProgress {
  return {
    id: 1,
    phase: "working",
    file: String.raw`C:\Projects\BMW\ELV-010 CCTV Layout Rev 0.dwg`,
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
    progress: progress({ id }),
    ...overrides,
  };
}

const running: TransferState = { kind: "running" };
const queued: TransferState = { kind: "queued" };
const paused: TransferState = { kind: "paused" };
const done: TransferState = { kind: "done" };

function ended(id: number, overrides: Partial<TransferEnded> = {}): TransferEnded {
  return {
    id,
    state: done,
    files: 211,
    folders: 2,
    bytes: 90 * MB,
    skipped: 0,
    failed: [
      {
        source: String.raw`C:\Projects\BMW\Drawings\ნახაზი.dwg`,
        target: String.raw`\\INSTA-NAS01\Projects\BMW\_Archive\ნახაზი.dwg`,
        error: { code: "InUse", path: null, params: { app: null } },
        copied: false,
      },
    ],
    ...overrides,
  };
}

test("the row shows the running job, the queue the others, then summaries", () => {
  const list = {
    generation: 3,
    jobs: [job(1, queued), job(2, running), job(3, done), job(4, paused)],
  };
  const view = panelView(list, new Set([2]), [ended(3)]);
  expect(view.current).toMatchObject({ kind: "job", id: 2 });
  expect(view.queue.map((item) => `${item.kind} ${item.id}`)).toEqual([
    "job 1",
    "job 4",
    "summary 3",
  ]);
  expect(view.active.map((each) => each.id)).toEqual([1, 2, 4]);
  expect(view.due).toBe(true);
});

test("nothing running: the first job to do, then the oldest summary", () => {
  const two = { generation: 1, jobs: [job(1, paused), job(2, done)] };
  expect(panelView(two, new Set(), [ended(2)]).current).toMatchObject({ kind: "job", id: 1 });
  const none = { generation: 1, jobs: [job(1, done), job(2, done)] };
  const view = panelView(none, new Set(), [ended(2), ended(1)]);
  expect(view.current).toMatchObject({ kind: "summary", id: 2 });
  expect(view.queue.map((item) => item.id)).toEqual([1]);
});

test("the panel comes up for a job that lasted a second, or for a summary", () => {
  const list = { generation: 1, jobs: [job(1, running)] };
  expect(panelView(list, new Set(), []).due).toBe(false);
  expect(panelView(list, new Set([1]), []).due).toBe(true);
  // A summary heard before its job is listed as ended waits for that list.
  expect(panelView(list, new Set(), [ended(1)]).due).toBe(false);
  const over = { generation: 2, jobs: [job(1, done)] };
  expect(panelView(over, new Set(), [ended(1)]).due).toBe(true);
  expect(panelView(over, new Set([1]), []).due).toBe(false);
});

test("percent by bytes, by files without bytes, unknown before counting", () => {
  expect(percentOf(progress())).toBe(58);
  expect(percentOf(progress({ bytesTotal: 0, bytesDone: 0, filesDone: 1, filesTotal: 4 }))).toBe(
    25,
  );
  expect(percentOf(progress({ bytesTotal: 0, filesTotal: 0 }))).toBeNull();
  expect(
    overallPercent([job(1, running), job(2, queued, { progress: progress({ bytesDone: 0 }) })]),
  ).toBe(29);
});

test("time left at the current speed", () => {
  expect(secondsLeft(progress())).toBe(21);
  expect(secondsLeft(progress({ speed: 0 }))).toBeNull();
  const t = strings.transfers.panel;
  expect(t.timeLeft(21)).toBe("About 21 s left");
  expect(t.timeLeft(0.2)).toBe("About 1 s left");
  expect(t.timeLeft(170)).toBe("About 3 min left");
  expect(t.timeLeft(3600)).toBe("About 1 h left");
  expect(t.timeLeft(7500)).toBe("About 2 h 5 min left");
  expect(t.speed(42.6 * MB)).toBe("42.6 MB/s");
});

test("the row's lines follow the design", () => {
  const copying = job(1, running);
  expect(jobTitle(copying, true)).toBe(
    String.raw`Copying 214 items to \\INSTA-NAS01\Projects\BMW\_Archive`,
  );
  expect(jobTitle(copying, false)).toBe("Copying 214 items to _Archive");
  expect(jobDetail(copying)).toBe("ELV-010 CCTV Layout Rev 0.dwg · item 126 of 214");
  expect(jobPace(copying)).toEqual(["2 MB/s", "About 21 s left"]);
  expect(queueDetail(copying)).toBe("58 MB of 100 MB · 2 MB/s · About 21 s left");

  const counting = job(2, running, { progress: progress({ phase: "counting", speed: 0 }) });
  expect(jobDetail(counting)).toBe("Counting…");
  expect(jobPace(counting)).toEqual(["", ""]);

  const one = job(3, queued, { sources: 1, source: String.raw`C:\ნახაზი.dwg` });
  expect(jobTitle(one, false)).toBe("Copying ნახაზი.dwg to _Archive");
  expect(jobDetail(one)).toBe(String.raw`C:\ნახაზი.dwg → \\INSTA-NAS01\Projects\BMW\_Archive`);
  expect(queueDetail(one)).toBe("Queued · 125 of 214 files · 58 MB of 100 MB");
});

test("waiting, asking and unfinished jobs say so", () => {
  const gone: TransferState = {
    kind: "waiting",
    error: { code: "DeviceGone", path: null },
    ready: false,
  };
  expect(jobPace(job(1, gone))).toEqual(["Waiting for the drive", ""]);
  expect(jobPace(job(1, { ...gone, ready: true }))[0]).toBe("Ready to resume");
  expect(jobPace(job(1, { kind: "unfinished" }))[0]).toBe("Not finished");
  expect(jobDetail(job(1, { kind: "unfinished" }))).toBe("Jerry closed before it finished");
  const asking = job(1, {
    kind: "conflict",
    conflict: {
      source: String.raw`C:\a\ნახაზი.dwg`,
      target: String.raw`D:\b\ნახაზი.dwg`,
      incoming: { folder: false, size: 1, modified: null, icon: 2 },
      existing: { folder: false, size: 1, modified: null, icon: 2 },
      choices: ["replace", "skip"],
      noRecycleBin: false,
    },
  });
  expect(jobDetail(asking)).toBe("ნახაზი.dwg is already there");
  expect(jobPace(asking)[0]).toBe("Needs your answer");
});

test("which of Pause and Resume applies", () => {
  expect(canPause(job(1, running))).toBe(true);
  expect(canPause(job(1, queued))).toBe(true);
  expect(canResume(job(1, paused))).toBe(true);
  expect(canResume(job(1, { kind: "unfinished" }))).toBe(true);
  const error = { code: "DeviceGone", path: null } as const;
  expect(canResume(job(1, { kind: "waiting", error, ready: false }))).toBe(false);
  expect(canResume(job(1, { kind: "waiting", error, ready: true }))).toBe(true);
  expect(canPause(job(1, paused)) || canResume(job(1, running))).toBe(false);
});

test("a summary says how many items failed, or why the job couldn't start", () => {
  const copied = job(1, done);
  expect(summaryTitle(ended(1), copied)).toBe("1 file could not be copied");
  expect(summaryDetail(ended(1), copied)).toBe("211 files copied to _Archive");
  const three = ended(1, { failed: [...ended(1).failed, ...ended(1).failed, ...ended(1).failed] });
  expect(summaryTitle(three, { ...copied, kind: "move" })).toBe("3 files could not be moved");
  const notFound: TransferState = {
    kind: "failed",
    error: { code: "NotFound", path: null },
  };
  const failed = ended(1, { state: notFound, failed: [] });
  expect(summaryTitle(failed, copied)).toBe("Couldn't copy 214 items to _Archive");
  expect(summaryDetail(failed, copied)).toMatch(/couldn't be found/);
});

test("a drag reorders the queue's jobs among the places they held", () => {
  // Job 2 runs in the panel's row; the queue shows 1, 3 and 4, and 4 goes first.
  expect(placeOrder([1, 2, 3, 4], [4, 1, 3])).toEqual([4, 2, 1, 3]);
  expect(placeOrder([2, 1, 3], [3, 1])).toEqual([2, 3, 1]);
  expect(placeOrder([5], [])).toEqual([5]);
});
