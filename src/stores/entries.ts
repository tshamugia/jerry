import type { ListingBatch, ListingDiff, ListingEntries } from "../ipc";

/** The `FILE_ATTRIBUTE_*` flags the file list reads. */
export const Attribute = { hidden: 0x2, system: 0x4, directory: 0x10 } as const;

/** Entries ids are given for, before the columns grow. */
const MIN_CAPACITY = 1024;

/**
 * A listing's entries by id, as columns that grow batch by batch: entry `id` is `names[id]`,
 * `sizes[id]` and so on. Typed arrays keep 100 000 entries at a few MB, with no object per entry.
 * A removed id stays a hole: the order leaves it out.
 */
export class FolderEntries {
  /** Ids given so far: every entry's id is below it. */
  count = 0;
  names: string[] = [];
  sizes = new Float64Array(0);
  modified = new Float64Array(0);
  created = new Float64Array(0);
  attributes = new Uint32Array(0);
  iconKeys = new Uint32Array(0);
  /** Ids diffs removed; they never come back. */
  readonly removed = new Set<number>();

  /** Puts a batch's entries in at `firstId` on. */
  appendBatch(batch: ListingBatch): void {
    const { firstId, names } = batch;
    this.reserve(firstId + names.length);
    for (let i = 0; i < names.length; i++) {
      this.names[firstId + i] = names[i] ?? "";
    }
    this.sizes.set(batch.sizes, firstId);
    this.modified.set(batch.modified, firstId);
    this.created.set(batch.created, firstId);
    this.attributes.set(batch.attributes, firstId);
    this.iconKeys.set(batch.iconKeys, firstId);
    this.count = Math.max(this.count, firstId + names.length);
  }

  /** Puts a diff's changed and new entries in and marks its removed ones. */
  applyDiff(diff: ListingDiff): void {
    const last = diff.ids.at(-1);
    if (last !== undefined) {
      this.reserve(last + 1);
    }
    for (let i = 0; i < diff.ids.length; i++) {
      this.put(diff.ids[i] ?? 0, diff, i);
    }
    for (const id of diff.removed) {
      this.removed.add(id);
    }
  }

  isFolder(id: number): boolean {
    return ((this.attributes[id] ?? 0) & Attribute.directory) !== 0;
  }

  /** Hidden or system: shown, dimmed, only with "Show hidden and system files" on. */
  isHidden(id: number): boolean {
    return ((this.attributes[id] ?? 0) & (Attribute.hidden | Attribute.system)) !== 0;
  }

  private put(id: number, from: ListingEntries, i: number): void {
    this.names[id] = from.names[i] ?? "";
    this.sizes[id] = from.sizes[i] ?? 0;
    this.modified[id] = from.modified[i] ?? 0;
    this.created[id] = from.created[i] ?? 0;
    this.attributes[id] = from.attributes[i] ?? 0;
    this.iconKeys[id] = from.iconKeys[i] ?? 0;
    this.count = Math.max(this.count, id + 1);
  }

  private reserve(needed: number): void {
    if (needed <= this.sizes.length) {
      return;
    }
    const capacity = Math.max(needed, this.sizes.length * 2, MIN_CAPACITY);
    this.sizes = grown(this.sizes, new Float64Array(capacity));
    this.modified = grown(this.modified, new Float64Array(capacity));
    this.created = grown(this.created, new Float64Array(capacity));
    this.attributes = grown(this.attributes, new Uint32Array(capacity));
    this.iconKeys = grown(this.iconKeys, new Uint32Array(capacity));
  }
}

function grown<T extends Float64Array | Uint32Array>(from: T, to: T): T {
  to.set(from);
  return to;
}

/**
 * The rows the list shows, as entry ids in display order: `order` (the core's), or every entry
 * in arrival order until the first order comes; hidden and system entries left out unless
 * `showHidden`. Returns `order` itself when nothing is left out.
 */
export function visibleRows(
  entries: FolderEntries,
  order: Uint32Array | undefined,
  showHidden: boolean,
): Uint32Array {
  const ids = order ?? arrivalOrder(entries);
  if (showHidden) {
    return ids;
  }
  let kept = 0;
  for (const id of ids) {
    if (!entries.isHidden(id)) kept += 1;
  }
  if (kept === ids.length) {
    return ids;
  }
  const rows = new Uint32Array(kept);
  let at = 0;
  for (const id of ids) {
    if (!entries.isHidden(id)) rows[at++] = id;
  }
  return rows;
}

function arrivalOrder(entries: FolderEntries): Uint32Array {
  const ids = new Uint32Array(entries.count - entries.removed.size);
  let at = 0;
  for (let id = 0; id < entries.count; id++) {
    if (!entries.removed.has(id)) ids[at++] = id;
  }
  return ids;
}
