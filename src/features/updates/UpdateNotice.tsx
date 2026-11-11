import { useState } from "react";
import type { UpdateStage } from "../../ipc";
import { runningJobs, useTransfers } from "../../stores/transfers";
import { installUpdate, updateLater, useUpdates } from "../../stores/updates";
import { strings } from "../../strings";
import { Notice, NoticeActions, NoticeBar, NoticeButton } from "../../ui/Notice";
import { shortChanges } from "./notes";

const t = strings.updates;

/**
 * The notice about a new version (#46), a card like a toast that stays until it is answered: the
 * short list of changes, Update and Later. After Update it shows the progress; if that fails, why
 * and Retry. Nothing installs without the Update button. While transfers run, Update first says
 * that Jerry closes to update and stops them (#52): Update anyway, or Not now.
 */
export function UpdateNotice() {
  const offer = useUpdates((state) => state.offer);
  const later = useUpdates((state) => state.later);
  const running = useTransfers((state) => runningJobs(state.list).length);
  const [warned, setWarned] = useState(false);
  if (!offer || later) {
    return null;
  }
  const { version, notes, stage } = offer;
  const answerable = stage.kind === "offered" || stage.kind === "failed";
  if (answerable && warned && running > 0) {
    return (
      <Notice title={t.transfersRunning(running)} detail={t.available(version)}>
        <p className="text-small text-fg-2">{t.transfersHint(running)}</p>
        <NoticeActions>
          <NoticeButton onClick={() => setWarned(false)}>{t.notNow}</NoticeButton>
          <NoticeButton className="font-semibold" onClick={installUpdate}>
            {t.updateAnyway}
          </NoticeButton>
        </NoticeActions>
      </Notice>
    );
  }
  const { title, detail } = headline(version, stage);
  const update = () => {
    if (running > 0) {
      setWarned(true);
    } else {
      installUpdate();
    }
  };
  return (
    <Notice title={title} detail={detail}>
      {stage.kind === "offered" && <Changes notes={notes} />}
      {stage.kind === "downloading" && <NoticeBar percent={stage.percent} />}
      {stage.kind === "installing" && <NoticeBar percent={100} />}
      {answerable && (
        <NoticeActions>
          <NoticeButton onClick={updateLater}>{t.later}</NoticeButton>
          <NoticeButton className="font-semibold" onClick={update}>
            {stage.kind === "failed" ? t.retry : t.update}
          </NoticeButton>
        </NoticeActions>
      )}
    </Notice>
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
