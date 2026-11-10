import { useEffect, useId, useState } from "react";
import { commands, stream, type Tally } from "../../ipc";
import { errorText, strings } from "../../strings";
import { Dialog } from "../../ui/Dialog";
import { DialogButton, DialogButtons } from "../../ui/DialogButton";
import {
  confirmDelete,
  type DeleteDialogState,
  type DeleteMode,
  type DeleteRequest,
  dismissDelete,
  type Failure,
  retryDelete,
  useDeleteDialog,
} from "./flow";

const t = strings.deleting;

/** Failures listed by name; the rest are counted. */
const LISTED = 50;

/**
 * The delete confirmations and what a delete left behind (`flow.ts` decides which). Enter or the
 * focused button answers, Esc cancels.
 */
export function DeleteDialog() {
  const dialog = useDeleteDialog((state) => state.dialog);
  return (
    <Dialog
      open={dialog !== undefined}
      onClose={dismissDelete}
      title={dialog ? titleOf(dialog) : ""}
      closeHint={t.close}
    >
      {dialog?.kind === "confirm" && (
        <Confirm key={dialogKey(dialog)} mode={dialog.mode} request={dialog.request} />
      )}
      {dialog?.kind === "failed" && <Failed failures={dialog.failures} />}
    </Dialog>
  );
}

function titleOf(dialog: DeleteDialogState): string {
  return dialog.kind === "failed" ? t.failed.title : t[dialog.mode].title;
}

/** A new question starts fresh: its own count and an unticked system confirmation. */
function dialogKey(dialog: Extract<DeleteDialogState, { kind: "confirm" }>): string {
  return `${dialog.mode}:${dialog.request.listing}:${dialog.request.ids.join(",")}`;
}

function Confirm({ mode, request }: { mode: DeleteMode; request: DeleteRequest }) {
  const [agreed, setAgreed] = useState(false);
  const agreeId = useId();
  const { ids, folder } = request;
  const subject = {
    count: ids.length,
    name: ids.length === 1 ? folder.store.getState().entries.names[ids[0] ?? -1] : undefined,
  };
  const forGood = mode !== "recycle";
  const text = t[mode];

  return (
    <div className="flex flex-col">
      <div className="flex flex-col gap-2 px-4 py-3.5">
        <p className="break-words">{text.question(subject)}</p>
        {mode === "noRecycleBin" && <p className="text-fg-2 text-small">{t.noRecycleBin.why}</p>}
        {forGood && <Measure request={request} />}
        {request.system && (
          <div className="mt-1 flex flex-col gap-2">
            <p className="text-red text-small">{t.system.warning(ids.length)}</p>
            <label htmlFor={agreeId} className="flex items-center gap-2 text-small">
              <input
                id={agreeId}
                type="checkbox"
                checked={agreed}
                onChange={(event) => setAgreed(event.target.checked)}
                className="accent-accent"
              />
              {t.system.agree}
            </label>
          </div>
        )}
      </div>
      <DialogButtons>
        <DialogButton
          primary
          disabled={request.system && !agreed}
          onClick={() => void confirmDelete()}
        >
          {text.confirm}
        </DialogButton>
        <DialogButton onClick={dismissDelete}>
          {mode === "noRecycleBin" ? t.noRecycleBin.keep(ids.length) : t.cancel}
        </DialogButton>
      </DialogButtons>
    </div>
  );
}

/** "This can't be undone." and what goes, counted in Rust while the question is open. */
function Measure({ request }: { request: DeleteRequest }) {
  const [count, setCount] = useState<{ tally: Tally; done: boolean } | undefined>(undefined);
  const { listing, ids } = request;

  useEffect(() => {
    let open = true;
    const progress = stream<Tally>((tally) => {
      if (open) setCount({ tally, done: false });
    });
    void commands.deleteMeasure(listing, ids, progress).then((result) => {
      if (open && result.status === "ok") setCount({ tally: result.data, done: true });
    });
    return () => {
      open = false;
      void commands.deleteMeasureCancel(listing);
    };
  }, [listing, ids]);

  return (
    <p className="text-fg-2 text-small tabular-nums">
      {t.noUndo} {count ? t.tally(count.tally, count.done) : t.counting}
    </p>
  );
}

function Failed({ failures }: { failures: Failure[] }) {
  const listed = failures.slice(0, LISTED);
  return (
    <div className="flex flex-col">
      <div className="flex flex-col gap-2 px-4 py-3.5">
        <p>{t.failed.summary(failures.length)}</p>
        <ul className="flex max-h-60 flex-col gap-1.5 overflow-y-auto text-small">
          {listed.map((failure) => (
            <li key={failure.id} className="break-words">
              <span className="font-semibold">{failure.name}</span>{" "}
              <span className="text-fg-2">{errorText(failure.error)}</span>
            </li>
          ))}
        </ul>
        {failures.length > LISTED && (
          <p className="text-fg-2 text-small">{t.failed.more(failures.length - LISTED)}</p>
        )}
      </div>
      <DialogButtons>
        <DialogButton primary onClick={() => void retryDelete()}>
          {t.failed.retry}
        </DialogButton>
        <DialogButton onClick={dismissDelete}>{t.failed.dismiss}</DialogButton>
      </DialogButtons>
    </div>
  );
}
