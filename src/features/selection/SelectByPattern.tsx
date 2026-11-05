import { type FormEvent, useMemo, useState } from "react";
import { closeOverlay, useOverlay } from "../../stores/overlay";
import { strings } from "../../strings";
import { Dialog } from "../../ui/Dialog";
import { activeFolder, selectionOf, shownRows } from "./commands";
import { selectMatching } from "./model";

const t = strings.selectByPattern;

/** The pattern used last in this window, offered (selected) the next time. */
let lastPattern = "";

/**
 * Select by pattern (Ctrl+Shift+A): a pattern such as `*.pdf` or `*_rev*` (the quick filter's
 * wildcards) selects every row of the active pane's list whose name matches, in place of the
 * selection. While typing it says how many match; Enter selects them, Esc leaves the selection
 * as it was.
 */
export function SelectByPattern() {
  const open = useOverlay((state) => state.overlay === "selectByPattern");
  return (
    <Dialog open={open} onClose={closeOverlay} title={t.title} closeHint={t.close}>
      {open && <PatternForm />}
    </Dialog>
  );
}

function PatternForm() {
  const [pattern, setPattern] = useState(lastPattern);
  // The active pane and its tab can't change while the dialog is open.
  const folder = useMemo(() => activeFolder(), []);
  const preview = useMemo(() => {
    if (pattern === "" || !folder) {
      return undefined;
    }
    const state = folder.store.getState();
    const rows = shownRows(folder);
    const matched = selectMatching(selectionOf(state), rows, state.entries.names, pattern);
    return { count: matched.selected.size, total: rows.length };
  }, [folder, pattern]);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (pattern === "" || !folder) {
      return;
    }
    const state = folder.store.getState();
    folder.setSelection(
      selectMatching(selectionOf(state), shownRows(folder), state.entries.names, pattern),
    );
    lastPattern = pattern;
    closeOverlay();
  };

  return (
    <form onSubmit={submit} className="flex flex-col">
      <input
        type="text"
        value={pattern}
        onChange={(event) => setPattern(event.target.value)}
        onFocus={(event) => event.currentTarget.select()}
        aria-label={t.pattern}
        placeholder={t.placeholder}
        spellCheck={false}
        autoComplete="off"
        className="h-13 border-stroke border-b bg-transparent px-4 text-fg outline-none placeholder:text-fg-3"
      />
      <output aria-live="polite" className="px-4 py-2.5 text-fg-2 text-small tabular-nums">
        {preview ? t.matches(preview.count, preview.total) : t.hint}
      </output>
    </form>
  );
}
