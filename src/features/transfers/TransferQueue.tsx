import {
  type CSSProperties,
  type KeyboardEvent,
  lazy,
  memo,
  type PointerEvent,
  type ReactNode,
  Suspense,
} from "react";
import type { TransferEnded, TransferJob } from "../../ipc";
import { controlTransfer, reorderTransfers } from "../../stores/transfers";
import { strings } from "../../strings";
import { IconButton } from "../../ui/IconButton";
import { CancelIcon, ErrorIcon, GripIcon, PauseIcon, ResumeIcon } from "../../ui/icons";
import {
  canPause,
  canResume,
  jobRoute,
  jobTitle,
  measuring,
  type PanelItem,
  percentOf,
  placeOrder,
  queueDetail,
  summaryDetail,
  summaryTitle,
} from "./panel";
import { FailureList, KindIcon, PanelButton, ProgressBar, SummaryButtons } from "./parts";

const t = strings.transfers.panel;

/** dnd-kit, for reordering the queue: loaded once the queue opens (Tech PRD «წარმადობა»). */
const SortableQueue = lazy(() => import("./SortableQueue"));

/** What dragging adds to a queue line: dnd-kit's nodes, its moving style, the grip's keys and the press. */
export interface QueueDrag {
  ref: (node: HTMLElement | null) => void;
  /** The grip: it takes the keyboard (Space picks the line up), and focus comes back to it. */
  handleRef: (node: HTMLElement | null) => void;
  handle: {
    tabIndex: number;
    "aria-roledescription": string;
    "aria-describedby": string;
    "aria-pressed": boolean | undefined;
    onKeyDown: ((event: KeyboardEvent) => void) | undefined;
  };
  style: CSSProperties;
  onPointerDown: ((event: PointerEvent) => void) | undefined;
  dragging: boolean;
}

export type RenderJob = (job: TransferJob, drag?: QueueDrag) => ReactNode;

/**
 * The queue above the panel's row: how many transfers there are with Pause all, Resume all and
 * Cancel all, then the other jobs in queue order, each with its progress, Pause or Resume and
 * Cancel, reordered by drag (dnd-kit; files go by OLE), then the summaries of jobs that ended
 * with problems.
 */
export function TransferQueue({
  items,
  active,
  failuresOpen,
}: {
  /** The panel's other lines: jobs in queue order, then summaries. */
  items: readonly PanelItem[];
  /** Every job still to do, the panel row's among them, in queue order. */
  active: readonly TransferJob[];
  failuresOpen: number | null;
}) {
  const jobs = items.flatMap((item) => (item.kind === "job" ? [item.job] : []));
  const render: RenderJob = (job, drag) => <QueueJob job={job} drag={drag} />;
  // The row's job keeps its place among them.
  const order = (moved: number[]) =>
    reorderTransfers(
      placeOrder(
        active.map((job) => job.id),
        moved,
      ),
    );
  return (
    <div className="flex flex-col pb-1">
      {active.length > 0 && <QueueHeader active={active} />}
      {jobs.length > 0 && (
        <ul aria-label={t.queueLabel} className="flex flex-col">
          <Suspense fallback={jobs.map((job) => <QueueJob key={job.id} job={job} />)}>
            <SortableQueue jobs={jobs} render={render} onOrder={order} />
          </Suspense>
        </ul>
      )}
      {items.map(
        (item) =>
          item.kind === "summary" && (
            <QueueSummary
              key={item.id}
              summary={item.summary}
              job={item.job}
              open={failuresOpen === item.id}
            />
          ),
      )}
    </div>
  );
}

/** "4 transfers", and what applies to all of them. */
function QueueHeader({ active }: { active: readonly TransferJob[] }) {
  return (
    <div className="flex h-10 items-center gap-1.5 pr-3 pl-4">
      <span className="flex-1 truncate text-fg-2 text-small tabular-nums">
        {t.count(active.length)}
      </span>
      <PanelButton
        icon={<PauseIcon />}
        disabled={!active.some(canPause)}
        onClick={() => controlTransfer("pause", null)}
      >
        {t.pauseAll}
      </PanelButton>
      <PanelButton
        icon={<ResumeIcon />}
        disabled={!active.some(canResume)}
        onClick={() => controlTransfer("resume", null)}
      >
        {t.resumeAll}
      </PanelButton>
      <PanelButton icon={<CancelIcon />} onClick={() => controlTransfer("cancel", null)}>
        {t.cancelAll}
      </PanelButton>
    </div>
  );
}

/** The grip a queue line is dragged by, at its left. */
const GRIP =
  "inline-flex size-6 shrink-0 cursor-grab items-center justify-center rounded-ctrl text-fg-3";

/** One job in the queue: what goes where, where it is, its bar, Pause or Resume, Cancel. */
const QueueJob = memo(function QueueJob({ job, drag }: { job: TransferJob; drag?: QueueDrag }) {
  const percent = percentOf(job.progress);
  const sweeping = measuring(job);
  const title = jobTitle(job, false);
  const unfinished = job.state.kind === "unfinished";
  const resumes = !canPause(job);
  return (
    <li
      ref={drag?.ref}
      style={drag?.style}
      onPointerDown={drag?.onPointerDown}
      className={`flex h-11 items-center gap-2 pr-3 pl-1.5 ${drag?.dragging ? "relative z-10 rounded-ctrl bg-flyout shadow-flyout" : ""}`}
    >
      {drag ? (
        <button
          type="button"
          ref={drag.handleRef}
          {...drag.handle}
          aria-label={t.reorder(title)}
          className={GRIP}
        >
          <GripIcon />
        </button>
      ) : (
        <span aria-hidden="true" className={GRIP}>
          <GripIcon />
        </span>
      )}
      <span className="shrink-0 text-fg-2">
        <KindIcon job={job} size={16} />
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-px">
        <span className="truncate" title={jobRoute(job)}>
          {title}
        </span>
        <span className="truncate text-fg-2 text-small tabular-nums">{queueDetail(job)}</span>
      </div>
      <div className="flex w-40 shrink-0 flex-col gap-1">
        <span className="h-4 text-right text-fg-2 text-small tabular-nums">
          {!sweeping && percent !== null && t.percent(percent)}
        </span>
        <ProgressBar label={t.progress(job.kind)} percent={sweeping ? null : (percent ?? 0)} />
      </div>
      {resumes ? (
        <IconButton
          label={t.resume}
          title={t.resume}
          disabled={!canResume(job)}
          onClick={() => controlTransfer("resume", job.id)}
          className="size-7 rounded-ctrl"
        >
          <ResumeIcon />
        </IconButton>
      ) : (
        <IconButton
          label={t.pause}
          title={t.pause}
          onClick={() => controlTransfer("pause", job.id)}
          className="size-7 rounded-ctrl"
        >
          <PauseIcon />
        </IconButton>
      )}
      <IconButton
        label={unfinished ? t.discard : t.cancel}
        title={unfinished ? t.discard : t.cancel}
        onClick={() => controlTransfer("cancel", job.id)}
        className="size-7 rounded-ctrl"
      >
        <CancelIcon />
      </IconButton>
    </li>
  );
});

/** A job that ended with problems, with its buttons and, when open, its failed items. */
function QueueSummary({
  summary,
  job,
  open,
}: {
  summary: TransferEnded;
  job: TransferJob;
  open: boolean;
}) {
  return (
    <div className="flex flex-col">
      <div className="flex h-11 items-center gap-2 pr-3 pl-1.5">
        <span className="size-6 shrink-0" />
        <span className="shrink-0 text-red">
          <ErrorIcon size={16} />
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-px">
          <span className="truncate" title={jobRoute(job)}>
            {summaryTitle(summary, job)}
          </span>
          <span className="truncate text-fg-2 text-small">{summaryDetail(summary, job)}</span>
        </div>
        <SummaryButtons summary={summary} open={open} />
      </div>
      {open && <FailureList summary={summary} />}
    </div>
  );
}
