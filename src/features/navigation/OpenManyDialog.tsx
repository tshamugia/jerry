import { strings } from "../../strings";
import { Dialog } from "../../ui/Dialog";
import { useOpenMany } from "./open";

const t = strings.openMany;

const BUTTON = "h-8 rounded-ctrl border border-ctrl-border bg-ctrl px-3 text-fg hover:bg-hover";

/**
 * Asks before opening many items at once (more than `MANY_ITEMS`), as Explorer does: each would
 * start a program or a tab. Enter opens them (the focus starts on Open all), Esc cancels.
 */
export function OpenManyDialog() {
  const pending = useOpenMany((state) => state.pending);
  const close = () => useOpenMany.setState({ pending: undefined });
  const open = () => {
    close();
    pending?.run();
  };
  return (
    <Dialog open={pending !== undefined} onClose={close} title={t.title} closeHint={t.close}>
      <p className="px-4 pt-3 text-fg">{pending && t.question(pending.count)}</p>
      <div className="flex justify-end gap-2 px-4 pt-4 pb-4">
        <button
          type="button"
          // Radix focuses the first focusable element: Enter opens.
          onClick={open}
          className={`${BUTTON} font-semibold`}
        >
          {t.open}
        </button>
        <button type="button" onClick={close} className={BUTTON}>
          {t.cancel}
        </button>
      </div>
    </Dialog>
  );
}
