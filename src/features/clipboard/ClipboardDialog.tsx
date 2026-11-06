import { errorText, strings } from "../../strings";
import { Dialog } from "../../ui/Dialog";
import { dismissClipboardFailure, retryClipboard, useClipboard } from "./clipboard";

const t = strings.clipboard;

const BUTTON = "h-8 rounded-ctrl border border-ctrl-border bg-ctrl px-3 text-fg hover:bg-hover";

/**
 * A clipboard command that failed: which, why, and Retry (the focus starts there: Enter retries,
 * Esc closes). Paste's own trouble (a file in use, no space) Windows shows in its window.
 */
export function ClipboardDialog() {
  const failed = useClipboard((state) => state.failed);
  return (
    <Dialog
      open={failed !== undefined}
      onClose={dismissClipboardFailure}
      title={failed ? t.failed[failed.action] : ""}
      closeHint={t.closeHint}
    >
      <p className="break-words px-4 pt-3 text-fg">{failed && errorText(failed.error)}</p>
      <div className="flex justify-end gap-2 p-4">
        <button type="button" onClick={retryClipboard} className={`${BUTTON} font-semibold`}>
          {t.retry}
        </button>
        <button type="button" onClick={dismissClipboardFailure} className={BUTTON}>
          {t.close}
        </button>
      </div>
    </Dialog>
  );
}
