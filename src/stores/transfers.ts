import { create } from "zustand";
import {
  type ConflictChoice,
  commands,
  events,
  subscribe,
  type TransferConflict,
  type TransferJob,
  type TransferList,
} from "../ipc";

interface TransfersState {
  /** The queue as the core last listed it: the list with the highest generation, its running jobs' progress kept up. */
  list: TransferList;
}

const empty: TransferList = { generation: -1, jobs: [] };

/** The transfer queue (#49). The core owns it; this window only shows it and asks for changes. */
export const useTransfers = create<TransfersState>(() => ({ list: empty }));

/** Takes `list` unless the one kept is newer. */
function keep(list: TransferList): void {
  useTransfers.setState((state) => (list.generation > state.list.generation ? { list } : state));
}

/**
 * Loads the queue and follows `transfer:changed` and `transfer:progress` until the returned
 * function is called.
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
  return () => {
    stopChanges();
    stopProgress();
  };
}

/** The jobs a Jerry before this one didn't finish (#50), in queue order. */
export function unfinishedJobs(list: TransferList): TransferJob[] {
  return list.jobs.filter((job) => job.state.kind === "unfinished");
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
