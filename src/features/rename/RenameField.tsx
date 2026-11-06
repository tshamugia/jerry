import {
  type RefObject,
  type SyntheticEvent,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { RENAME_SCOPE } from "../../commands/dispatch";
import { errorText, strings } from "../../strings";
import type { NameProblem } from "./name";
import { leaveRename, setRenameText, useRename } from "./rename";

const t = strings.rename;

/** Keeps the list's own mouse handling (selection, marquee, open) out of the field. */
const stop = (event: SyntheticEvent) => event.stopPropagation();

/**
 * The inline rename field in a row's name cell (`rename.ts` runs it). It takes the focus with
 * the name selected without its extension (folders, and names whose extension is hidden: all of
 * it). Under it, as the user types, why the name can't be saved, or why the core refused it; the
 * hint goes above the field when there's no room under it. Its Enter, Esc and Tab are the rename
 * commands (`RENAME_SCOPE`).
 */
export function RenameField({ folder }: { folder: boolean }) {
  const editing = useRename((state) => state.editing);
  const field = useRef<HTMLInputElement>(null);
  const hintId = useId();
  const session = editing?.session;

  // biome-ignore lint/correctness/useExhaustiveDependencies: once per editing session.
  useLayoutEffect(() => {
    const input = field.current;
    if (!input || !editing) {
      return;
    }
    input.focus({ preventScroll: true });
    const { text, hidden } = editing;
    const dot = text.lastIndexOf(".");
    input.setSelectionRange(0, folder || hidden !== "" || dot <= 0 ? text.length : dot);
  }, [session]);

  const hint = editing?.error
    ? `${errorText(editing.error)} ${t.tryAgain}`
    : editing?.problem && problemText(editing.problem);
  const above = useHintAbove(field, hint);

  if (!editing) {
    return null;
  }
  const invalid = editing.problem !== undefined || editing.error !== undefined;
  return (
    <span className="relative flex min-w-0 flex-1 items-center">
      <input
        ref={field}
        {...RENAME_SCOPE}
        type="text"
        value={editing.text}
        aria-label={t.field}
        aria-invalid={invalid}
        aria-describedby={hint ? hintId : undefined}
        readOnly={editing.saving}
        spellCheck={false}
        autoComplete="off"
        onChange={(event) => setRenameText(event.target.value)}
        // Not when the window itself loses the focus: the field is still there on return.
        onBlur={() => document.hasFocus() && leaveRename(editing.session)}
        onPointerDown={stop}
        onMouseDown={stop}
        onClick={stop}
        onDoubleClick={stop}
        className={`min-w-0 flex-1 select-text rounded-ctrl border bg-pane px-1.5 py-0.5 text-fg outline-none ${invalid ? "border-red" : "border-accent"}`}
      />
      {hint && (
        <span
          id={hintId}
          className={`absolute left-0 z-10 w-max max-w-80 rounded-ctrl border border-flyout-border bg-flyout px-2 py-1 text-fg text-small shadow-flyout ${above ? "bottom-full mb-1" : "top-full mt-1"}`}
        >
          {hint}
        </span>
      )}
    </span>
  );
}

function problemText(problem: NameProblem): string {
  switch (problem.kind) {
    case "empty":
      return t.problems.empty;
    case "character":
      return t.problems.character(problem.character);
    case "trailing":
      return t.problems.trailing;
    case "reserved":
      return t.problems.reserved(problem.device);
    case "tooLong":
      return t.problems.tooLong;
    case "taken":
      return t.problems.taken(problem.name);
  }
}

/** Hint height the list must have room for under the field, in CSS px, before it goes above. */
const HINT_ROOM = 56;

/** Whether the hint goes above the field: too close to the list's bottom edge for it to fit under. */
function useHintAbove(
  field: RefObject<HTMLInputElement | null>,
  hint: string | undefined,
): boolean {
  const [above, setAbove] = useState(false);
  useEffect(() => {
    const input = field.current;
    const list = input?.closest<HTMLElement>('[role="grid"]');
    if (!input || !list || !hint) {
      return;
    }
    const place = () => {
      const bottom = list.getBoundingClientRect().top + list.clientHeight;
      setAbove(input.getBoundingClientRect().bottom + HINT_ROOM > bottom);
    };
    place();
    list.addEventListener("scroll", place, { passive: true });
    return () => list.removeEventListener("scroll", place);
  }, [field, hint]);
  return above;
}
