import { useMemo } from "react";
import type { TransferJob } from "../../ipc";
import {
  discardTransfers,
  resumeTransfers,
  unfinishedJobs,
  useTransfers,
} from "../../stores/transfers";
import { strings } from "../../strings";
import { Notice, NoticeActions, NoticeButton } from "../../ui/Notice";

const t = strings.transfers;

/** Jobs listed by name; the rest are counted. */
const SHOWN = 3;

/**
 * The notice about the transfers the Jerry before this one didn't finish (#50, PRD: Jerry closing
 * unexpectedly during a transfer): what each was doing and how far it came, Discard and Resume.
 * It stays until it is answered, and nothing goes on or goes away without the user. Resume
 * carries every listed job on from where it stopped; Discard ends them and keeps what they did.
 * Their half-written files go either way.
 */
export function UnfinishedNotice() {
  const list = useTransfers((state) => state.list);
  const jobs = useMemo(() => unfinishedJobs(list), [list]);
  if (jobs.length === 0) {
    return null;
  }
  const ids = jobs.map((job) => job.id);
  return (
    <Notice title={t.unfinished(jobs.length)}>
      <ul className="flex flex-col gap-1.5 text-small">
        {jobs.slice(0, SHOWN).map((job) => (
          <UnfinishedJob key={job.id} job={job} />
        ))}
        {jobs.length > SHOWN && <li className="text-fg-2">{t.more(jobs.length - SHOWN)}</li>}
      </ul>
      <NoticeActions>
        <NoticeButton onClick={() => discardTransfers(ids)}>{t.discard}</NoticeButton>
        <NoticeButton className="font-semibold" onClick={() => resumeTransfers(ids)}>
          {t.resume}
        </NoticeButton>
      </NoticeActions>
    </Notice>
  );
}

function UnfinishedJob({ job }: { job: TransferJob }) {
  const { progress } = job;
  const what = t.job(job.kind, job.sources, lastName(job.source), lastName(job.destination));
  const done =
    progress.filesTotal > 0
      ? t.done(progress.filesDone, progress.filesTotal, progress.bytesDone, progress.bytesTotal)
      : t.nothingDone;
  return (
    <li className="flex min-w-0 flex-col">
      <span className="truncate" title={`${job.source} → ${job.destination}`}>
        {what}
      </span>
      <span className="truncate text-fg-2 tabular-nums">{done}</span>
    </li>
  );
}

/** The last part of a display path: a file's or folder's name, or a drive as it is (`D:\`). */
export function lastName(path: string): string {
  const trimmed = path.replace(/\\+$/, "");
  const name = trimmed.slice(trimmed.lastIndexOf("\\") + 1);
  return name === "" || name.endsWith(":") ? path : name;
}
