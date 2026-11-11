import type { ButtonHTMLAttributes, ReactNode } from "react";
import type { TransferEnded, TransferJob } from "../../ipc";
import { dismissSummary, retryTransfer, toggleFailures } from "../../stores/transfers";
import { errorText, strings } from "../../strings";
import { CancelIcon, CopyIcon, MoveIcon } from "../../ui/icons";
import { lastName } from "./UnfinishedNotice";

const t = strings.transfers.panel;

/** Failed items listed before the rest are counted. */
const FAILURES_SHOWN = 200;

/** A copy or a move. */
export function KindIcon({ job, size }: { job: TransferJob; size: 16 | 20 }) {
  return job.kind === "copy" ? <CopyIcon size={size} /> : <MoveIcon size={size} />;
}

/** The 4 px accent bar on `track`; it sweeps while the size isn't known (`percent` null). */
export function ProgressBar({ label, percent }: { label: string; percent: number | null }) {
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent ?? undefined}
      className="h-1 overflow-hidden rounded-bar bg-track"
    >
      {percent === null ? (
        <div className="h-full w-1/3 animate-loading rounded-bar bg-accent motion-reduce:animate-none" />
      ) : (
        <div className="h-full rounded-bar bg-accent" style={{ width: `${percent}%` }} />
      )}
    </div>
  );
}

/** The panel's 28 px button (design: `ctrl` fill and border, 12 px text, an icon before it). */
export function PanelButton({
  icon,
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { icon?: ReactNode }) {
  return (
    <button
      type="button"
      className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-ctrl border border-ctrl-border bg-ctrl px-2.5 text-small enabled:hover:bg-hover disabled:text-disabled"
      {...rest}
    >
      {icon}
      {children}
    </button>
  );
}

/** A summary's buttons: Show list (when items failed), Retry and Dismiss. */
export function SummaryButtons({ summary, open }: { summary: TransferEnded; open: boolean }) {
  return (
    <>
      {summary.failed.length > 0 && (
        <PanelButton aria-expanded={open} onClick={() => toggleFailures(summary.id)}>
          {open ? t.hideFailures : t.showFailures}
        </PanelButton>
      )}
      <PanelButton onClick={() => retryTransfer(summary.id)}>{t.retry}</PanelButton>
      <PanelButton icon={<CancelIcon />} onClick={() => dismissSummary(summary.id)}>
        {t.dismiss}
      </PanelButton>
    </>
  );
}

/** The items a job couldn't copy or move, each with why. */
export function FailureList({ summary }: { summary: TransferEnded }) {
  const { failed } = summary;
  return (
    <ul aria-label={t.failures} className="flex flex-col py-1 text-small">
      {failed.slice(0, FAILURES_SHOWN).map((failure) => {
        const why = failure.copied
          ? `${t.copiedOnly} ${errorText(failure.error)}`
          : errorText(failure.error);
        return (
          <li
            key={`${failure.source}\n${failure.target}`}
            className="flex h-6 items-center gap-3 pr-3 pl-12"
          >
            <span className="w-60 shrink-0 truncate" title={failure.source}>
              {lastName(failure.source)}
            </span>
            <span className="min-w-0 flex-1 truncate text-fg-2" title={why}>
              {why}
            </span>
          </li>
        );
      })}
      {failed.length > FAILURES_SHOWN && (
        <li className="flex h-6 items-center pl-12 text-fg-2">
          {strings.transfers.more(failed.length - FAILURES_SHOWN)}
        </li>
      )}
    </ul>
  );
}
