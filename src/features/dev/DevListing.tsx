import { type FormEvent, useEffect, useState } from "react";
import { flushSync } from "react-dom";
import {
  commands,
  type ListingMessage,
  type ListingTarget,
  type OpenListing,
  openListing,
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
    /** Dev builds: for scripts that time listings over the DevTools protocol. */
    jerryDev?: {
      listing: (path: string) => Promise<ListingOutcome>;
      synthFolder: (count: number) => Promise<ListingOutcome | string>;
    };
  }
}

/**
 * Dev builds only: lists a folder through `listing_open`, paints the first rows and times it
 * (the PRD's "1 000 items shown < 0.2 s" and "100 000 items: first rows < 0.3 s"). Not the file
 * list (#29): just enough to receive and paint the batches.
 */
export function DevListing() {
  const [path, setPath] = useState("");
  const [rows, setRows] = useState<Row[]>([]);
  const [count, setCount] = useState(0);
  const [status, setStatus] = useState<string>();
  const [lister] = useState(() =>
    createLister((shown, total) => {
      setRows(shown);
      setCount(total);
    }),
  );

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

  useEffect(() => () => lister.close(), [lister]);
  useEffect(() => {
    window.jerryDev = { listing: listPath, synthFolder: listSynth };
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
      {status && <p className="text-small text-fg-2 tabular-nums">{status}</p>}
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

/**
 * Opens listings one at a time and shows each batch with `show` the moment it arrives
 * (`flushSync`), so the paint that follows is the one the timings report.
 */
function createLister(show: (rows: Row[], count: number) => void) {
  let current: OpenListing | undefined;
  let generation = 0;

  const close = () => {
    generation += 1;
    void current?.close();
    current = undefined;
  };

  const list = async (target: ListingTarget): Promise<ListingOutcome> => {
    close();
    const mine = generation;
    flushSync(() => show([], 0));
    await afterNextPaint();

    const started = performance.now();
    const rows: Row[] = [];
    let count = 0;
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
            const { firstId, names, sizes, attributes } = message;
            for (let i = 0; i < names.length && rows.length < SHOWN_ROWS; i++) {
              rows.push({
                id: firstId + i,
                name: names[i] ?? "",
                size: sizes[i] ?? 0,
                folder: ((attributes[i] ?? 0) & FILE_ATTRIBUTE_DIRECTORY) !== 0,
              });
            }
            count += names.length;
            batches += 1;
            flushSync(() => show([...rows], count));
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
        } else {
          void result.data.close();
        }
      });
    });
  };

  return { list, close };
}
