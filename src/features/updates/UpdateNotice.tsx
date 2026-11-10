import type { UpdateStage } from "../../ipc";
import { installUpdate, updateLater, useUpdates } from "../../stores/updates";
import { strings } from "../../strings";
import { Notice, NoticeActions, NoticeBar, NoticeButton } from "../../ui/Notice";
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
    <Notice title={title} detail={detail}>
      {stage.kind === "offered" && <Changes notes={notes} />}
      {stage.kind === "downloading" && <NoticeBar percent={stage.percent} />}
      {stage.kind === "installing" && <NoticeBar percent={100} />}
      {answerable && (
        <NoticeActions>
          <NoticeButton onClick={updateLater}>{t.later}</NoticeButton>
          <NoticeButton className="font-semibold" onClick={installUpdate}>
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
