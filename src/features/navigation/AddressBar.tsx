import {
  type KeyboardEvent,
  type MouseEvent,
  memo,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useStore } from "zustand";
import { type Crumb, commands, type JerryError } from "../../ipc";
import { useSetting } from "../../stores/settings";
import type { Tab } from "../../stores/tabs";
import type { PaneSide } from "../../stores/window";
import { errorText, strings } from "../../strings";
import { DropdownItem, DropdownMenu } from "../../ui/DropdownMenu";
import { IconButton } from "../../ui/IconButton";
import { ChevronIcon, MoreIcon } from "../../ui/icons";
import { focusListSoon } from "../filter/focus";
import { addressError, addressTarget, editAddress, stopEditing, useAddressEdit } from "./address";

const t = strings.address;

/** Completions wait this long after the last key. */
const COMPLETE_DELAY_MS = 120;

/**
 * A pane's address bar (#33): the folder as segments (`D: › Projects › BMW › Drawings`), the
 * current one bold. A segment opens its folder; the arrow after it drops its subfolders. In a
 * narrow pane the middle segments fold into one button. A click on the bar's empty space, or
 * Ctrl+L, turns it into a text field with completions: any form of a path opens the right folder,
 * a file's path opens its folder with the file selected, and one that leads nowhere says so at
 * the field without leaving the folder.
 */
export const AddressBar = memo(function AddressBar({ tab, pane }: { tab: Tab; pane: PaneSide }) {
  const entry = useStore(tab.store, (state) => state.history[state.index]);
  const editing = useAddressEdit((state) => state.pane === pane);
  return editing ? (
    <AddressField tab={tab} pane={pane} path={entry?.path ?? ""} />
  ) : (
    <Crumbs tab={tab} pane={pane} crumbs={entry?.crumbs ?? []} />
  );
});

/** Which segments a narrow bar leaves out: `lo` up to (not with) `hi`. */
interface Folded {
  lo: number;
  hi: number;
}

function Crumbs({ tab, pane, crumbs }: { tab: Tab; pane: PaneSide; crumbs: readonly Crumb[] }) {
  const bar = useRef<HTMLDivElement>(null);
  const ruler = useRef<HTMLDivElement>(null);
  const [folded, setFolded] = useState<Folded>();
  const go = (crumb: Crumb) => {
    void tab.open({ kind: "folder", folder: crumb.folder });
    focusListSoon(pane);
  };

  // Folds the middle segments when they don't fit, measured on a hidden copy of all of them.
  useLayoutEffect(() => {
    const box = bar.current;
    const measured = ruler.current;
    if (!box || !measured) {
      return;
    }
    const refold = () => setFolded(fold(box, measured));
    refold();
    const observer = new ResizeObserver(refold);
    observer.observe(box);
    return () => observer.disconnect();
  }, []);

  const last = crumbs.length - 1;
  const segment = (crumb: Crumb, index: number) => (
    <span key={crumb.folder} className="flex min-w-0 shrink-0 items-center last:shrink">
      {index === last ? (
        <span className="truncate px-1 font-semibold text-fg">{crumb.name}</span>
      ) : (
        <button
          type="button"
          onClick={() => go(crumb)}
          className="no-drag truncate rounded-kbd px-1 text-fg hover:bg-hover"
        >
          {crumb.name}
        </button>
      )}
      <SubfolderMenu crumb={crumb} next={crumbs[index + 1]?.folder} onOpen={go} />
    </span>
  );
  const shown = folded
    ? [
        ...crumbs.slice(0, folded.lo).map(segment),
        <FoldedMenu key="folded" crumbs={crumbs.slice(folded.lo, folded.hi)} onOpen={go} />,
        ...crumbs.slice(folded.hi).map((crumb, i) => segment(crumb, folded.hi + i)),
      ]
    : crumbs.map(segment);

  // Empty space, the current folder's name included, turns the bar into a text field.
  const edit = (event: MouseEvent) => {
    if (!(event.target as Element).closest("button")) {
      editAddress(pane);
    }
  };

  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: Ctrl+L is the keyboard's way in.
    // biome-ignore lint/a11y/noStaticElementInteractions: as above.
    <div
      ref={bar}
      onClick={edit}
      title={t.edit}
      className="relative flex h-full min-w-0 flex-1 cursor-text items-center overflow-hidden text-small"
    >
      {shown}
      <div ref={ruler} aria-hidden="true" className="invisible absolute flex">
        {crumbs.map((crumb) => (
          <span key={crumb.folder} className="flex shrink-0 items-center">
            <span className="px-1 font-semibold">{crumb.name}</span>
            <span className="size-5" />
          </span>
        ))}
      </div>
    </div>
  );
}

/** Room the folded button takes, in px. */
const FOLDED_PX = 24;

/** Which segments to fold so the rest fit `box`: none if all do; else keep the last ones. */
function fold(box: HTMLElement, ruler: HTMLElement): Folded | undefined {
  const room = box.clientWidth;
  const widths = [...ruler.children].map((child) => (child as HTMLElement).offsetWidth);
  const total = widths.reduce((sum, width) => sum + width, 0);
  // Not laid out (a hidden pane) or room for all: nothing to fold.
  if (room === 0 || total <= room || widths.length < 3) {
    return undefined;
  }
  const first = widths[0] ?? 0;
  let hi = widths.length - 1;
  let used = FOLDED_PX + (widths[hi] ?? 0);
  while (hi > 1 && used + first + (widths[hi - 1] ?? 0) <= room) {
    hi -= 1;
    used += widths[hi] ?? 0;
  }
  return { lo: used + first <= room ? 1 : 0, hi };
}

/** The arrow after a segment: its folder's subfolders, the one on the path bold. */
function SubfolderMenu({
  crumb,
  next,
  onOpen,
}: {
  crumb: Crumb;
  /** The subfolder the path goes on through. */
  next: number | undefined;
  onOpen: (crumb: Crumb) => void;
}) {
  const showHidden = useSetting("showHidden") ?? false;
  const [list, setList] = useState<
    { folders: readonly Crumb[]; more: boolean } | { error: JerryError } | undefined
  >();
  const load = (open: boolean) => {
    if (!open) {
      return;
    }
    setList(undefined);
    void commands.folderSubfolders(crumb.folder, showHidden).then((result) => {
      setList(result.status === "ok" ? result.data : { error: result.error });
    });
  };
  const label = t.subfolders(crumb.name);
  return (
    <DropdownMenu
      label={label}
      onOpenChange={load}
      trigger={
        <IconButton label={label} className="size-5 shrink-0 rounded-kbd text-fg-2">
          <ChevronIcon />
        </IconButton>
      }
    >
      {list === undefined ? (
        <DropdownItem disabled>{t.loading}</DropdownItem>
      ) : "error" in list ? (
        <DropdownItem disabled>{errorText(list.error)}</DropdownItem>
      ) : list.folders.length === 0 ? (
        <DropdownItem disabled>{t.none}</DropdownItem>
      ) : (
        <>
          {list.folders.map((folder) => (
            <DropdownItem key={folder.folder} onSelect={() => onOpen(folder)} title={folder.name}>
              <span className={`truncate ${folder.folder === next ? "font-semibold" : ""}`}>
                {folder.name}
              </span>
            </DropdownItem>
          ))}
          {list.more && <DropdownItem disabled>{t.more(list.folders.length)}</DropdownItem>}
        </>
      )}
    </DropdownMenu>
  );
}

/** The middle segments of a narrow bar, as one button that lists them. */
function FoldedMenu({
  crumbs,
  onOpen,
}: {
  crumbs: readonly Crumb[];
  onOpen: (crumb: Crumb) => void;
}) {
  return (
    <DropdownMenu
      label={t.collapsed}
      trigger={
        <IconButton label={t.collapsed} className="h-5 w-6 shrink-0 rounded-kbd text-fg-2">
          <MoreIcon />
        </IconButton>
      }
    >
      {[...crumbs].reverse().map((crumb) => (
        <DropdownItem key={crumb.folder} onSelect={() => onOpen(crumb)} title={crumb.name}>
          <span className="truncate">{crumb.name}</span>
        </DropdownItem>
      ))}
    </DropdownMenu>
  );
}

/**
 * The bar as a text field, its text selected. Enter opens what it names (a suggestion, if one is
 * picked with the arrows); Esc or leaving it goes back to the segments; Tab takes a suggestion
 * into the field to go on typing.
 */
function AddressField({ tab, pane, path }: { tab: Tab; pane: PaneSide; path: string }) {
  const showHidden = useSetting("showHidden") ?? false;
  const [text, setText] = useState(path);
  const [suggestions, setSuggestions] = useState<readonly string[]>([]);
  const [picked, setPicked] = useState(-1);
  const [error, setError] = useState<string>();
  const input = useRef<HTMLInputElement>(null);
  const typed = useRef(false);
  const asked = useRef(0);
  const busy = useRef(false);
  const listId = useId();
  const value = suggestions[picked] ?? text;

  useLayoutEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);

  // Completions for what was typed, a moment after the last key; a late answer is dropped.
  useEffect(() => {
    if (!typed.current) {
      return;
    }
    const mine = ++asked.current;
    const timer = setTimeout(() => {
      const from = tab.current()?.folder ?? null;
      void commands.autocompletePath(text, from, showHidden).then((result) => {
        if (mine === asked.current) {
          setSuggestions(result.status === "ok" ? result.data : []);
          setPicked(-1);
        }
      });
    }, COMPLETE_DELAY_MS);
    return () => clearTimeout(timer);
  }, [text, tab, showHidden]);

  const close = (toList: boolean) => {
    asked.current += 1;
    stopEditing(pane);
    if (toList) {
      focusListSoon(pane);
    }
  };
  const open = async (address: string) => {
    if (busy.current) {
      return;
    }
    busy.current = true;
    setError(undefined);
    const result = await commands.resolvePath(address, tab.current()?.folder ?? null);
    busy.current = false;
    if (result.status === "error") {
      setSuggestions([]);
      setError(addressError(result.error, address));
      return;
    }
    close(true);
    const { target, restore } = addressTarget(result.data);
    void tab.open(target, restore);
  };

  const keyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) {
      return;
    }
    const step = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0;
    if (event.key === "Enter") {
      event.preventDefault();
      void open(value);
    } else if (event.key === "Escape") {
      event.preventDefault();
      close(true);
    } else if (step !== 0 && suggestions.length > 0) {
      event.preventDefault();
      const count = suggestions.length;
      // Round the list and back to what was typed (-1).
      setPicked((at) => ((at + 1 + step + count + 1) % (count + 1)) - 1);
    } else if (event.key === "Tab" && !event.shiftKey && suggestions.length > 0) {
      event.preventDefault();
      typed.current = true;
      setText(suggestions[Math.max(picked, 0)] ?? text);
      setPicked(-1);
    }
  };

  return (
    <div className="relative flex h-full min-w-0 flex-1 items-center">
      <input
        ref={input}
        type="text"
        role="combobox"
        aria-label={t.edit}
        aria-expanded={suggestions.length > 0}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={picked >= 0 ? `${listId}-${picked}` : undefined}
        aria-invalid={error !== undefined}
        value={value}
        onChange={(event) => {
          typed.current = true;
          setError(undefined);
          setPicked(-1);
          setText(event.target.value);
        }}
        onKeyDown={keyDown}
        onBlur={() => close(false)}
        spellCheck={false}
        autoComplete="off"
        className="min-w-0 flex-1 bg-transparent px-1 text-fg text-small outline-none"
      />
      {error !== undefined ? (
        <p
          role="alert"
          className="absolute inset-x-0 top-full z-50 mt-1.5 rounded-flyout border border-flyout-border bg-flyout px-3 py-2 text-red text-small shadow-flyout"
        >
          {error}
        </p>
      ) : (
        suggestions.length > 0 && (
          <div
            id={listId}
            role="listbox"
            aria-label={t.suggestions}
            // Picking with the mouse keeps the field's focus.
            onMouseDown={(event) => event.preventDefault()}
            className="absolute inset-x-0 top-full z-50 mt-1.5 max-h-menu overflow-y-auto rounded-flyout border border-flyout-border bg-flyout p-1 text-fg shadow-flyout"
          >
            {suggestions.map((suggestion, index) => (
              // biome-ignore lint/a11y/useKeyWithClickEvents: the field's arrows and Enter pick.
              <div
                key={suggestion}
                id={`${listId}-${index}`}
                role="option"
                tabIndex={-1}
                aria-selected={index === picked}
                onClick={() => void open(suggestion)}
                className={`flex h-8 items-center truncate rounded-ctrl px-2 text-small hover:bg-hover ${index === picked ? "bg-hover" : ""}`}
              >
                {suggestion}
              </div>
            ))}
          </div>
        )
      )}
    </div>
  );
}
