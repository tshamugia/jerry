/** Every user-visible string lives here. English only, no i18n library (PRD). */

import type { SortColumn } from "../ipc";

const integer = new Intl.NumberFormat("en-US");
const decimal = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 });

/** Column titles, as the Details view's header shows them. */
const sortColumns: Record<SortColumn, string> = {
  name: "Name",
  modified: "Date modified",
  type: "Type",
  size: "Size",
  created: "Date created",
  extension: "Extension",
};

export const strings = {
  appName: "Jerry",
  version: (version: string) => `Version ${version}`,
  /** The dev panel (dev builds only). */
  dev: {
    title: "Dev checks",
    sampleError: "Sample error",
    panicInJob: "Panic in a job",
    blockMainThread: "Freeze main thread",
    blockPage: "Freeze page",
    stream: "Binary stream",
    noError: "The command didn't fail.",
    frozeMainThread: (ms: number) => `Froze the main thread for ${ms} ms. The log has a warning.`,
    frozePage: (ms: number) => `Froze the page for ${ms} ms. The log has a warning.`,
    streamed: (messages: number, values: string) =>
      `Received ${messages} binary messages: ${values}.`,
    streamIncomplete: (messages: number) => `Only ${messages} of 3 messages arrived.`,
    toggleHidden: "Toggle hidden files",
    nextTheme: "Next theme",
    openWindow: "Open another window",
    settingsLoading: "Settings: loading…",
    settings: (showHidden: boolean, theme: string) =>
      `Settings: hidden files ${showHidden ? "shown" : "hidden"}, theme ${theme}.`,
    listingTitle: "Listing",
    listingPath: "Folder path",
    listFolder: "List",
    listSynth: (count: number) => `List ${integer.format(count)}`,
    buildingSynth: (count: number) => `Building a folder with ${integer.format(count)} items…`,
    listing: "Listing…",
    items: (count: number) => `${integer.format(count)} items`,
    listed: (timings: {
      count: number;
      batches: number;
      firstBatchMs: number;
      firstPaintMs: number;
      doneMs: number;
      donePaintMs: number;
      rustMs: number;
    }) =>
      `${integer.format(timings.count)} items in ${timings.batches} ${timings.batches === 1 ? "batch" : "batches"}. ` +
      `First rows painted ${decimal.format(timings.firstPaintMs)} ms, all ${decimal.format(timings.donePaintMs)} ms ` +
      `(first batch ${decimal.format(timings.firstBatchMs)} ms, done ${decimal.format(timings.doneMs)} ms, ` +
      `Rust ${decimal.format(timings.rustMs)} ms).`,
    sortBy: "Sort by",
    /** A sort button; `descending` is set on the column the list is sorted by. */
    sortColumn: (column: SortColumn, descending?: boolean) =>
      `${sortColumns[column]}${descending === undefined ? "" : descending ? " ↓" : " ↑"}`,
    sorted: (timings: {
      count: number;
      sort: { column: SortColumn; descending: boolean };
      orderMs: number;
      paintMs: number;
    }) =>
      `Sorted ${integer.format(timings.count)} items by ${sortColumns[timings.sort.column].toLowerCase()}, ` +
      `${timings.sort.descending ? "descending" : "ascending"}: painted ${decimal.format(timings.paintMs)} ms ` +
      `(order in JS ${decimal.format(timings.orderMs)} ms).`,
    nothingToSort: "List a folder first.",
    sortPending:
      "No new order yet: the folder is still being read, or a later sort overtook this one.",
    /** The last change the watcher sent, and how long it took to paint. */
    changed: (change: { added: number; changed: number; removed: number; paintMs: number }) =>
      `Live: ${integer.format(change.added)} new, ${integer.format(change.changed)} changed, ` +
      `${integer.format(change.removed)} removed; painted ${decimal.format(change.paintMs)} ms after it arrived.`,
  },
} as const;

export { errorText, fileName, sizeText } from "./errors";
