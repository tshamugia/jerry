import { create } from "zustand";
import { Channel, commands, type SortBy } from "../../ipc";
import { stamp } from "../startup/stamps";
import { Columns } from "./columns";
import { decodeMessage, type ListingMessage } from "./protocol";

/** One open listing's data (spike #11's memory test keeps several, like tabs). */
export interface Listing {
  id: number | null;
  columns: Columns;
  /** Entry ids in display order; null while loading means arrival order. */
  order: Uint32Array | null;
  count: number;
  loading: boolean;
  error: string | null;
}

interface ListingState extends Listing {
  /** Bumped on every change to `columns` or `order` (both are mutated in place). */
  version: number;
  column: SortBy;
  descending: boolean;
  /** Listings of the other tabs (memory test only; never rendered). */
  background: Listing[];
  openStartup: () => void;
  openBackground: (count: number) => Promise<void>;
  sortBy: (column: SortBy, descending?: boolean) => Promise<void>;
}

/** Feeds one listing's messages into `target`; `changed` runs after each one. */
function receive(target: Listing, onDone: () => void, changed: () => void): Channel<ArrayBuffer> {
  const channel = new Channel<ArrayBuffer>();
  channel.onmessage = (buffer) => {
    const message: ListingMessage = decodeMessage(buffer);
    switch (message.kind) {
      case "batch": {
        const { firstId, names, sizes, modified, attributes } = message;
        target.columns.append(firstId, names, sizes, modified, attributes);
        target.count = target.columns.count;
        break;
      }
      case "order":
        target.order = message.ids;
        break;
      case "done":
        target.loading = false;
        onDone();
        break;
      case "error":
        target.loading = false;
        target.error = message.code;
        onDone();
        break;
    }
    changed();
  };
  return channel;
}

const emptyListing = (): Listing => ({
  id: null,
  columns: new Columns(),
  order: null,
  count: 0,
  loading: true,
  error: null,
});

export const useListing = create<ListingState>()((set, get) => ({
  ...emptyListing(),
  loading: false,
  version: 0,
  column: "name",
  descending: false,
  background: [],

  openStartup: () => {
    const target = emptyListing();
    set({ ...target, version: get().version + 1 });
    let first = true;
    const channel = receive(
      target,
      () => stamp("listing:done"),
      () => {
        if (first) {
          stamp("listing:first_message");
          first = false;
        }
        set({ ...target, version: get().version + 1 });
      },
    );
    stamp("listing:invoke");
    void commands.listingStartup(channel).then((result) => {
      if (result.status === "ok") {
        target.id = result.data;
        set({ id: result.data });
      } else {
        set({ loading: false, error: result.error.code });
      }
    });
  },

  openBackground: (count) =>
    new Promise<void>((resolve) => {
      const target = emptyListing();
      const channel = receive(target, resolve, () => {});
      void commands.listingOpen(count, channel).then((result) => {
        if (result.status === "ok") {
          target.id = result.data;
        } else {
          resolve();
        }
      });
      set({ background: [...get().background, target] });
    }),

  sortBy: async (column, descending) => {
    const state = get();
    if (state.id === null) {
      return;
    }
    const desc = descending ?? (state.column === column ? !state.descending : false);
    const result = await commands.listingSort(state.id, column, desc);
    if (result.status === "error") {
      set({ error: result.error.code });
      return;
    }
    const message = decodeMessage(result.data);
    if (message.kind === "order") {
      set({ order: message.ids, column, descending: desc, version: get().version + 1 });
    }
  },
}));
