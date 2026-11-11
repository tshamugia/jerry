import { memo } from "react";
import { type Folder, type SearchState, useFolder } from "../../stores/folder";
import { strings } from "../../strings";
import { SearchIcon } from "../../ui/icons";
import { dateField, filtersText, searchTitle } from "./query";
import { openSearch } from "./search";

const t = strings.search;

/**
 * A results tab's bar, where a folder's address would be (#66): what the tab searched for and
 * where, then "Found 48, searching…" with Stop while it runs, and the count and time once it is
 * done. A click on the search opens the search window to change it; Ctrl+L still types an address.
 */
export const SearchBar = memo(function SearchBar({ folder }: { folder: Folder }) {
  const search = useFolder(folder, (state) => state.search);
  if (!search) {
    return null;
  }
  const { spec } = search;
  const filters = filtersText(spec.search, dateField);
  const where = t.within(spec.search.wholeDrive ? spec.drive : spec.name);
  return (
    // The search's words keep a little room, then the count gives way; Stop always shows.
    <div className="@container flex h-full min-w-0 flex-1 items-center gap-1.5 overflow-hidden text-small">
      <button
        type="button"
        title={t.edit}
        onClick={openSearch}
        className="flex h-full min-w-16 flex-1 items-center gap-2 overflow-hidden rounded-kbd px-1 text-left hover:bg-hover"
      >
        <span className="shrink-0 text-fg-2">
          <SearchIcon />
        </span>
        <span className="min-w-0 truncate font-semibold text-fg">{searchTitle(spec.search)}</span>
        {/* Only where it has room: a sliver of it would read as a stray letter. */}
        <span className="hidden min-w-0 flex-1 basis-0 truncate text-fg-3 @3xs:block">
          {filters === "" ? where : `${filters} · ${where}`}
        </span>
      </button>
      <Status search={search} />
      {search.phase === "searching" && (
        <button
          type="button"
          onClick={() => folder.stopSearch()}
          className="h-6 shrink-0 rounded-kbd border border-ctrl-border px-2 text-fg hover:bg-hover"
        >
          {t.stop}
        </button>
      )}
    </div>
  );
});

/**
 * "Found 48, searching…", "Found 48 in 1.2 s", "Stopped · found 48"; folders that couldn't be
 * read in its tip, so the bar keeps room for the search in a narrow pane.
 */
function Status({ search }: { search: SearchState }) {
  const { phase, found, skipped, ms } = search;
  const text =
    phase === "searching"
      ? t.searching(found)
      : phase === "stopped"
        ? t.stopped(found)
        : t.done(found, ms);
  // The whole of it, which a narrow bar cuts short.
  const tip = skipped > 0 ? `${text} · ${t.skipped(skipped)}` : text;
  return (
    <output
      title={tip}
      className="min-w-0 truncate whitespace-nowrap text-caption text-fg-2 tabular-nums"
    >
      {text}
    </output>
  );
}
