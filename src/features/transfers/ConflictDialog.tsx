import {
  type KeyboardEvent,
  type RefObject,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  type ConflictChoice,
  type ConflictItem,
  type ConflictSide,
  commands,
  conflictThumbUrl,
  typeIconUrl,
} from "../../ipc";
import { useSetting } from "../../stores/settings";
import { type Asking, firstConflict, resolveConflict, useTransfers } from "../../stores/transfers";
import { sizeText, strings } from "../../strings";
import { Dialog } from "../../ui/Dialog";
import { DialogButton } from "../../ui/DialogButton";
import { Kbd } from "../../ui/Kbd";
import { Thumbnail } from "../../ui/Thumbnail";
import { dateText } from "../file-list/format";
import { lastName } from "./UnfinishedNotice";

const t = strings.transfers.conflict;

/** Each choice's key: `KeyboardEvent.code`, so it works on the Georgian layout too. */
const KEYS: Record<ConflictChoice, { code: string; label: string }> = {
  replace: { code: "KeyR", label: "R" },
  skip: { code: "KeyS", label: "S" },
  keepBoth: { code: "KeyK", label: "K" },
  replaceIfNewer: { code: "KeyN", label: "N" },
  merge: { code: "KeyM", label: "M" },
};

const APPLY_TO_ALL = { code: "KeyA", label: "A" };

/** The icons' size, CSS px. */
const ICON_PX = 32;

/** The box a thumbnail fits in, CSS px (`size-16`). */
const THUMB_PX = 64;

/**
 * The conflict window (#51, PRD «კონფლიქტები»): a name a copy or move writes is taken, and its
 * job waits for the answer while the others go on. It shows the first such job in queue order:
 * both items with their size, date modified and thumbnail (#67; the type icon where there is
 * none), the newer one marked, and the choices the core offers for them. A letter key or Enter
 * on the focused choice answers; A ticks Apply to all, ↑↓ move between the choices, and Esc
 * cancels the transfer, as closing Explorer's does.
 * Where Replace would delete for good (no Recycle Bin), Skip has the focus.
 */
export function ConflictDialog() {
  const list = useTransfers((state) => state.list);
  const asking = useMemo(() => firstConflict(list), [list]);
  // Hidden once answered until the queue says what comes next, so nothing is answered twice.
  const [answeredAt, setAnsweredAt] = useState<number | undefined>(undefined);
  const shown = asking && answeredAt !== list.generation ? asking : undefined;
  const focusFirst = useRef<HTMLButtonElement | null>(null);

  // A new question while the window is open starts at its own first choice; as it opens, the
  // dialog does that.
  const question = shown ? `${shown.job.id}\n${JSON.stringify(shown.conflict)}` : "";
  const previous = useRef("");
  useEffect(() => {
    if (previous.current !== "" && question !== "" && question !== previous.current) {
      focusFirst.current?.focus({ preventScroll: true });
    }
    previous.current = question;
  }, [question]);

  const job = shown?.job.id;
  const answer = (choice: ConflictChoice, all: boolean) => {
    if (job === undefined) {
      return;
    }
    setAnsweredAt(list.generation);
    resolveConflict(job, choice, all);
  };
  const cancel = () => {
    if (job === undefined) {
      return;
    }
    setAnsweredAt(list.generation);
    void commands.transferControl("cancel", job);
  };

  const conflict = shown?.conflict;
  const title = conflict
    ? conflict.noRecycleBin
      ? t.noRecycleBinTitle
      : t.title(conflict.existing.folder)
    : "";
  return (
    <Dialog
      open={shown !== undefined}
      onClose={cancel}
      title={title}
      closeHint={t.close}
      initialFocus={focusFirst}
    >
      {shown && (
        <Question
          key={question}
          asking={shown}
          question={question}
          focusFirst={focusFirst}
          onAnswer={answer}
          onCancel={cancel}
        />
      )}
    </Dialog>
  );
}

function Question({
  asking,
  question,
  focusFirst,
  onAnswer,
  onCancel,
}: {
  asking: Asking;
  /** Tells this question from the next one the job asks. */
  question: string;
  focusFirst: RefObject<HTMLButtonElement | null>;
  onAnswer: (choice: ConflictChoice, all: boolean) => void;
  onCancel: () => void;
}) {
  const { job, conflict } = asking;
  const [all, setAll] = useState(false);
  const allId = useId();
  const ids = useId();
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);
  const { choices } = conflict;
  const first: ConflictChoice | undefined = conflict.noRecycleBin ? "skip" : choices[0];
  const newer = newerOf(conflict.incoming, conflict.existing);
  const thumb = (side: ConflictSide, item: ConflictItem) =>
    item.folder ? undefined : conflictThumbUrl(job.id, side, question, THUMB_PX);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) {
      return;
    }
    if (event.code === APPLY_TO_ALL.code) {
      event.preventDefault();
      setAll((ticked) => !ticked);
      return;
    }
    const chosen = choices.find((choice) => KEYS[choice].code === event.code);
    if (chosen) {
      event.preventDefault();
      onAnswer(chosen, all);
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      const at = buttons.current.indexOf(document.activeElement as HTMLButtonElement);
      if (at >= 0) {
        event.preventDefault();
        const step = event.key === "ArrowDown" ? 1 : -1;
        buttons.current[(at + step + choices.length) % choices.length]?.focus();
      }
    }
  };

  return (
    // The keys work wherever the focus is in the window.
    // biome-ignore lint/a11y/noStaticElementInteractions: a key handler for the whole dialog
    <div className="flex flex-col" onKeyDown={onKeyDown}>
      <div className="flex flex-col gap-3 px-4 py-3.5">
        <p className="break-words">
          {t.already(lastName(conflict.target), lastName(parentOf(conflict.target)))}
        </p>
        {conflict.noRecycleBin && <p className="text-red text-small">{t.noRecycleBin}</p>}
        <div className="grid grid-cols-2 gap-2">
          <Side
            caption={t.incoming(job.kind, lastName(parentOf(conflict.source)))}
            name={lastName(conflict.source)}
            item={conflict.incoming}
            thumb={thumb("incoming", conflict.incoming)}
            newer={newer === "incoming"}
          />
          <Side
            caption={t.existing(lastName(parentOf(conflict.target)))}
            name={lastName(conflict.target)}
            item={conflict.existing}
            thumb={thumb("existing", conflict.existing)}
            newer={newer === "existing"}
          />
        </div>
        <fieldset aria-label={t.choices} className="flex flex-col gap-1">
          {choices.map((choice, index) => (
            <button
              key={choice}
              type="button"
              ref={(button) => {
                buttons.current[index] = button;
                if (choice === first) {
                  focusFirst.current = button;
                }
              }}
              onClick={() => onAnswer(choice, all)}
              aria-labelledby={`${ids}-${choice}`}
              aria-describedby={`${ids}-${choice}-hint`}
              aria-keyshortcuts={KEYS[choice].label}
              className="flex items-center gap-3 rounded-ctrl border border-ctrl-border bg-ctrl px-3 py-2 text-left hover:bg-hover"
            >
              <span className="flex min-w-0 flex-1 flex-col">
                <span id={`${ids}-${choice}`} className="font-semibold">
                  {t.choice[choice]}
                </span>
                <span id={`${ids}-${choice}-hint`} className="text-fg-2 text-small">
                  {choice === "replace" && conflict.noRecycleBin
                    ? t.replaceForGood
                    : t.hint[choice]}
                </span>
              </span>
              <Kbd>{KEYS[choice].label}</Kbd>
            </button>
          ))}
        </fieldset>
      </div>
      <div className="flex items-center gap-2 border-stroke border-t px-4 py-3">
        <label htmlFor={allId} className="flex flex-1 items-center gap-2">
          <input
            id={allId}
            type="checkbox"
            checked={all}
            onChange={(event) => setAll(event.target.checked)}
            aria-keyshortcuts={APPLY_TO_ALL.label}
            className="accent-accent"
          />
          {t.applyToAll}
          <Kbd>{APPLY_TO_ALL.label}</Kbd>
        </label>
        <DialogButton onClick={onCancel}>{t.cancel}</DialogButton>
      </div>
    </div>
  );
}

/** One side: where it is, its thumbnail or icon, name, size and date modified. */
function Side({
  caption,
  name,
  item,
  thumb,
  newer,
}: {
  caption: string;
  name: string;
  item: ConflictItem;
  /** Its `thumb://` URL; none for a folder. */
  thumb: string | undefined;
  newer: boolean;
}) {
  const format = useSetting("dateFormat") ?? "DD.MM.YYYY";
  const date = item.modified === null ? t.noDate : dateText(item.modified, format);
  return (
    <div className="flex min-w-0 flex-col gap-1.5 rounded-ctrl border border-stroke px-3 py-2.5">
      <span className="truncate text-caption text-fg-3" title={caption}>
        {caption}
      </span>
      <div className="flex min-w-0 items-center gap-2.5">
        <Thumbnail
          src={thumb}
          icon={typeIconUrl(item.icon, ICON_PX)}
          className="size-16 shrink-0"
          iconClassName="size-8"
        />
        <div className="flex min-w-0 flex-col">
          <span className="truncate" title={name}>
            {name}
          </span>
          <span className="text-fg-2 text-small tabular-nums">
            {item.folder ? t.folder : sizeText(item.size)}
          </span>
          <span
            className={`text-small tabular-nums ${newer ? "font-semibold text-fg" : "text-fg-2"}`}
          >
            {newer ? `${date} · ${t.newer}` : date}
          </span>
        </div>
      </div>
    </div>
  );
}

/** Which side was modified later; neither if a date is missing or they are the same. */
function newerOf(incoming: ConflictItem, existing: ConflictItem): "incoming" | "existing" | null {
  if (
    incoming.modified === null ||
    existing.modified === null ||
    incoming.modified === existing.modified
  ) {
    return null;
  }
  return incoming.modified > existing.modified ? "incoming" : "existing";
}

/** The folder a display path is in: `C:\Work` for `C:\Work\a.dwg`, `C:\` for `C:\a.dwg`. */
export function parentOf(path: string): string {
  const parent = path.slice(0, Math.max(0, path.lastIndexOf("\\")));
  return parent.endsWith(":") ? `${parent}\\` : parent;
}
