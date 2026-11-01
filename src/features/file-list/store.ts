import { create } from "zustand";
import { Channel, commands, type JsonEvent, type ListingStats, type SortBy } from "../../ipc";
import { Columns } from "./columns";
import { paintOf } from "./measure";
import { decodeMessage, type ListingMessage } from "./protocol";

export type WireFormat = "binary" | "json";

/** Timings of one `open`, in ms from just before `listing_open` is invoked. */
export interface OpenTimings {
  format: WireFormat;
  count: number;
  /** First batch decoded in JS. */
  firstBatchMs: number;
  /** First rows painted. */
  firstRowsMs: number;
  /** Done message in JS. */
  doneMs: number;
  /** The sorted list painted (the order arrives after the last batch). */
  sortedPaintMs: number;
  /** JS time spent decoding and appending batches. */
  decodeMs: number;
  rust: ListingStats | null;
}

/** Timings of one sort change, in ms. */
export interface SortTimings {
  column: SortBy;
  descending: boolean;
  /** Click to the new order painted. */
  totalMs: number;
  /** `listing_sort` round trip, Rust sort included. */
  ipcMs: number;
  /** Rust sort alone. */
  rustMs: number;
  decodeMs: number;
  /** New order handed to React until painted. */
  paintMs: number;
}

interface ListingState {
  listingId: number | null;
  columns: Columns;
  /** Entry ids in display order; null while loading means arrival order. */
  order: Uint32Array | null;
  count: number;
  /** Bumped on every change to `columns` or `order` (both are mutated in place). */
  version: number;
  loading: boolean;
  error: string | null;
  column: SortBy;
  descending: boolean;
  lastOpen: OpenTimings | null;
  lastSort: SortTimings | null;
  open: (count: number, format?: WireFormat) => Promise<OpenTimings>;
  sortBy: (column: SortBy, descending?: boolean) => Promise<SortTimings | null>;
}

let openToken = 0;

export const useListing = create<ListingState>()((set, get) => {
  const bump = (patch: Partial<ListingState>): number => {
    const version = get().version + 1;
    set({ ...patch, version });
    return version;
  };

  return {
    listingId: null,
    columns: new Columns(),
    order: null,
    count: 0,
    version: 0,
    loading: false,
    error: null,
    column: "name",
    descending: false,
    lastOpen: null,
    lastSort: null,

    open: (count, format = "binary") => {
      const previous = get().listingId;
      if (previous !== null) {
        void commands.listingClose(previous);
      }
      const token = ++openToken;
      const columns = new Columns();
      bump({
        listingId: null,
        columns,
        order: null,
        count: 0,
        loading: true,
        error: null,
        column: "name",
        descending: false,
      });

      return new Promise<OpenTimings>((resolve) => {
        const started = performance.now();
        let decodeMs = 0;
        let firstBatchMs = 0;
        let firstRows: Promise<number> | null = null;
        let sorted: Promise<number> | null = null;
        let invoked: Promise<unknown> = Promise.resolve();

        const finish = async (rust: ListingStats | null) => {
          const doneMs = performance.now() - started;
          await invoked;
          const [firstPaint, sortedPaint] = await Promise.all([firstRows, sorted]);
          const timings: OpenTimings = {
            format,
            count: columns.count,
            firstBatchMs,
            firstRowsMs: (firstPaint ?? started) - started,
            doneMs,
            sortedPaintMs: (sortedPaint ?? started) - started,
            decodeMs,
            rust,
          };
          set({ lastOpen: timings });
          resolve(timings);
        };

        const apply = (message: ListingMessage, receivedAt: number) => {
          if (token !== openToken) {
            return;
          }
          switch (message.kind) {
            case "batch": {
              const { firstId, names, sizes, modified, attributes } = message;
              columns.append(firstId, names, sizes, modified, attributes);
              const version = bump({ count: columns.count });
              if (!firstRows) {
                firstBatchMs = performance.now() - started;
                firstRows = paintOf(version);
              }
              break;
            }
            case "order":
              sorted = paintOf(bump({ order: message.ids }));
              break;
            case "done":
              set({ loading: false });
              void finish(message.stats);
              break;
            case "error":
              set({ loading: false, error: message.code });
              void finish(null);
              break;
          }
          decodeMs += performance.now() - receivedAt;
        };

        let request: ReturnType<typeof commands.listingOpen>;
        if (format === "binary") {
          const channel = new Channel<ArrayBuffer>();
          channel.onmessage = (buffer) => {
            const receivedAt = performance.now();
            apply(decodeMessage(buffer), receivedAt);
          };
          request = commands.listingOpen(count, channel);
        } else {
          const channel = new Channel<JsonEvent>();
          channel.onmessage = (event) => {
            const receivedAt = performance.now();
            apply(fromJson(event), receivedAt);
          };
          request = commands.listingOpenJson(count, channel);
        }
        invoked = request.then((result) => {
          if (token !== openToken) {
            return;
          }
          if (result.status === "ok") {
            set({ listingId: result.data });
          } else {
            set({ loading: false, error: result.error.code });
            void finish(null);
          }
        });
      });
    },

    sortBy: async (column, descending) => {
      const state = get();
      if (state.listingId === null) {
        return null;
      }
      const desc = descending ?? (state.column === column ? !state.descending : false);
      const started = performance.now();
      const result = await commands.listingSort(state.listingId, column, desc);
      const received = performance.now();
      if (result.status === "error") {
        set({ error: result.error.code });
        return null;
      }
      const message = decodeMessage(result.data);
      if (message.kind !== "order") {
        return null;
      }
      const decoded = performance.now();
      const painted = await paintOf(bump({ order: message.ids, column, descending: desc }));
      const timings: SortTimings = {
        column,
        descending: desc,
        totalMs: painted - started,
        ipcMs: received - started,
        rustMs: message.rustMs,
        decodeMs: decoded - received,
        paintMs: painted - decoded,
      };
      set({ lastSort: timings });
      return timings;
    },
  };
});

/** The JSON format's events as the binary decoder's messages. */
function fromJson(event: JsonEvent): ListingMessage {
  switch (event.kind) {
    case "batch":
      return {
        kind: "batch",
        firstId: event.firstId,
        count: event.names.length,
        names: event.names,
        sizes: Float64Array.from(event.sizes, (value) => value ?? 0),
        modified: Float64Array.from(event.modified, (value) => value ?? 0),
        attributes: Uint32Array.from(event.attributes),
      };
    case "order":
      return { kind: "order", ids: Uint32Array.from(event.ids), rustMs: event.rustMs ?? 0 };
    case "done": {
      const { kind: _, ...stats } = event;
      return { kind: "done", stats };
    }
    case "error":
      return { kind: "error", code: event.code, os: event.os ?? 0 };
  }
}
