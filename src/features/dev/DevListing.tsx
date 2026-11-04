import { type FormEvent, useEffect, useState } from "react";
import { flushSync } from "react-dom";
import { commands, type ListingTarget, type Sort } from "../../ipc";
import { type Folder, paneFolder, useFolder } from "../../stores/folder";
import { useWindow } from "../../stores/window";
import { errorText, strings } from "../../strings";
import { selectAllRows, selectionOf } from "../selection/commands";
import { DevButton } from "./DevButton";
import { DevIcons } from "./DevIcons";
import { afterNextPaint } from "./paint";
import { type ScrollResult, scrollTest } from "./scrollTest";

const SYNTH_SIZES = [1_000, 10_000, 100_000] as const;
/** Comfortable density, if the token can't be read. */
const FALLBACK_ROW_PX = 30;

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

/** One re-sort's timings, in ms from just before the sort is asked for. */
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

/** Select all's timings, in ms from just before Ctrl+A's command runs. */
export interface SelectTimings {
  count: number;
  /** The new selection in the store. */
  selectMs: number;
  /** The selected rows painted: what "Ctrl+A on 100 000 entries is instant" measures. */
  paintMs: number;
}

/** The last diff: what it changed, and how long from its arrival to the paint. */
export interface LiveChange {
  added: number;
  changed: number;
  removed: number;
  paintMs: number;
}

export interface PaintedChange extends LiveChange {
  paintedAt: number;
}

declare global {
  interface Window {
    /** Dev builds: for scripts that time listings, sorts and scrolling over the DevTools protocol. */
    jerryDev?: {
      listing: (path: string) => Promise<ListingOutcome>;
      synthFolder: (count: number) => Promise<ListingOutcome | string>;
      sort: (sort: Sort) => Promise<SortOutcome>;
      scroll: (durationMs?: number) => Promise<ScrollResult | undefined>;
      /** Ctrl+A's command on the active pane, from nothing selected, timed to the paint. */
      selectAll: () => Promise<SelectTimings>;
      /** The next diff, once painted; `paintedAt` is `Date.now()` then, to time from outside. */
      nextChange: () => Promise<PaintedChange>;
    };
  }
}

/**
 * Dev builds only: opens a folder (or a synthetic one) in the active pane's file list and times
 * it (the PRD's "1 000 items shown < 0.2 s" and "100 000 items: first rows < 0.3 s"), times
 * re-sorts ("sort < 100 ms") and scrolls the list from top to bottom ("60 fps"). The list is the
 * real Details view, so the numbers are its own. Changes made in Explorer show as a live line.
 */
export function DevListing() {
  const [path, setPath] = useState("");
  const [status, setStatus] = useState<string>();
  const [change, setChange] = useState<string>();
  const pane = useWindow((state) => state.activePane);
  const folder = paneFolder(pane);
  const count = useFolder(folder, (state) => state.entries.count - state.entries.removed.size);

  const listPath = async (target: string) => {
    setStatus(strings.dev.listing);
    setChange(undefined);
    const outcome = await timeListing(folder, { kind: "path", path: target });
    setStatus(outcome.status === "ok" ? strings.dev.listed(outcome.timings) : outcome.text);
    return outcome;
  };
  const listSynth = async (size: number) => {
    setStatus(strings.dev.buildingSynth(size));
    const synth = await commands.devSynthFolder(size);
    if (synth.status === "error") {
      setStatus(errorText(synth.error));
      return errorText(synth.error);
    }
    setPath(synth.data);
    return listPath(synth.data);
  };
  const sort = async (next: Sort) => {
    const outcome = await timeSort(folder, next);
    setStatus(outcome.status === "ok" ? strings.dev.sorted(outcome.timings) : outcome.text);
    return outcome;
  };
  const scroll = async (durationMs?: number) => {
    const viewport = document.querySelector<HTMLElement>(`[data-file-list="${pane}"]`);
    if (!viewport) {
      return undefined;
    }
    setStatus(strings.dev.scrolling);
    const result = await scrollTest(viewport, rowPx(), durationMs);
    setStatus(strings.dev.scrolled(result));
    return result;
  };

  const selectAll = async () => {
    const timings = await timeSelectAll(folder);
    setStatus(strings.dev.selectedAll(timings));
    return timings;
  };

  useEffect(
    () =>
      watchChanges(folder, (live) => {
        setChange(strings.dev.changed(live));
      }),
    [folder],
  );
  useEffect(() => {
    window.jerryDev = {
      listing: listPath,
      synthFolder: listSynth,
      sort,
      scroll,
      selectAll,
      nextChange: () =>
        new Promise((resolve) => {
          const stop = watchChanges(folder, (live) => {
            stop();
            resolve({ ...live, paintedAt: Date.now() });
          });
        }),
    };
    return () => {
      window.jerryDev = undefined;
    };
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    void listPath(path);
  };

  return (
    <section aria-label={strings.dev.listingTitle} className="flex w-full flex-col gap-2">
      <form onSubmit={submit} className="flex flex-wrap gap-2">
        <input
          aria-label={strings.dev.listingPath}
          value={path}
          onChange={(event) => setPath(event.target.value)}
          className="h-row w-full rounded-ctrl border border-ctrl-border bg-ctrl px-2 text-fg"
        />
        <DevButton type="submit">{strings.dev.listFolder}</DevButton>
        {SYNTH_SIZES.map((size) => (
          <DevButton key={size} onClick={() => void listSynth(size)}>
            {strings.dev.listSynth(size)}
          </DevButton>
        ))}
        <DevButton onClick={() => void scroll()}>{strings.dev.scrollTest}</DevButton>
        <DevButton onClick={() => void selectAll()}>{strings.dev.timeSelectAll}</DevButton>
      </form>
      {status && <p className="text-small text-fg-2 tabular-nums">{status}</p>}
      {change && <p className="text-small text-fg-2 tabular-nums">{change}</p>}
      <p className="text-small text-fg-3 tabular-nums">{strings.dev.items(count)}</p>
      <DevIcons />
    </section>
  );
}

/** The row height of the current density, from the design tokens. */
function rowPx(): number {
  const token = getComputedStyle(document.documentElement).getPropertyValue("--jerry-row-h");
  return Number.parseFloat(token) || FALLBACK_ROW_PX;
}

const sameSort = (a: Sort, b: Sort) =>
  a.column === b.column && a.descending === b.descending && a.foldersFirst === b.foldersFirst;

/** Opens `target` in `folder` and resolves once the whole folder is in and painted. */
async function timeListing(folder: Folder, target: ListingTarget): Promise<ListingOutcome> {
  // The previous folder's rows go first, so their removal isn't timed.
  folder.close();
  await afterNextPaint();
  return new Promise<ListingOutcome>((resolve) => {
    const started = performance.now();
    let settled = false;
    let batches = 0;
    let firstBatchMs = 0;
    let firstPaint: Promise<number> | undefined;
    const settle = (outcome: Promise<ListingOutcome>) => {
      settled = true;
      stop();
      void outcome.then(resolve);
    };
    const stop = folder.listen((message) => {
      const at = performance.now();
      switch (message.kind) {
        case "batch":
          batches += 1;
          if (!firstPaint) {
            firstBatchMs = at - started;
            firstPaint = afterNextPaint();
          }
          break;
        case "done": {
          const donePaint = afterNextPaint();
          settle(
            Promise.all([firstPaint ?? donePaint, donePaint]).then(([first, all]) => ({
              status: "ok",
              timings: {
                count: message.count,
                batches,
                firstBatchMs,
                firstPaintMs: first - started,
                doneMs: at - started,
                donePaintMs: all - started,
                rustMs: message.rustMs,
              },
            })),
          );
          break;
        }
        case "error":
          settle(Promise.resolve({ status: "error", text: errorText(message.error) }));
          break;
      }
    });
    void folder.open(target).then(() => {
      // `listing_open` itself failed: no stream follows.
      const { status, error } = folder.store.getState();
      if (!settled && status === "failed" && error) {
        settle(Promise.resolve({ status: "error", text: errorText(error) }));
      }
    });
  });
}

/** Sorts `folder` by `sort` and resolves once the new order is painted. */
function timeSort(folder: Folder, sort: Sort): Promise<SortOutcome> {
  if (folder.store.getState().status !== "ready") {
    return Promise.resolve({ status: "error", text: strings.dev.nothingToSort });
  }
  return new Promise<SortOutcome>((resolve) => {
    const started = performance.now();
    const stop = folder.listen((message) => {
      if (message.kind === "order" && sameSort(message.sort, sort)) {
        stop();
        const orderMs = performance.now() - started;
        void afterNextPaint().then((painted) =>
          resolve({
            status: "ok",
            timings: { count: message.ids.length, sort, orderMs, paintMs: painted - started },
          }),
        );
      }
    });
    folder.setSort(sort);
  });
}

/** Clears `folder`'s selection, then runs Ctrl+A's command and resolves once it is painted. */
async function timeSelectAll(folder: Folder): Promise<SelectTimings> {
  flushSync(() =>
    folder.setSelection({ ...selectionOf(folder.store.getState()), selected: new Set() }),
  );
  await afterNextPaint();
  const started = performance.now();
  flushSync(selectAllRows);
  const selectMs = performance.now() - started;
  const painted = await afterNextPaint();
  const count = folder.store.getState().selected.size;
  return { count, selectMs, paintMs: painted - started };
}

/** Calls `changed` with every diff in `folder` once it is painted. Returns a function that stops. */
function watchChanges(folder: Folder, changed: (change: LiveChange) => void): () => void {
  // Ids below it existed before the diff: the others are new.
  let known = folder.store.getState().entries.count;
  return folder.listen((message) => {
    const at = performance.now();
    if (message.kind === "batch") {
      known = message.firstId === 0 ? message.names.length : known + message.names.length;
    } else if (message.kind === "diff") {
      const added = message.ids.filter((id) => id >= known).length;
      known += added;
      const counts = {
        added,
        changed: message.ids.length - added,
        removed: message.removed.length,
      };
      void afterNextPaint().then((painted) => changed({ ...counts, paintMs: painted - at }));
    }
  });
}
