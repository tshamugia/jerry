import { memo, type ReactNode, useLayoutEffect, useMemo, useRef } from "react";
import type { TransferEnded, TransferJob } from "../../ipc";
import { controlTransfer, hideTransfers, toggleQueue, useTransfers } from "../../stores/transfers";
import { strings } from "../../strings";
import { IconButton } from "../../ui/IconButton";
import { CancelIcon, CloseIcon, ErrorIcon, PauseIcon, QueueIcon, ResumeIcon } from "../../ui/icons";
import {
  canPause,
  canResume,
  jobDetail,
  jobPace,
  jobRoute,
  jobTitle,
  measuring,
  type PanelItem,
  panelView,
  percentOf,
  summaryDetail,
  summaryTitle,
} from "./panel";
import { FailureList, KindIcon, PanelButton, ProgressBar, SummaryButtons } from "./parts";
import { TransferQueue } from "./TransferQueue";

const t = strings.transfers.panel;

/**
 * The transfer panel (#52, design: «Transfer panel», 56 px on `bar`): it comes up once a transfer
 * has gone on for a second, or one ended with items left behind. Its row shows the running job:
 * what goes where, the current file and "item N of M", bytes, percent and the bar, speed and time
 * left, Pause or Resume and Cancel. "Queue · N" opens the other jobs above it, with Pause all,
 * Resume all and Cancel all, reordered by drag; ✕ closes it while the transfers go on, and the
 * status bar shows them. A job that ended with failed items leaves a summary: how many, the list,
 * Retry.
 */
export function TransferPanel() {
  const list = useTransfers((state) => state.list);
  const lasting = useTransfers((state) => state.lasting);
  const summaries = useTransfers((state) => state.summaries);
  const hidden = useTransfers((state) => state.hidden);
  const queueOpen = useTransfers((state) => state.queueOpen);
  const failuresOpen = useTransfers((state) => state.failuresOpen);
  const view = useMemo(() => panelView(list, lasting, summaries), [list, lasting, summaries]);
  const { current, queue, active } = view;
  if (hidden || !view.due || !current) {
    return null;
  }
  const showQueue = queueOpen && queue.length > 0;
  const showFailures = current.kind === "summary" && failuresOpen === current.id;
  return (
    <PanelSection>
      {(showQueue || showFailures) && (
        <div className="max-h-transfer-list overflow-y-auto border-stroke border-b">
          {showQueue && <TransferQueue items={queue} active={active} failuresOpen={failuresOpen} />}
          {current.kind === "summary" && showFailures && <FailureList summary={current.summary} />}
        </div>
      )}
      <div className="flex h-transfer items-center gap-5 pr-3 pl-4">
        <CurrentRow item={current} />
        <div className="flex shrink-0 items-center gap-1.5">
          {current.kind === "job" ? (
            <JobButtons job={current.job} />
          ) : (
            <SummaryButtons summary={current.summary} open={showFailures} />
          )}
          <span aria-hidden="true" className="mx-1 h-5 w-px bg-stroke" />
          {queue.length > 0 && (
            <button
              type="button"
              aria-expanded={queueOpen}
              aria-label={queueOpen ? t.hideQueue : t.showQueue(queue.length)}
              onClick={toggleQueue}
              className="inline-flex h-7 items-center gap-1.5 rounded-ctrl px-2 text-fg-2 text-small tabular-nums hover:bg-hover"
            >
              {t.queue(queue.length)}
              <QueueIcon open={queueOpen} />
            </button>
          )}
          <IconButton
            label={t.hide}
            title={t.hide}
            onClick={hideTransfers}
            className="size-7 rounded-ctrl text-fg-2"
          >
            <CloseIcon />
          </IconButton>
        </div>
      </div>
    </PanelSection>
  );
}

/** The CSS variable on `<html>` with the panel's height: the toasts stay above it. */
const PANEL_HEIGHT = "--jerry-transfer-panel-h";

/**
 * The panel's region. While it shows, `<html>` carries its height (`PANEL_HEIGHT`), so the
 * toasts and notices in the corner sit above it and its queue rather than over their buttons.
 */
function PanelSection({ children }: { children: ReactNode }) {
  const ref = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const node = ref.current;
    const root = document.documentElement;
    if (!node) {
      return;
    }
    const measure = () => root.style.setProperty(PANEL_HEIGHT, `${node.offsetHeight}px`);
    measure();
    // jsdom has no ResizeObserver.
    const observer =
      typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(measure);
    observer?.observe(node);
    return () => {
      observer?.disconnect();
      root.style.removeProperty(PANEL_HEIGHT);
    };
  }, []);
  return (
    <section
      ref={ref}
      aria-label={strings.frame.transfers}
      className="shrink-0 border-stroke border-t bg-bar"
    >
      {children}
    </section>
  );
}

function CurrentRow({ item }: { item: PanelItem }) {
  return item.kind === "job" ? (
    <JobRow job={item.job} />
  ) : (
    <SummaryRow summary={item.summary} job={item.job} />
  );
}

/** The running job: what goes where and the current file, its progress, its pace. */
const JobRow = memo(function JobRow({ job }: { job: TransferJob }) {
  const { progress } = job;
  const percent = percentOf(progress);
  const sweeping = measuring(job);
  const [pace, left] = jobPace(job);
  return (
    <>
      <div className="flex w-transfer-what min-w-0 items-center gap-3">
        <span className="shrink-0 text-accent">
          <KindIcon job={job} size={20} />
        </span>
        <div className="flex min-w-0 flex-col gap-px">
          <span className="truncate font-semibold" title={jobRoute(job)}>
            {jobTitle(job, true)}
          </span>
          <span className="truncate text-fg-2 text-small" title={progress.file ?? undefined}>
            {jobDetail(job)}
          </span>
        </div>
      </div>
      <div className="flex min-w-24 flex-1 flex-col gap-1.5">
        <div className="flex justify-between gap-2 text-fg-2 text-small tabular-nums">
          <span className="truncate">
            {!sweeping &&
              progress.bytesTotal > 0 &&
              t.bytes(progress.bytesDone, progress.bytesTotal)}
          </span>
          <span>{!sweeping && percent !== null && t.percent(percent)}</span>
        </div>
        <ProgressBar label={t.progress(job.kind)} percent={sweeping ? null : (percent ?? 0)} />
      </div>
      <div className="flex w-28 shrink-0 flex-col gap-px text-small tabular-nums">
        <span className="truncate">{pace}</span>
        <span className="truncate text-fg-2">{left}</span>
      </div>
    </>
  );
});

/** A job that ended with problems: how many items failed, or why it couldn't start. */
function SummaryRow({ summary, job }: { summary: TransferEnded; job: TransferJob }) {
  return (
    <div className="flex min-w-0 flex-1 items-center gap-3">
      <span className="shrink-0 text-red">
        <ErrorIcon />
      </span>
      <div className="flex min-w-0 flex-col gap-px">
        <span className="truncate font-semibold" title={jobRoute(job)}>
          {summaryTitle(summary, job)}
        </span>
        <span className="truncate text-fg-2 text-small">{summaryDetail(summary, job)}</span>
      </div>
    </div>
  );
}

/** Pause or Resume, and Cancel; Resume and Discard for a job an earlier Jerry didn't finish. */
function JobButtons({ job }: { job: TransferJob }) {
  if (job.state.kind === "unfinished") {
    return (
      <>
        <PanelButton icon={<ResumeIcon />} onClick={() => controlTransfer("resume", job.id)}>
          {t.resume}
        </PanelButton>
        <PanelButton icon={<CancelIcon />} onClick={() => controlTransfer("cancel", job.id)}>
          {t.discard}
        </PanelButton>
      </>
    );
  }
  const resumes = !canPause(job);
  return (
    <>
      {resumes ? (
        <PanelButton
          icon={<ResumeIcon />}
          disabled={!canResume(job)}
          onClick={() => controlTransfer("resume", job.id)}
        >
          {t.resume}
        </PanelButton>
      ) : (
        <PanelButton icon={<PauseIcon />} onClick={() => controlTransfer("pause", job.id)}>
          {t.pause}
        </PanelButton>
      )}
      <PanelButton icon={<CancelIcon />} onClick={() => controlTransfer("cancel", job.id)}>
        {t.cancel}
      </PanelButton>
    </>
  );
}
