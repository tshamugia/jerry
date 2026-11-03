import { type FormEvent, useEffect, useReducer, useRef, useState } from "react";
import { flushSync } from "react-dom";
import {
  commands,
  type ListingMessage,
  type ListingTarget,
  type OpenListing,
  openListing,
} from "../../ipc";
import { errorText, strings } from "../../strings";
import { DevButton } from "./DevButton";
import { DevIcons } from "./DevIcons";
import { type DevEntries, DevRows, emptyEntries } from "./DevRows";
import { afterNextPaint } from "./paint";
import { type ScrollResult, scrollTest } from "./scrollTest";

/** The dev panel's own tab: each listing it opens replaces the previous one. */
const DEV_TAB = 0;
const SYNTH_SIZES = [1_000, 10_000, 100_000] as const;
const FILE_ATTRIBUTE_DIRECTORY = 0x10;
/** Comfortable density, until settings (#41) choose it. */
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
  /** The whole folder in and painted. */
  donePaintMs: number;
  /** The listing in Rust alone. */
  rustMs: number;
}

export type ListingOutcome =
  | { status: "ok"; timings: ListingTimings }
  | { status: "error"; text: string };

declare global {
  interface Window {
    /** Dev builds: for scripts that time listings and scrolling over the DevTools protocol. */
    jerryDev?: {
      listing: (path: string) => Promise<ListingOutcome>;
      synthFolder: (count: number) => Promise<ListingOutcome | string>;
      scroll: (durationMs?: number) => Promise<ScrollResult | undefined>;
    };
  }
}

/**
 * Dev builds only: lists a folder through `listing_open`, shows every entry with its icon in a
 * virtualized list and times it (the PRD's "1 000 items shown < 0.2 s", "100 000 items: first
 * rows < 0.3 s" and "scrolling 10 000+ items at 60 fps"). Not the file list (#29).
 */
export function DevListing() {
  const [path, setPath] = useState("");
  const [, changed] = useReducer((version: number) => version + 1, 0);
  const [status, setStatus] = useState<string>();
  const viewport = useRef<HTMLDivElement>(null);
  const [rowHeight] = useState(rowPx);
  const [lister] = useState(() => createLister(changed));
  const entries = lister.entries();
  const count = entries.names.length;

  const report = (outcome: ListingOutcome) => {
    setStatus(outcome.status === "ok" ? strings.dev.listed(outcome.timings) : outcome.text);
    return outcome;
  };
  const listPath = (folder: string) => {
    setStatus(strings.dev.listing);
    return lister.list({ kind: "path", path: folder }).then(report);
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
  const scroll = async (durationMs?: number) => {
    if (!viewport.current) {
      return undefined;
    }
    setStatus(strings.dev.scrolling);
    const result = await scrollTest(viewport.current, rowHeight, durationMs);
    setStatus(strings.dev.scrolled(result));
    return result;
  };

  useEffect(() => () => lister.close(), [lister]);
  useEffect(() => {
    window.jerryDev = { listing: listPath, synthFolder: listSynth, scroll };
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
      <DevIcons />
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
        <DevButton onClick={() => void scroll()}>{strings.dev.scrollTest}</DevButton>
      </form>
      {status && <p className="text-small text-fg-2 tabular-nums">{status}</p>}
      <p className="text-small text-fg-3 tabular-nums">{strings.dev.items(count)}</p>
      <DevRows entries={entries} count={count} rowHeight={rowHeight} viewport={viewport} />
    </section>
  );
}

/** The row height of the current density, from the design tokens. */
function rowPx(): number {
  const token = getComputedStyle(document.documentElement).getPropertyValue("--jerry-row-h");
  return Number.parseFloat(token) || FALLBACK_ROW_PX;
}

/**
 * Opens listings one at a time and shows each change with `show`, each batch the moment it arrives
 * (`flushSync`), so the paint that follows is the one the timings report.
 */
function createLister(show: () => void) {
  let current: OpenListing | undefined;
  let entries: DevEntries = emptyEntries();
  let generation = 0;

  const close = () => {
    generation += 1;
    void current?.close();
    current = undefined;
  };

  const list = async (target: ListingTarget): Promise<ListingOutcome> => {
    close();
    const mine = generation;
    entries = emptyEntries();
    flushSync(show);
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
            const { names, sizes, modified, attributes, iconKeys } = message;
            for (let i = 0; i < names.length; i++) {
              entries.names.push(names[i] ?? "");
              entries.sizes.push(sizes[i] ?? 0);
              entries.modified.push(modified[i] ?? 0);
              entries.folders.push(((attributes[i] ?? 0) & FILE_ATTRIBUTE_DIRECTORY) !== 0);
              entries.iconKeys.push(iconKeys[i] ?? 0);
            }
            batches += 1;
            flushSync(show);
            if (!firstPaint) {
              firstBatchMs = at - started;
              firstPaint = afterNextPaint();
            }
            break;
          }
          case "done": {
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

      void openListing(DEV_TAB, target, onMessage).then((result) => {
        if (result.status === "error") {
          resolve({ status: "error", text: errorText(result.error) });
        } else if (mine === generation) {
          current = result.data;
          entries.listing = result.data.id;
          show();
        } else {
          void result.data.close();
        }
      });
    });
  };

  return { list, close, entries: () => entries };
}
