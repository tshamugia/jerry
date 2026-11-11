import { create } from "zustand";
import {
  type ConflictChoice,
  commands,
  events,
  subscribe,
  type TransferAction,
  type TransferConflict,
  type TransferEnded,
  type TransferJob,
  type TransferList,
} from "../ipc";

interface TransfersState {
  /** The queue as the core last listed it: the list with the highest generation, its running jobs' progress kept up. */
  list: TransferList;
  /**
   * The jobs that haven't ended and that this window has seen for `LASTING_MS` or longer. One of
   * them brings the transfer panel up (PRD: an operation that runs longer than 1 s).
   */
  lasting: ReadonlySet<number>;
  /**
   * Jobs this window heard end with items left behind, or that couldn't start: the panel's
   * summary ("3 files could not be copied", the list, Retry), oldest first, until dismissed or
   * tried again.
   */
  summaries: readonly TransferEnded[];
  /** The user closed the panel: the transfers go on, and the status bar shows them. */
  hidden: boolean;
  /** The queue above the panel is open. */
  queueOpen: boolean;
  /** The summary whose failed items are listed. */
  failuresOpen: number | null;
}

/** How long a job runs before the panel comes up (PRD). */
export const LASTING_MS = 1000;

const empty: TransferList = { generation: -1, jobs: [] };

const initial: TransfersState = {
  list: empty,
  lasting: new Set(),
  summaries: [],
  hidden: false,
  queueOpen: false,
  failuresOpen: null,
};

/** The transfer queue (#49) and its panel (#52). The core owns the queue; this window only shows it and asks for changes. */
export const useTransfers = create<TransfersState>(() => initial);

/** Every job id this window has listed: a new one brings a closed panel back. */
const seen = new Set<number>();

/** A timer per job that hasn't ended yet and hasn't lasted `LASTING_MS`. */
const timers = new Map<number, ReturnType<typeof setTimeout>>();

/** Whether `job` is still to do: running, waiting for its turn, the user or a drive, or unfinished. */
export function isActive(job: TransferJob): boolean {
  const { kind } = job.state;
  return kind !== "done" && kind !== "cancelled" && kind !== "failed";
}

/** Takes `list` unless the one kept is newer. */
function keep(list: TransferList): void {
  if (list.generation > useTransfers.getState().list.generation) {
    useTransfers.setState((state) => ({ list, ...follow(state, list) }));
  }
}

/**
 * What changes with a new list: which jobs have lasted, which summaries still stand, and the
 * panel opens again for a job it hasn't shown. Starts the timer of each job that just began.
 */
function follow(state: TransfersState, list: TransferList): Partial<TransfersState> {
  const active = new Set(list.jobs.filter(isActive).map((job) => job.id));
  let fresh = false;
  for (const job of list.jobs) {
    if (!seen.has(job.id)) {
      seen.add(job.id);
      fresh = true;
    }
  }
  for (const [id, timer] of timers) {
    if (!active.has(id)) {
      clearTimeout(timer);
      timers.delete(id);
    }
  }
  const lasting = new Set([...state.lasting].filter((id) => active.has(id)));
  for (const id of active) {
    if (!lasting.has(id) && !timers.has(id)) {
      timers.set(
        id,
        setTimeout(() => lasted(id), LASTING_MS),
      );
    }
  }
  // A summary goes once its job is tried again, or once the core forgets the job.
  const listed = new Map(list.jobs.map((job) => [job.id, job]));
  const summaries = state.summaries.filter((summary) => {
    const job = listed.get(summary.id);
    return job !== undefined && !isActive(job);
  });
  const nothingLeft = active.size === 0 && summaries.length === 0;
  return {
    lasting,
    summaries: summaries.length === state.summaries.length ? state.summaries : summaries,
    hidden: fresh || nothingLeft ? false : state.hidden,
    queueOpen: nothingLeft ? false : state.queueOpen,
    failuresOpen: summaries.some((summary) => summary.id === state.failuresOpen)
      ? state.failuresOpen
      : null,
  };
}

/** Job `id` has gone on for `LASTING_MS`: if it hasn't ended, the panel shows. */
function lasted(id: number): void {
  timers.delete(id);
  useTransfers.setState((state) => {
    const job = state.list.jobs.find((each) => each.id === id);
    return job && isActive(job) ? { lasting: new Set([...state.lasting, id]) } : state;
  });
}

/** A job that ended and needs the user's eye: items left behind, or it couldn't start. */
function summarizes(ended: TransferEnded): boolean {
  return ended.state.kind === "failed" || (ended.state.kind === "done" && ended.failed.length > 0);
}

/**
 * Loads the queue and follows `transfer:changed`, `transfer:progress` and `transfer:ended` until
 * the returned function is called.
 */
export function syncTransfers(): () => void {
  const stopChanges = subscribe(events.transferChanged, keep, () => {
    void commands.transferList().then((listed) => {
      if (listed.status === "ok") {
        keep(listed.data);
      }
    });
  });
  const stopProgress = subscribe(events.transferProgress, (running) => {
    const byId = new Map(running.map((progress) => [progress.id, progress]));
    useTransfers.setState((state) => ({
      list: {
        ...state.list,
        jobs: state.list.jobs.map((job) => {
          const progress = byId.get(job.id);
          return progress ? { ...job, progress } : job;
        }),
      },
    }));
  });
  // The core tells of a job's end before it lists the job as ended.
  const stopEnded = subscribe(events.transferEnded, (ended) => {
    if (summarizes(ended)) {
      useTransfers.setState((state) => ({
        summaries: [...state.summaries.filter((summary) => summary.id !== ended.id), ended],
      }));
    }
  });
  return () => {
    stopChanges();
    stopProgress();
    stopEnded();
  };
}

/** Back to nothing listed, with no timers left (tests). */
export function resetTransfers(): void {
  for (const timer of timers.values()) {
    clearTimeout(timer);
  }
  timers.clear();
  seen.clear();
  useTransfers.setState(initial, true);
}

/** The jobs a Jerry before this one didn't finish (#50), in queue order. */
export function unfinishedJobs(list: TransferList): TransferJob[] {
  return list.jobs.filter((job) => job.state.kind === "unfinished");
}

/**
 * The transfers that stop if Jerry closes now: running, paused, queued, asking or waiting for a
 * drive. Unfinished ones have stopped already.
 */
export function runningJobs(list: TransferList): TransferJob[] {
  return list.jobs.filter((job) => isActive(job) && job.state.kind !== "unfinished");
}

/** Carries on `jobs` from where they stopped. */
export function resumeTransfers(jobs: readonly number[]): void {
  for (const job of jobs) {
    void commands.transferControl("resume", job);
  }
}

/** Ends `jobs` as they are: what they did stays, their half-written files go. */
export function discardTransfers(jobs: readonly number[]): void {
  for (const job of jobs) {
    void commands.transferControl("cancel", job);
  }
}

/**
 * Pauses, resumes or cancels job `job`, or every job with `null`. What doesn't apply where a job
 * is does nothing; the core lists what came of it.
 */
export function controlTransfer(action: TransferAction, job: number | null): void {
  void commands.transferControl(action, job);
}

/**
 * Puts the jobs of `order` first, in that order; the others follow as they were. The list
 * changes here at once, so a dragged row stays where it was dropped until the core's list comes.
 */
export function reorderTransfers(order: readonly number[]): void {
  const rank = new Map(order.map((id, index) => [id, index]));
  const last = order.length;
  useTransfers.setState((state) => ({
    list: {
      ...state.list,
      jobs: [...state.list.jobs].sort(
        (a, b) => (rank.get(a.id) ?? last) - (rank.get(b.id) ?? last),
      ),
    },
  }));
  void commands.transferReorder([...order]);
}

/** Tries job `job`'s failed items again, or the whole job if it couldn't start. */
export function retryTransfer(job: number): void {
  dismissSummary(job);
  void commands.transferRetryFailed(job);
}

/** The user has seen job `job`'s summary. */
export function dismissSummary(job: number): void {
  useTransfers.setState((state) => ({
    summaries: state.summaries.filter((summary) => summary.id !== job),
    failuresOpen: state.failuresOpen === job ? null : state.failuresOpen,
  }));
}

/** Closes the panel: the transfers go on, and the status bar shows them. */
export function hideTransfers(): void {
  useTransfers.setState({ hidden: true });
}

/** Opens the panel again (the status bar's indicator). */
export function showTransfers(): void {
  useTransfers.setState({ hidden: false });
}

/** Opens or closes the queue above the panel. */
export function toggleQueue(): void {
  useTransfers.setState((state) => ({ queueOpen: !state.queueOpen }));
}

/** Lists job `job`'s failed items, or hides them. */
export function toggleFailures(job: number): void {
  useTransfers.setState((state) => ({ failuresOpen: state.failuresOpen === job ? null : job }));
}

/** A job waiting for an answer to a conflict (#51), and the conflict. */
export interface Asking {
  job: TransferJob;
  conflict: TransferConflict;
}

/** The first job in queue order that waits for an answer to a conflict, if one does. */
export function firstConflict(list: TransferList): Asking | undefined {
  for (const job of list.jobs) {
    if (job.state.kind === "conflict") {
      return { job, conflict: job.state.conflict };
    }
  }
  return undefined;
}

/**
 * Answers the conflict job `job` waits on; with `all`, every later conflict of its kind in the
 * job gets the same answer. If another window answered first, nothing happens.
 */
export function resolveConflict(job: number, choice: ConflictChoice, all: boolean): void {
  void commands.conflictResolve(job, { choice, all });
}
