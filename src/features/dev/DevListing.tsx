import { type FormEvent, useEffect, useState } from "react";
import { flushSync } from "react-dom";
import {
  commands,
  type ListingDiff,
  type ListingMessage,
  type ListingOrder,
  type ListingTarget,
  type OpenListing,
  openListing,
  type Sort,
  type SortColumn,
} from "../../ipc";
import { errorText, sizeText, strings } from "../../strings";
import { DevButton } from "./DevButton";
import { afterNextPaint } from "./paint";

/** The dev panel's own tab: each listing it opens replaces the previous one. */
const DEV_TAB = 0;
/** Rows the view paints; the rest are only counted. */
const SHOWN_ROWS = 20;
const SYNTH_SIZES = [1_000, 100_000] as const;
const FILE_ATTRIBUTE_DIRECTORY = 0x10;
const SORT_COLUMNS: readonly SortColumn[] = [
  "name",
  "modified",
  "type",
  "size",
  "created",
  "extension",
];
const DEFAULT_SORT: Sort = { column: "name", descending: false, foldersFirst: true };

interface Row {
  id: number;
  name: string;
  size: number;
  folder: boolean;
}

/** One listing's timings, in ms from just before `listing_open` is invoked. */
export interface ListingTimings {
  count: number;
  batches: number;
  /** First batch decoded in JS. */
  firstBatchMs: number;
  /** First rows painted. */
  firstPaintMs: number;
  /** Done message in JS. */
  doneMs: number;
  /** The whole folder in, sorted and painted. */
  donePaintMs: number;
  /** The listing in Rust alone. */
  rustMs: number;
}

export type ListingOutcome =
  | { status: "ok"; timings: ListingTimings }
  | { status: "error"; text: string };

/** One re-sort's timings, in ms from just before `listing_sort` is invoked. */
export interface SortTimings {
  count: number;
  sort: Sort;
  /** The order decoded in JS. */
  orderMs: number;
  /** The first rows in the new order painted: what the PRD's "sort < 100 ms" measures. */
  paintMs: number;
}

export type SortOutcome =
  | { status: "ok"; timings: SortTimings }
  | { status: "error"; text: string };

declare global {
  interface Window {
    /** Dev builds: for scripts that time listings over the DevTools protocol. */
    jerryDev?: {
      listing: (path: string) => Promise<ListingOutcome>;
      synthFolder: (count: number) => Promise<ListingOutcome | string>;
      sort: (sort: Sort) => Promise<SortOutcome>;
    };
  }
}

/**
 * Dev builds only: lists a folder through `listing_open`, paints the first rows and times it
 * (the PRD's "1 000 items shown < 0.2 s" and "100 000 items: first rows < 0.3 s"), then re-sorts
 * it through `listing_sort` and times that ("100 000 items: sort < 100 ms"). The listing stays
 * live: changes made in Explorer arrive as diffs and repaint the rows. Not the file list (#29):
 * just enough to receive the batches, diffs and orders and paint them.
 */
export function DevListing() {
  const [path, setPath] = useState("");
  const [rows, setRows] = useState<Row[]>([]);
  const [count, setCount] = useState(0);
  const [sort, setSort] = useState(DEFAULT_SORT);
  const [status, setStatus] = useState<string>();
  const [change, setChange] = useState<string>();
  const [lister] = useState(() =>
    createLister(
      (shown, total) => {
        setRows(shown);
        setCount(total);
      },
      (live) => setChange(strings.dev.changed(live)),
    ),
  );

  const report = (outcome: ListingOutcome) => {
    setStatus(outcome.status === "ok" ? strings.dev.listed(outcome.timings) : outcome.text);
    return outcome;
  };
  const listPath = (folder: string) => {
    setStatus(strings.dev.listing);
    setChange(undefined);
    return lister.list({ kind: "path", path: folder }, sort).then(report);
  };
  const listSynth = async (size: number) => {
    setStatus(strings.dev.buildingSynth(size));
    const folder = await commands.devSynthFolder(size);
    if (folder.status === "error") {
      setStatus(errorText(folder.error));
      return errorText(folder.error);
    }
    setPath(folder.data);
    return listPath(folder.data);
  };
  const sortBy = async (next: Sort) => {
    setSort(next);
    const outcome = await lister.sort(next);
    setStatus(outcome.status === "ok" ? strings.dev.sorted(outcome.timings) : outcome.text);
    return outcome;
  };
  /** Like a column header: a new column sorts ascending, the same one flips direction. */
  const clickColumn = (column: SortColumn) =>
    void sortBy({
      ...sort,
      column,
      descending: column === sort.column && !sort.descending,
    });

  useEffect(() => () => lister.close(), [lister]);
  useEffect(() => {
    window.jerryDev = { listing: listPath, synthFolder: listSynth, sort: sortBy };
    return () => {
      window.jerryDev = undefined;
    };
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    void listPath(path);
  };

  return (
    <section aria-label={strings.dev.listingTitle} className="flex w-full max-w-xl flex-col gap-2">
      <form onSubmit={submit} className="flex gap-2">
        <input
          aria-label={strings.dev.listingPath}
          value={path}
          onChange={(event) => setPath(event.target.value)}
          className="h-row min-w-0 flex-1 rounded-ctrl border border-ctrl-border bg-ctrl px-2 text-fg"
        />
        <DevButton type="submit">{strings.dev.listFolder}</DevButton>
        {SYNTH_SIZES.map((size) => (
          <DevButton key={size} onClick={() => void listSynth(size)}>
            {strings.dev.listSynth(size)}
          </DevButton>
        ))}
      </form>
      <fieldset aria-label={strings.dev.sortBy} className="flex flex-wrap gap-2">
        {SORT_COLUMNS.map((column) => (
          <DevButton key={column} onClick={() => clickColumn(column)}>
            {strings.dev.sortColumn(column, column === sort.column ? sort.descending : undefined)}
          </DevButton>
        ))}
      </fieldset>
      {status && <p className="text-small text-fg-2 tabular-nums">{status}</p>}
      {change && <p className="text-small text-fg-2 tabular-nums">{change}</p>}
      <p className="text-small text-fg-3 tabular-nums">{strings.dev.items(count)}</p>
      <ul className="flex flex-col">
        {rows.map((row) => (
          <li key={row.id} className="flex h-row items-center justify-between gap-4">
            <span className="truncate">{row.name}</span>
            <span className="text-fg-2 tabular-nums">{row.folder ? "" : sizeText(row.size)}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** The last diff: what it changed, and how long from its arrival to the paint. */
export interface LiveChange {
  added: number;
  changed: number;
  removed: number;
  paintMs: number;
}

const sameSort = (a: Sort, b: Sort) =>
  a.column === b.column && a.descending === b.descending && a.foldersFirst === b.foldersFirst;

/**
 * Opens listings one at a time and shows the first rows of each batch, diff and order with
 * `show` the moment it arrives (`flushSync`), so the paint that follows is the one the timings
 * report. `changed` hears about every diff once it is painted.
 */
function createLister(
  show: (rows: Row[], count: number) => void,
  changed: (change: LiveChange) => void,
) {
  let current: OpenListing | undefined;
  let generation = 0;
  // Every entry the current listing has had by id, the removed ones, and the display order.
  let names: string[] = [];
  let sizes: number[] = [];
  let folders: boolean[] = [];
  let removed = new Set<number>();
  let order: Uint32Array | undefined;
  let done = false;
  let wanted = DEFAULT_SORT;
  /** A re-sort waiting for its order on the stream. */
  let sorting: { sort: Sort; started: number; resolve: (outcome: SortOutcome) => void } | undefined;

  const firstRows = (): Row[] => {
    const rows: Row[] = [];
    const count = order?.length ?? names.length;
    for (let i = 0; i < count && rows.length < SHOWN_ROWS; i++) {
      const id = order?.[i] ?? i;
      if (!removed.has(id)) {
        rows.push({
          id,
          name: names[id] ?? "",
          size: sizes[id] ?? 0,
          folder: folders[id] ?? false,
        });
      }
    }
    return rows;
  };
  const paint = () => flushSync(() => show(firstRows(), names.length - removed.size));
  /** Applies `next` if it is for the sort asked for last. */
  const apply = (next: ListingOrder) => {
    if (!sameSort(next.sort, wanted)) {
      return;
    }
    order = next.ids;
    paint();
    if (sorting && sameSort(sorting.sort, next.sort)) {
      const { sort, started, resolve } = sorting;
      sorting = undefined;
      const orderMs = performance.now() - started;
      void afterNextPaint().then((painted) =>
        resolve({
          status: "ok",
          timings: { count: next.ids.length, sort, orderMs, paintMs: painted - started },
        }),
      );
    }
  };
  /** Puts entries `ids` in (new ones continue the ids), drops `gone`, and paints. */
  const applyDiff = (diff: ListingDiff, at: number) => {
    let added = 0;
    diff.ids.forEach((id, i) => {
      if (id >= names.length) {
        added += 1;
      }
      names[id] = diff.names[i] ?? "";
      sizes[id] = diff.sizes[i] ?? 0;
      folders[id] = ((diff.attributes[i] ?? 0) & FILE_ATTRIBUTE_DIRECTORY) !== 0;
    });
    for (const id of diff.removed) {
      removed.add(id);
    }
    paint();
    const counts = { added, changed: diff.ids.length - added, removed: diff.removed.length };
    void afterNextPaint().then((painted) => changed({ ...counts, paintMs: painted - at }));
  };

  const close = () => {
    generation += 1;
    void current?.close();
    current = undefined;
    names = [];
    sizes = [];
    folders = [];
    removed = new Set();
    order = undefined;
    done = false;
    sorting?.resolve({ status: "error", text: strings.dev.sortPending });
    sorting = undefined;
  };

  const list = async (target: ListingTarget, sort: Sort): Promise<ListingOutcome> => {
    close();
    const mine = generation;
    wanted = sort;
    paint();
    await afterNextPaint();

    const started = performance.now();
    let batches = 0;
    let firstBatchMs = 0;
    let firstPaint: Promise<number> | undefined;

    return new Promise<ListingOutcome>((resolve) => {
      const onMessage = (message: ListingMessage) => {
        if (mine !== generation) {
          return;
        }
        const at = performance.now();
        switch (message.kind) {
          case "batch": {
            const { names: batchNames, sizes: batchSizes, attributes } = message;
            for (let i = 0; i < batchNames.length; i++) {
              names.push(batchNames[i] ?? "");
              sizes.push(batchSizes[i] ?? 0);
              folders.push(((attributes[i] ?? 0) & FILE_ATTRIBUTE_DIRECTORY) !== 0);
            }
            batches += 1;
            paint();
            if (!firstPaint) {
              firstBatchMs = at - started;
              firstPaint = afterNextPaint();
            }
            break;
          }
          case "order":
            apply(message);
            break;
          case "diff":
            applyDiff(message, at);
            break;
          case "done": {
            done = true;
            const doneMs = at - started;
            const donePaint = afterNextPaint();
            void Promise.all([firstPaint ?? donePaint, donePaint]).then(([first, all]) =>
              resolve({
                status: "ok",
                timings: {
                  count: message.count,
                  batches,
                  firstBatchMs,
                  firstPaintMs: first - started,
                  doneMs,
                  donePaintMs: all - started,
                  rustMs: message.rustMs,
                },
              }),
            );
            break;
          }
          case "error":
            resolve({ status: "error", text: errorText(message.error) });
            break;
        }
      };

      void openListing(DEV_TAB, target, sort, onMessage).then((result) => {
        if (result.status === "error") {
          resolve({ status: "error", text: errorText(result.error) });
        } else if (mine === generation) {
          current = result.data;
        } else {
          void result.data.close();
        }
      });
    });
  };

  /** Re-sorts and resolves once the new order, which comes on the stream, is painted. */
  const sort = async (next: Sort): Promise<SortOutcome> => {
    wanted = next;
    const listing = current;
    if (!listing) {
      return { status: "error", text: strings.dev.nothingToSort };
    }
    // An earlier sort still waiting is overtaken: its order won't be painted.
    sorting?.resolve({ status: "error", text: strings.dev.sortPending });
    sorting = undefined;
    const started = performance.now();
    const ordered = done
      ? new Promise<SortOutcome>((resolve) => {
          sorting = { sort: next, started, resolve };
        })
      : // Still being read: the stream's order before done follows the new sort.
        Promise.resolve<SortOutcome>({ status: "error", text: strings.dev.sortPending });
    const result = await listing.sort(next);
    if (result.status === "error") {
      sorting = undefined;
      return { status: "error", text: errorText(result.error) };
    }
    return ordered;
  };

  return { list, sort, close };
}
