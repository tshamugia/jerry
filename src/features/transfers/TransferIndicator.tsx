import { type ReactNode, useMemo } from "react";
import { showTransfers, useTransfers } from "../../stores/transfers";
import { strings } from "../../strings";
import { ErrorIcon } from "../../ui/icons";
import { overallPercent, panelView, summaryTitle } from "./panel";
import { KindIcon } from "./parts";

const t = strings.transfers.panel;

/**
 * The status bar's transfer indicator (#52), while the panel is closed: how far the transfers
 * have come, all of them together, with a 48 px bar; "Paused" when none runs; a summary's line
 * once only those are left. A click opens the panel again.
 */
export function TransferIndicator() {
  const list = useTransfers((state) => state.list);
  const lasting = useTransfers((state) => state.lasting);
  const summaries = useTransfers((state) => state.summaries);
  const hidden = useTransfers((state) => state.hidden);
  const view = useMemo(() => panelView(list, lasting, summaries), [list, lasting, summaries]);
  const { current, active } = view;
  if (!hidden || !view.due || !current) {
    return null;
  }
  let content: ReactNode;
  let text: string;
  if (current.kind === "job") {
    const percent = overallPercent(active);
    const running = active.some((job) => job.state.kind === "running");
    text = !running ? t.paused : percent === null ? t.counting : t.percent(percent);
    content = (
      <>
        <span className="shrink-0 text-accent">
          <KindIcon job={current.job} size={16} />
        </span>
        <span>{text}</span>
        <span aria-hidden="true" className="h-1 w-12 shrink-0 overflow-hidden rounded-bar bg-track">
          <span
            className="block h-full rounded-bar bg-accent"
            style={{ width: `${percent ?? 0}%` }}
          />
        </span>
      </>
    );
  } else {
    text = summaryTitle(current.summary, current.job);
    content = (
      <>
        <span className="shrink-0 text-red">
          <ErrorIcon size={16} />
        </span>
        <span className="truncate text-fg">{text}</span>
      </>
    );
  }
  return (
    <>
      <button
        type="button"
        title={t.show}
        aria-label={`${t.show}: ${text}`}
        onClick={showTransfers}
        className="inline-flex h-5.5 min-w-0 items-center gap-1.5 rounded-kbd px-1.5 hover:bg-hover"
      >
        {content}
      </button>
      <span aria-hidden="true" className="h-3 w-px shrink-0 bg-stroke" />
    </>
  );
}
