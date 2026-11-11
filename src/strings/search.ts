/** Deep search (Ctrl+Shift+F): the search window and results tabs. English only (PRD). */

const integer = new Intl.NumberFormat("en-US");
const decimal = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 });

/** "1.2 s" or "350 ms". */
const duration = (ms: number) =>
  ms >= 1000 ? `${decimal.format(ms / 1000)} s` : `${integer.format(Math.round(ms))} ms`;

export const searchStrings = {
  /** The search window. */
  title: "Search",
  close: "Esc",
  name: "Name",
  placeholder: "Names, *.dwg, several words…",
  /** Where to search: the folder the tab shows, with everything in it, or its whole drive. */
  scope: "Search in",
  thisFolder: (name: string) => `${name} and subfolders`,
  wholeDriveOf: (drive: string) => `Whole drive ${drive}`,
  filters: "Filters",
  type: "Type",
  size: "Size",
  modified: "Date modified",
  kind: "Items",
  anyType: "Any type",
  types: {
    documents: "Documents",
    drawings: "Drawings (DWG, DXF)",
    images: "Images",
    archives: "Archives",
    video: "Video",
  },
  extension: "Extension…",
  customSizeLabel: "Custom size",
  dateRangeLabel: "Date range",
  extensionField: "Extension",
  extensionPlaceholder: ".pdf",
  noExtension: "No extension",
  anySize: "Any size",
  sizes: {
    empty: "Empty (0 KB)",
    small: "Under 1 MB",
    medium: "1 – 100 MB",
    large: "100 MB – 1 GB",
    huge: "Over 1 GB",
  },
  customSize: "Custom size…",
  atLeast: "At least",
  atMost: "At most",
  sizePlaceholder: "10 MB",
  sizeBetween: (min: string, max: string) => `${min} – ${max}`,
  sizeAtLeast: (min: string) => `At least ${min}`,
  sizeAtMost: (max: string) => `At most ${max}`,
  badSize: "Sizes look like 500 KB, 10 MB or 2 GB.",
  anyTime: "Any time",
  dates: {
    today: "Today",
    thisWeek: "This week",
    thisMonth: "This month",
  },
  dateRange: "Date range…",
  from: "From",
  to: "To",
  datePlaceholder: "DD.MM.YYYY",
  dateBetween: (from: string, to: string) => `${from} – ${to}`,
  dateFrom: (from: string) => `From ${from}`,
  dateUntil: (to: string) => `Until ${to}`,
  badDate: "Dates look like 31.12.2026.",
  items: {
    all: "Files and folders",
    files: "Files only",
    folders: "Folders only",
  },
  wholeDrive: "Whole drive",
  recent: "Recent searches",
  /** A recent search without a name: its filters say what it was. */
  anyName: "Any name",
  removeRecent: "Remove from recent searches",
  search: "Search",
  hints: {
    search: "Search",
    recent: "Recent",
    close: "Close",
  },
  /** A results tab. */
  results: "Search results",
  /** The tab's tooltip: what it searched and where. */
  tooltip: (title: string, filters: string, path: string) =>
    filters === "" ? `${title} in ${path}` : `${title} (${filters}) in ${path}`,
  /** The results bar, where a folder's address would be. */
  bar: "Search",
  edit: "Edit search",
  within: (name: string) => `in ${name}`,
  searching: (found: number) => `Found ${integer.format(found)}, searching…`,
  done: (found: number, ms: number | undefined) =>
    ms === undefined
      ? `Found ${integer.format(found)}`
      : `Found ${integer.format(found)} in ${duration(ms)}`,
  stopped: (found: number) => `Stopped · found ${integer.format(found)}`,
  /** Folders the search couldn't read: no permission, or gone meanwhile. */
  skipped: (count: number) =>
    `${integer.format(count)} ${count === 1 ? "folder" : "folders"} couldn't be read`,
  stop: "Stop",
  /** The list, before anything is found and once nothing was. */
  nothingYet: "Searching…",
  nothingFound: "Nothing found",
  filterPlaceholder: "Filter results",
} as const;
