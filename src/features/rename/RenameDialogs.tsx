import { errorText, strings } from "../../strings";
import { Dialog } from "../../ui/Dialog";
import {
  confirmExtension,
  dismissCreateFailed,
  extensionChange,
  keepEditing,
  retryNewFolder,
  useRename,
} from "./rename";

const t = strings.rename;

const BUTTON = "h-8 rounded-ctrl border border-ctrl-border bg-ctrl px-3 text-fg hover:bg-hover";

/**
 * The rename's questions: saving a name with another extension (Change, or back to the field),
 * and a new folder that couldn't be made (Retry). The first button has the focus: Enter answers
 * yes, Esc no.
 */
export function RenameDialogs() {
  const editing = useRename((state) => state.editing);
  const createFailed = useRename((state) => state.createFailed);
  const asking = editing?.confirming !== undefined;
  const change = editing && asking ? extensionChange(editing) : undefined;
  return (
    <>
      <Dialog
        open={asking}
        onClose={keepEditing}
        title={t.extension.title}
        closeHint={t.extension.close}
      >
        <p className="break-words px-4 pt-3 text-fg">
          {change && t.extension.question(change.from, change.to)}
        </p>
        <div className="flex justify-end gap-2 p-4">
          <button
            type="button"
            onClick={() => void confirmExtension()}
            className={`${BUTTON} font-semibold`}
          >
            {t.extension.change}
          </button>
          <button type="button" onClick={keepEditing} className={BUTTON}>
            {t.extension.cancel}
          </button>
        </div>
      </Dialog>
      <Dialog
        open={createFailed !== undefined}
        onClose={dismissCreateFailed}
        title={t.createFailed.title}
        closeHint={t.createFailed.closeHint}
      >
        <p className="break-words px-4 pt-3 text-fg">
          {createFailed && errorText(createFailed.error)}
        </p>
        <div className="flex justify-end gap-2 p-4">
          <button type="button" onClick={retryNewFolder} className={`${BUTTON} font-semibold`}>
            {t.createFailed.retry}
          </button>
          <button type="button" onClick={dismissCreateFailed} className={BUTTON}>
            {t.createFailed.close}
          </button>
        </div>
      </Dialog>
    </>
  );
}
