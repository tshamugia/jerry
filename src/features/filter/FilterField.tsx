import { type KeyboardEvent, memo, useRef } from "react";
import { type Folder, useFolder } from "../../stores/folder";
import { useShortcut } from "../../stores/keymap";
import type { PaneSide } from "../../stores/window";
import { strings } from "../../strings";
import { IconButton } from "../../ui/IconButton";
import { CloseIcon, FilterIcon } from "../../ui/icons";
import { useShownRows } from "../file-list/rows";
import { focusList } from "./focus";

const t = strings.toolbar;

/**
 * The quick filter (Ctrl+F) in a pane's toolbar, 28 px with the shortcut as a hint: as you type,
 * the list shows only the entries whose names match (a part of the name, or wildcards: see
 * `wildcardMatcher`). While it is on, "Showing 12 of 340" says the list is narrowed, and a button
 * clears it. Esc clears it and gives the keyboard back to the list; Enter and ↓ go to the list
 * and keep it.
 */
export const FilterField = memo(function FilterField({
  folder,
  pane,
}: {
  folder: Folder;
  pane: PaneSide;
}) {
  const filter = useFolder(folder, (state) => state.filter);
  const results = useFolder(folder, (state) => state.search !== undefined);
  const shortcut = useShortcut("filter");
  const input = useRef<HTMLInputElement>(null);

  const keyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) {
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      folder.setFilter("");
      focusList(pane);
    } else if (event.key === "Enter" || event.key === "ArrowDown") {
      event.preventDefault();
      focusList(pane);
    }
  };
  const clear = () => {
    folder.setFilter("");
    input.current?.focus();
  };

  return (
    <div className="flex h-filter min-w-0 flex-1 items-center gap-2 rounded-ctrl border border-ctrl-border bg-ctrl px-2 text-fg-2 focus-within:outline-2 focus-within:outline-focus focus-within:-outline-offset-2">
      <label className="flex min-w-0 flex-1 items-center gap-2">
        <FilterIcon />
        <input
          ref={input}
          type="text"
          value={filter}
          onChange={(event) => folder.setFilter(event.target.value)}
          onKeyDown={keyDown}
          spellCheck={false}
          autoComplete="off"
          data-filter={pane}
          aria-label={t.filter}
          placeholder={results ? strings.search.filterPlaceholder : t.filterPlaceholder}
          className="min-w-0 flex-1 bg-transparent text-fg text-small outline-none placeholder:text-fg-3"
        />
      </label>
      {filter === "" ? (
        shortcut && (
          <kbd className="rounded-kbd border border-ctrl-border px-1.25 font-sans text-caption text-fg-3">
            {shortcut}
          </kbd>
        )
      ) : (
        <>
          <FilterCount folder={folder} />
          <IconButton label={t.clearFilter} onClick={clear} className="size-5 rounded-kbd">
            <CloseIcon />
          </IconButton>
        </>
      )}
    </div>
  );
});

/** "Showing 12 of 340". Apart, so the field doesn't follow every change in the folder. */
function FilterCount({ folder }: { folder: Folder }) {
  const { rows, total } = useShownRows(folder);
  return (
    <output className="whitespace-nowrap text-caption text-fg-3 tabular-nums">
      {t.showing(rows.length, total)}
    </output>
  );
}
