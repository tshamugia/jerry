import { errorText, strings } from "../../strings";
import { Dialog } from "../../ui/Dialog";
import { dismissShellFailure, retryShell, useShellFailure } from "./shell";

const t = strings.shell;

const BUTTON = "h-8 rounded-ctrl border border-ctrl-border bg-ctrl px-3 text-fg hover:bg-hover";

/**
 * Show in Explorer or Open in Terminal failed: which, why, and Retry (the focus starts there:
 * Enter retries, Esc closes).
 */
export function ShellDialog() {
  const failed = useShellFailure((state) => state.failed);
  return (
    <Dialog
      open={failed !== undefined}
      onClose={dismissShellFailure}
      title={failed ? t.failed[failed.action] : ""}
      closeHint={t.closeHint}
    >
      <p className="break-words px-4 pt-3 text-fg">{failed && errorText(failed.error)}</p>
      <div className="flex justify-end gap-2 p-4">
        <button type="button" onClick={retryShell} className={`${BUTTON} font-semibold`}>
          {t.retry}
        </button>
        <button type="button" onClick={dismissShellFailure} className={BUTTON}>
          {t.close}
        </button>
      </div>
    </Dialog>
  );
}
