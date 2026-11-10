import { create } from "zustand";
import { commands, events, subscribe, type TransferJob, type TransferList } from "../ipc";

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
