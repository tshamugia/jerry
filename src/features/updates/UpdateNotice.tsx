import type { ButtonHTMLAttributes } from "react";
import type { UpdateStage } from "../../ipc";
import { installUpdate, updateLater, useUpdates } from "../../stores/updates";
import { strings } from "../../strings";
import { InfoIcon } from "../../ui/icons";
import { shortChanges } from "./notes";

const t = strings.updates;

/**
 * The notice about a new version (#46), a card like a toast that stays until it is answered: the
 * short list of changes, Update and Later. After Update it shows the progress; if that fails, why
 * and Retry. Nothing installs without the Update button.
 */
export function UpdateNotice() {
  const offer = useUpdates((state) => state.offer);
  const later = useUpdates((state) => state.later);
  if (!offer || later) {
    return null;
  }
  const { version, notes, stage } = offer;
  const { title, detail } = headline(version, stage);
  const answerable = stage.kind === "offered" || stage.kind === "failed";
  return (
    <div
      role="status"
      className="pointer-events-auto flex flex-col gap-2.5 rounded-flyout border border-flyout-border bg-flyout py-3 pr-3 pl-3.5 shadow-flyout"
    >
      <div className="flex items-center gap-3">
        <span className="shrink-0 text-fg-2">
          <InfoIcon />
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-px">
          <span className="truncate font-semibold">{title}</span>
          {detail && <span className="truncate text-small text-fg-2 tabular-nums">{detail}</span>}
        </div>
      </div>
      {stage.kind === "offered" && <Changes notes={notes} />}
      {stage.kind === "downloading" && <Bar percent={stage.percent} />}
      {stage.kind === "installing" && <Bar percent={100} />}
      {answerable && (
        <div className="flex justify-end gap-2">
          <NoticeButton onClick={updateLater}>{t.later}</NoticeButton>
          <NoticeButton className="font-semibold" onClick={installUpdate}>
            {stage.kind === "failed" ? t.retry : t.update}
          </NoticeButton>
        </div>
      )}
    </div>
  );
}

function headline(version: string, stage: UpdateStage): { title: string; detail?: string } {
  switch (stage.kind) {
    case "offered":
      return { title: t.available(version) };
    case "downloading":
      return { title: t.updating(version), detail: t.downloading(stage.percent) };
    case "installing":
      return { title: t.updating(version), detail: t.installing };
    case "failed":
      return { title: t.failed, detail: t.failedHint };
  }
}

function Changes({ notes }: { notes: string }) {
  const { changes, more } = shortChanges(notes);
  if (changes.length === 0) {
    return null;
  }
  return (
    <ul className="flex flex-col gap-0.5 text-small text-fg-2">
      {changes.map((change) => (
        <li key={change} className="truncate" title={change}>
          {change}
        </li>
      ))}
      {more > 0 && <li>{t.moreChanges(more)}</li>}
    </ul>
  );
}

/** The accent bar; it sweeps while the size isn't known. */
function Bar({ percent }: { percent: number | null }) {
  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent ?? undefined}
      className="h-1 overflow-hidden rounded-bar bg-track"
    >
      {percent === null ? (
        <div className="h-full w-1/3 animate-loading bg-accent motion-reduce:animate-none" />
      ) : (
        <div className="h-full bg-accent" style={{ width: `${percent}%` }} />
      )}
    </div>
  );
}

function NoticeButton({ className = "", ...rest }: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      className={`h-toolbar-btn min-w-20 rounded-ctrl border border-ctrl-border bg-ctrl px-3 hover:bg-hover ${className}`}
      {...rest}
    />
  );
}
