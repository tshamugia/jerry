import type { TransferEnded, TransferJob, TransferList, TransferProgress } from "../../ipc";
import { isActive } from "../../stores/transfers";
import { errorText, strings } from "../../strings";
import { lastName } from "./UnfinishedNotice";

const t = strings.transfers;

/** A line of the transfer panel: a job, or the summary of one that ended with problems. */
export type PanelItem =
  | { kind: "job"; id: number; job: TransferJob }
  | { kind: "summary"; id: number; summary: TransferEnded; job: TransferJob };

export interface PanelView {
  /** What the 56 px row shows: the first running job, else the first still to do, else the oldest summary. */
  current: PanelItem | undefined;
  /** The others, for the queue above the row: jobs in queue order, then summaries. */
  queue: PanelItem[];
  /** Jobs still to do, in queue order. */
  active: TransferJob[];
  /** Whether the panel comes up: a job has gone on for a second, or one ended with problems. */
  due: boolean;
}

/** What the panel shows of `list`, given the jobs that have lasted and the summaries kept. */
export function panelView(
  list: TransferList,
  lasting: ReadonlySet<number>,
  summaries: readonly TransferEnded[],
): PanelView {
  const active = list.jobs.filter(isActive);
  const listed = new Map(list.jobs.map((job) => [job.id, job]));
  const jobs: PanelItem[] = active.map((job) => ({ kind: "job", id: job.id, job }));
  const ended: PanelItem[] = [];
  for (const summary of summaries) {
    const job = listed.get(summary.id);
    // Until the core lists it as ended, the job shows as it was.
    if (job && !isActive(job)) {
      ended.push({ kind: "summary", id: summary.id, summary, job });
    }
  }
  const items = [...jobs, ...ended];
  const current =
    jobs.find((item) => item.kind === "job" && item.job.state.kind === "running") ?? items[0];
  return {
    current,
    queue: items.filter((item) => item !== current),
    active,
    due: ended.length > 0 || active.some((job) => lasting.has(job.id)),
  };
}

/** How far a job has come, 0 to 100, by bytes, or by files where there are no bytes; null before it knows. */
export function percentOf(progress: TransferProgress): number | null {
  const { bytesDone, bytesTotal, filesDone, filesTotal } = progress;
  if (bytesTotal > 0) {
    return Math.min(100, Math.floor((bytesDone * 100) / bytesTotal));
  }
  if (filesTotal > 0) {
    return Math.min(100, Math.floor((filesDone * 100) / filesTotal));
  }
  return null;
}

/** Seconds left at the speed of the last few seconds; null while the speed isn't known. */
export function secondsLeft(progress: TransferProgress): number | null {
  const { bytesDone, bytesTotal, speed } = progress;
  if (speed <= 0 || bytesTotal <= bytesDone) {
    return null;
  }
  return (bytesTotal - bytesDone) / speed;
}

/** All of `jobs` together, 0 to 100; null while none knows its size. */
export function overallPercent(jobs: readonly TransferJob[]): number | null {
  let done = 0;
  let total = 0;
  for (const { progress } of jobs) {
    done += progress.bytesDone;
    total += progress.bytesTotal;
  }
  return total > 0 ? Math.min(100, Math.floor((done * 100) / total)) : null;
}

/**
 * "Copying 214 items to D:\Projects\Archive": the destination as a whole with `full` (the
 * panel's row), its name otherwise (the queue).
 */
export function jobTitle(job: TransferJob, full: boolean): string {
  const destination = full ? job.destination : lastName(job.destination);
  return t.job(job.kind, job.sources, lastName(job.source), destination);
}

/** Where a job goes from and to, for a tooltip: "C:\Projects\BMW → D:\Archive". */
export function jobRoute(job: TransferJob): string {
  return `${job.source} → ${job.destination}`;
}

/**
 * The row's second line: the file being worked on and "item N of M", or what the job is doing or
 * waiting for.
 */
export function jobDetail(job: TransferJob): string {
  const { state, progress } = job;
  switch (state.kind) {
    case "conflict":
      return t.panel.askingDetail(lastName(state.conflict.target));
    case "waiting":
      return errorText(state.error);
    case "unfinished":
      return t.panel.unfinishedDetail;
    case "queued":
      return jobRoute(job);
    default:
      break;
  }
  switch (progress.phase) {
    case "counting":
      return t.panel.counting;
    case "checkingSpace":
      return t.panel.checkingSpace;
    case "working": {
      if (progress.file === null) {
        return jobRoute(job);
      }
      const item = Math.min(progress.filesDone + 1, progress.filesTotal);
      return t.panel.item(lastName(progress.file), item, progress.filesTotal);
    }
  }
}

/** The right column's two lines: speed and time left while it goes, else where it is. */
export function jobPace(job: TransferJob): [string, string] {
  const { state, progress } = job;
  switch (state.kind) {
    case "running": {
      if (progress.phase !== "working" || progress.speed <= 0) {
        return ["", ""];
      }
      const left = secondsLeft(progress);
      return [t.panel.speed(progress.speed), left === null ? "" : t.panel.timeLeft(left)];
    }
    case "paused":
      return [t.panel.paused, ""];
    case "queued":
      return [t.panel.queued, ""];
    case "conflict":
      return [t.panel.asking, ""];
    case "waiting":
      return [state.ready ? t.panel.ready : t.panel.waiting, ""];
    case "unfinished":
      return [t.panel.unfinished, ""];
    default:
      return ["", ""];
  }
}

/** The queue line's second line: where the job is and how far it came. */
export function queueDetail(job: TransferJob): string {
  const { state, progress } = job;
  const [pace, left] = jobPace(job);
  const parts: string[] = [];
  if (state.kind === "running") {
    if (progress.phase !== "working") {
      return jobDetail(job);
    }
    parts.push(t.panel.bytes(progress.bytesDone, progress.bytesTotal), pace, left);
  } else {
    parts.push(pace);
    if (progress.filesTotal > 0) {
      parts.push(
        t.done(progress.filesDone, progress.filesTotal, progress.bytesDone, progress.bytesTotal),
      );
    }
  }
  return parts.filter((part) => part !== "").join(" · ");
}

/** The sweeping bar: a running job that doesn't know its size yet. */
export function measuring(job: TransferJob): boolean {
  return job.state.kind === "running" && job.progress.phase !== "working";
}

/** Pause applies: it runs, waits for its turn, or asks. */
export function canPause(job: TransferJob): boolean {
  const { kind } = job.state;
  return kind === "running" || kind === "queued" || kind === "conflict";
}

/** Resume applies: paused, back from a drive that went away, or not finished by an earlier Jerry. */
export function canResume(job: TransferJob): boolean {
  const { state } = job;
  return (
    state.kind === "paused" ||
    state.kind === "unfinished" ||
    (state.kind === "waiting" && state.ready)
  );
}

/** A summary's bold line: "3 files could not be copied", or "Couldn't copy 3 items to Archive". */
export function summaryTitle(summary: TransferEnded, job: TransferJob): string {
  if (summary.state.kind === "failed") {
    return t.panel.couldntStart(
      job.kind,
      job.sources,
      lastName(job.source),
      lastName(job.destination),
    );
  }
  return t.panel.failed(summary.failed.length, job.kind);
}

/** Its second line: why it couldn't start, or what made it. */
export function summaryDetail(summary: TransferEnded, job: TransferJob): string {
  if (summary.state.kind === "failed") {
    return errorText(summary.state.error);
  }
  return t.panel.finished(summary.files, job.kind, lastName(job.destination));
}

/**
 * `ids` (the queue order) with the jobs of `moved` in their new order, in the places they held:
 * the jobs that weren't dragged among stay where they were.
 */
export function placeOrder(ids: readonly number[], moved: readonly number[]): number[] {
  const among = new Set(moved);
  let next = 0;
  return ids.map((id) => {
    if (!among.has(id)) {
      return id;
    }
    const placed = moved[next] ?? id;
    next += 1;
    return placed;
  });
}
