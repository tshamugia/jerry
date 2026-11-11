import type {
  ListingBatch,
  ListingDiff,
  ListingEntries,
  ResultFolders,
  SearchBatch,
  SearchDiff,
} from "../ipc";

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
  /**
   * Search results only (#66): each result's folder, by id in `folders` (display text). Empty for
   * a folder's entries.
   */
  folderIds = new Uint32Array(0);
  folders: string[] = [];

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

  /** Puts a search batch's results in, with their folders. */
  appendResults(batch: SearchBatch): void {
    this.appendBatch(batch);
    this.putFolders(batch, batch.firstId, batch.names.length);
  }

  /** Puts a search diff in: renamed results, results in renamed folders, removed ones. */
  applyResultsDiff(diff: SearchDiff): void {
    this.applyDiff(diff);
    this.reserveFolders();
    for (let i = 0; i < diff.ids.length; i++) {
      this.folderIds[diff.ids[i] ?? 0] = diff.folderIds[i] ?? 0;
    }
    this.addFolders(diff);
  }

  /** Where search result `id` is, as display text; undefined for a folder's entry. */
  location(id: number): string | undefined {
    return id < this.folderIds.length ? this.folders[this.folderIds[id] ?? 0] : undefined;
  }

  isFolder(id: number): boolean {
    return ((this.attributes[id] ?? 0) & Attribute.directory) !== 0;
  }

  /** Hidden or system: shown, dimmed, only with "Show hidden and system files" on. */
  isHidden(id: number): boolean {
    return ((this.attributes[id] ?? 0) & (Attribute.hidden | Attribute.system)) !== 0;
  }

  private putFolders(from: ResultFolders, firstId: number, count: number): void {
    this.reserveFolders();
    this.folderIds.set(from.folderIds.subarray(0, count), firstId);
    this.addFolders(from);
  }

  private addFolders({ firstFolder, folders }: ResultFolders): void {
    for (let i = 0; i < folders.length; i++) {
      this.folders[firstFolder + i] = folders[i] ?? "";
    }
  }

  /** The folder ids grow with the other columns, for search results only. */
  private reserveFolders(): void {
    if (this.folderIds.length < this.sizes.length) {
      this.folderIds = grown(this.folderIds, new Uint32Array(this.sizes.length));
    }
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

/**
 * Search results' `order` with the results found since it came after it, in arrival order: an
 * order covers what was found when it was made (#66).
 */
export function laterRows(entries: FolderEntries, order: Uint32Array): Uint32Array {
  let last = -1;
  for (const id of order) {
    if (id > last) last = id;
  }
  let later = 0;
  for (let id = last + 1; id < entries.count; id++) {
    if (!entries.removed.has(id)) later += 1;
  }
  if (later === 0) {
    return order;
  }
  const rows = new Uint32Array(order.length + later);
  rows.set(order);
  let at = order.length;
  for (let id = last + 1; id < entries.count; id++) {
    if (!entries.removed.has(id)) rows[at++] = id;
  }
  return rows;
}

/** The `rows` whose names `matches` (the quick filter), in the same order. */
export function matchingRows(
  entries: FolderEntries,
  rows: Uint32Array,
  matches: (name: string) => boolean,
): Uint32Array {
  const kept = new Uint32Array(rows.length);
  let at = 0;
  for (const id of rows) {
    if (matches(entries.names[id] ?? "")) kept[at++] = id;
  }
  return kept.subarray(0, at);
}

function arrivalOrder(entries: FolderEntries): Uint32Array {
  const ids = new Uint32Array(entries.count - entries.removed.size);
  let at = 0;
  for (let id = 0; id < entries.count; id++) {
    if (!entries.removed.has(id)) ids[at++] = id;
  }
  return ids;
}
