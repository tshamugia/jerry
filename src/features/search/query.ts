import type {
  DateFilter,
  DateRange,
  FileType,
  SavedSearch,
  SearchItems,
  SearchQuery,
  SizeRange,
} from "../../ipc";
import { sizeText, strings } from "../../strings";

/**
 * The search window's form (PRD «ღრმა ძებნა», #66) and what it means: a `SavedSearch` (what the
 * history keeps and a results tab runs again) becomes the engine's `SearchQuery` here, with its
 * relative dates ("this week") worked out in the user's time zone at the moment it runs.
 */

const t = strings.search;

/** A search with nothing set: every item under the folder. */
export const EMPTY_SEARCH: SavedSearch = {
  name: "",
  fileType: null,
  size: null,
  modified: null,
  items: "all",
  wholeDrive: false,
};

/** Bytes in a KB, MB, GB and TB, as Explorer and the Size column count them. */
const KB = 1024;
const MB = KB * 1024;
const GB = MB * 1024;
const TB = GB * 1024;

/** The ready size ranges (PRD: "მზა დიაპაზონები"), as the Size menu offers them. */
export const SIZE_PRESETS: readonly { label: string; size: SizeRange }[] = [
  { label: t.sizes.empty, size: { min: 0, max: 0 } },
  { label: t.sizes.small, size: { min: null, max: MB - 1 } },
  { label: t.sizes.medium, size: { min: MB, max: 100 * MB - 1 } },
  { label: t.sizes.large, size: { min: 100 * MB, max: GB - 1 } },
  { label: t.sizes.huge, size: { min: GB, max: null } },
];

/** The type groups, as the Type menu offers them. */
export const TYPE_GROUPS: readonly Exclude<FileType["kind"], "extension">[] = [
  "documents",
  "drawings",
  "images",
  "archives",
  "video",
];

/** The relative dates, as the Date modified menu offers them. */
export const DATE_PRESETS: readonly Exclude<DateFilter["kind"], "range">[] = [
  "today",
  "thisWeek",
  "thisMonth",
];

export const ITEMS: readonly SearchItems[] = ["all", "files", "folders"];

/**
 * The engine's query for `search` at `now`: dates in the user's time zone, hidden items with
 * `showHidden` (Ctrl+H), Windows' own folders and links left out as the PRD says.
 */
export function queryOf(search: SavedSearch, showHidden: boolean, now = new Date()): SearchQuery {
  return {
    name: search.name.trim(),
    fileType: search.fileType,
    size: search.size,
    modified: dateRange(search.modified, now),
    items: search.items,
    hidden: showHidden,
    followLinks: false,
    systemFolders: false,
  };
}

/**
 * The days `filter` covers at `now`, local time: today from midnight, this week from Monday,
 * this month from the 1st, each to the same point a day, week or month on.
 */
export function dateRange(filter: DateFilter | null, now: Date): DateRange | null {
  if (filter === null) {
    return null;
  }
  const day = (year: number, month: number, date: number) => new Date(year, month, date).getTime();
  const [year, month, date] = [now.getFullYear(), now.getMonth(), now.getDate()];
  switch (filter.kind) {
    case "today":
      return { from: day(year, month, date), to: day(year, month, date + 1) };
    case "thisWeek": {
      // Monday is 0: getDay() counts from Sunday.
      const monday = date - ((now.getDay() + 6) % 7);
      return { from: day(year, month, monday), to: day(year, month, monday + 7) };
    }
    case "thisMonth":
      return { from: day(year, month, 1), to: day(year, month + 1, 1) };
    case "range":
      return { from: filter.from, to: filter.to };
  }
}

/** `DD.MM.YYYY` (dots, slashes or dashes; the year may be 2 digits) as local midnight, in ms. */
export function parseDate(text: string): number | undefined {
  const match = /^\s*(\d{1,2})[./-](\d{1,2})[./-](\d{2}|\d{4})\s*$/.exec(text);
  if (!match) {
    return undefined;
  }
  const [date, month, rawYear] = [Number(match[1]), Number(match[2]) - 1, Number(match[3])];
  const year = rawYear < 100 ? 2000 + rawYear : rawYear;
  const parsed = new Date(year, month, date);
  const real =
    parsed.getFullYear() === year && parsed.getMonth() === month && parsed.getDate() === date;
  return real ? parsed.getTime() : undefined;
}

/** A day's ms as `DD.MM.YYYY`, for a date field. */
export function dateField(ms: number): string {
  const date = new Date(ms);
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${pad(date.getDate())}.${pad(date.getMonth() + 1)}.${date.getFullYear()}`;
}

/**
 * The range a user typed as two dates, `from` and `to` both included: `to` becomes the next
 * midnight, as the engine leaves `to` out. Undefined when a date that is there doesn't read.
 */
export function rangeOf(from: string, to: string): DateFilter | undefined {
  const start = from.trim() === "" ? null : parseDate(from);
  const end = to.trim() === "" ? null : parseDate(to);
  if (start === undefined || end === undefined) {
    return undefined;
  }
  const next = end === null ? null : nextDay(end);
  return { kind: "range", from: start, to: next };
}

/** The last day a range's `to` (next midnight, left out) includes, as ms of its midnight. */
export function lastDay(to: number): number {
  const date = new Date(to);
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() - 1).getTime();
}

function nextDay(ms: number): number {
  const date = new Date(ms);
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() + 1).getTime();
}

/** "500 KB", "1.5 GB", "12" (MB) as bytes; undefined when it doesn't read. */
export function parseSize(text: string): number | undefined {
  const match = /^\s*(\d+(?:[.,]\d+)?)\s*(b|kb|mb|gb|tb)?\s*$/i.exec(text);
  if (!match) {
    return undefined;
  }
  const value = Number((match[1] ?? "").replace(",", "."));
  const unit = (match[2] ?? "mb").toLowerCase();
  const bytes = { b: 1, kb: KB, mb: MB, gb: GB, tb: TB }[unit] ?? MB;
  return Math.round(value * bytes);
}

/** Bytes as a size field shows them: the largest whole unit, "500 KB", "1.5 GB". */
export function sizeField(bytes: number): string {
  return sizeText(bytes);
}

/** A size range the user typed: at least `min`, at most `max`. Undefined if one doesn't read. */
export function sizeRangeOf(min: string, max: string): SizeRange | undefined {
  const low = min.trim() === "" ? null : parseSize(min);
  const high = max.trim() === "" ? null : parseSize(max);
  if (low === undefined || high === undefined) {
    return undefined;
  }
  return { min: low, max: high };
}

/** The preset `size` is, if it is one. */
export function sizePreset(size: SizeRange | null): string | undefined {
  if (size === null) {
    return undefined;
  }
  return SIZE_PRESETS.find((preset) => preset.size.min === size.min && preset.size.max === size.max)
    ?.label;
}

/** What the Type filter shows: a group's name, or the extension (".dwg"). */
export function typeLabel(type: FileType | null): string {
  if (type === null) {
    return t.anyType;
  }
  if (type.kind === "extension") {
    const extension = type.extension.trim().replace(/^\*?\.?/, "");
    return extension === "" ? t.noExtension : `.${extension}`;
  }
  return t.types[type.kind];
}

/** What the Size filter shows. */
export function sizeLabel(size: SizeRange | null): string {
  if (size === null) {
    return t.anySize;
  }
  const preset = sizePreset(size);
  if (preset !== undefined) {
    return preset;
  }
  if (size.min !== null && size.max !== null) {
    return t.sizeBetween(sizeText(size.min), sizeText(size.max));
  }
  if (size.min !== null) {
    return t.sizeAtLeast(sizeText(size.min));
  }
  return size.max === null ? t.anySize : t.sizeAtMost(sizeText(size.max));
}

/** What the Date modified filter shows. */
export function dateLabel(filter: DateFilter | null, format: (ms: number) => string): string {
  if (filter === null) {
    return t.anyTime;
  }
  if (filter.kind !== "range") {
    return t.dates[filter.kind];
  }
  const { from, to } = filter;
  if (from !== null && to !== null) {
    return t.dateBetween(format(from), format(lastDay(to)));
  }
  if (from !== null) {
    return t.dateFrom(format(from));
  }
  return to === null ? t.anyTime : t.dateUntil(format(lastDay(to)));
}

/**
 * The filters of `search` in a few words, for the history and a results tab's tooltip:
 * "Drawings · This week · Files only · Whole drive". Empty with none set.
 */
export function filtersText(search: SavedSearch, format: (ms: number) => string): string {
  const parts: string[] = [];
  if (search.fileType !== null) parts.push(typeLabel(search.fileType));
  if (search.size !== null) parts.push(sizeLabel(search.size));
  if (search.modified !== null) parts.push(dateLabel(search.modified, format));
  if (search.items !== "all") parts.push(t.items[search.items]);
  if (search.wholeDrive) parts.push(t.wholeDrive);
  return parts.join(" · ");
}

/** A results tab's title: the name text, or "Search results" without one. */
export function searchTitle(search: SavedSearch): string {
  const name = search.name.trim();
  return name === "" ? t.results : name;
}

/** A search's identity, filters and all: equal searches have equal keys. */
export function searchKey(search: SavedSearch): string {
  const { fileType, size, modified } = search;
  return JSON.stringify([
    search.name,
    fileType?.kind ?? null,
    fileType?.kind === "extension" ? fileType.extension : null,
    size?.min ?? null,
    size?.max ?? null,
    modified?.kind ?? null,
    modified?.kind === "range" ? [modified.from, modified.to] : null,
    search.items,
    search.wholeDrive,
  ]);
}
